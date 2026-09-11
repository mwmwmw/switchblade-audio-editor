use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;

use super::AudioClip;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Wav,
    Aiff,
    Flac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    Int24,
    Float32,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Wav, ExportFormat::Aiff, ExportFormat::Flac];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Wav => "WAV",
            ExportFormat::Aiff => "AIFF",
            ExportFormat::Flac => "FLAC",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Aiff => "aiff",
            ExportFormat::Flac => "flac",
        }
    }

    pub fn from_path(path: &Path) -> Option<ExportFormat> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "wav" | "wave" => Some(ExportFormat::Wav),
            "aif" | "aiff" | "aifc" => Some(ExportFormat::Aiff),
            "flac" => Some(ExportFormat::Flac),
            _ => None,
        }
    }

    pub fn supports(self, depth: BitDepth) -> bool {
        !(self == ExportFormat::Flac && depth == BitDepth::Float32)
    }
}

impl BitDepth {
    pub const ALL: [BitDepth; 3] = [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32];

    pub fn label(self) -> &'static str {
        match self {
            BitDepth::Int16 => "16-bit PCM",
            BitDepth::Int24 => "24-bit PCM",
            BitDepth::Float32 => "32-bit float",
        }
    }

    pub fn bits(self) -> u16 {
        match self {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        }
    }
}

pub fn save(path: &Path, clip: &AudioClip, format: ExportFormat, depth: BitDepth) -> Result<()> {
    if clip.channel_count() == 0 {
        bail!("nothing to save");
    }
    if !format.supports(depth) {
        bail!("{} does not support {}", format.label(), depth.label());
    }
    match format {
        ExportFormat::Wav => save_wav(path, clip, depth),
        ExportFormat::Aiff => save_aiff(path, clip, depth),
        ExportFormat::Flac => save_flac(path, clip, depth),
    }
    .with_context(|| format!("writing {}", path.display()))
}

fn quantize(sample: f32, bits: u16) -> i32 {
    let scale = (1_i64 << (bits - 1)) as f32;
    let max = scale - 1.0;
    (sample * scale).round().clamp(-scale, max) as i32
}

fn save_wav(path: &Path, clip: &AudioClip, depth: BitDepth) -> Result<()> {
    let sample_format = match depth {
        BitDepth::Float32 => hound::SampleFormat::Float,
        _ => hound::SampleFormat::Int,
    };
    let spec = hound::WavSpec {
        channels: clip.channel_count() as u16,
        sample_rate: clip.sample_rate,
        bits_per_sample: depth.bits(),
        sample_format,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in clip.to_interleaved() {
        match depth {
            BitDepth::Float32 => writer.write_sample(sample)?,
            BitDepth::Int16 => writer.write_sample(quantize(sample, 16) as i16)?,
            BitDepth::Int24 => writer.write_sample(quantize(sample, 24))?,
        }
    }
    writer.finalize()?;
    Ok(())
}

const AIFF_FORM: &[u8; 4] = b"FORM";
const AIFF_TYPE_PCM: &[u8; 4] = b"AIFF";
const AIFF_TYPE_COMPRESSED: &[u8; 4] = b"AIFC";
const AIFF_VERSION_CHUNK: &[u8; 4] = b"FVER";
const AIFF_COMMON_CHUNK: &[u8; 4] = b"COMM";
const AIFF_SOUND_CHUNK: &[u8; 4] = b"SSND";
const AIFC_VERSION_1: u32 = 0xA280_5140;
const AIFC_FLOAT32_TYPE: &[u8; 4] = b"fl32";
const AIFC_FLOAT32_NAME: &[u8] = b"IEEE 32-bit float";

fn save_aiff(path: &Path, clip: &AudioClip, depth: BitDepth) -> Result<()> {
    let is_float = depth == BitDepth::Float32;
    let common = aiff_common_chunk(clip, depth);
    let sound = aiff_sound_chunk(clip, depth);
    let version = if is_float {
        aiff_version_chunk()
    } else {
        Vec::new()
    };
    let form_type = if is_float {
        AIFF_TYPE_COMPRESSED
    } else {
        AIFF_TYPE_PCM
    };
    let body_len = 4 + version.len() + common.len() + sound.len();

    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(AIFF_FORM)?;
    out.write_all(&(body_len as u32).to_be_bytes())?;
    out.write_all(form_type)?;
    out.write_all(&version)?;
    out.write_all(&common)?;
    out.write_all(&sound)?;
    out.flush()?;
    Ok(())
}

fn aiff_chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(payload.len() + 9);
    chunk.extend_from_slice(id);
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        chunk.push(0);
    }
    chunk
}

fn aiff_version_chunk() -> Vec<u8> {
    aiff_chunk(AIFF_VERSION_CHUNK, &AIFC_VERSION_1.to_be_bytes())
}

fn aiff_common_chunk(clip: &AudioClip, depth: BitDepth) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&(clip.channel_count() as i16).to_be_bytes());
    payload.extend_from_slice(&(clip.frames() as u32).to_be_bytes());
    payload.extend_from_slice(&(depth.bits() as i16).to_be_bytes());
    payload.extend_from_slice(&extended_80_bit(clip.sample_rate as f64));
    if depth == BitDepth::Float32 {
        payload.extend_from_slice(AIFC_FLOAT32_TYPE);
        payload.push(AIFC_FLOAT32_NAME.len() as u8);
        payload.extend_from_slice(AIFC_FLOAT32_NAME);
    }
    aiff_chunk(AIFF_COMMON_CHUNK, &payload)
}

fn aiff_sound_chunk(clip: &AudioClip, depth: BitDepth) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    for sample in clip.to_interleaved() {
        match depth {
            BitDepth::Int16 => {
                payload.extend_from_slice(&(quantize(sample, 16) as i16).to_be_bytes())
            }
            BitDepth::Int24 => payload.extend_from_slice(&quantize(sample, 24).to_be_bytes()[1..]),
            BitDepth::Float32 => payload.extend_from_slice(&sample.to_be_bytes()),
        }
    }
    aiff_chunk(AIFF_SOUND_CHUNK, &payload)
}

/// IEEE 754 80-bit extended precision, as required by the AIFF COMM chunk.
fn extended_80_bit(value: f64) -> [u8; 10] {
    let mut bytes = [0_u8; 10];
    if value <= 0.0 {
        return bytes;
    }
    let exponent = value.log2().floor() as i32;
    let mantissa = (value / 2_f64.powi(exponent) * (1_u64 << 63) as f64) as u64;
    let biased = (16383 + exponent) as u16;
    bytes[..2].copy_from_slice(&biased.to_be_bytes());
    bytes[2..].copy_from_slice(&mantissa.to_be_bytes());
    bytes
}

fn save_flac(path: &Path, clip: &AudioClip, depth: BitDepth) -> Result<()> {
    let bits = depth.bits() as usize;
    let samples: Vec<i32> = clip
        .to_interleaved()
        .iter()
        .map(|s| quantize(*s, bits as u16))
        .collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, error)| anyhow!("flac config: {error:?}"))?;
    let source = flacenc::source::MemSource::from_samples(
        &samples,
        clip.channel_count(),
        bits,
        clip.sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|error| anyhow!("flac encode: {error:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|error| anyhow!("flac write: {error:?}"))?;
    let mut bytes = sink.into_inner();
    fix_flac_min_block_size(&mut bytes);
    std::fs::write(path, bytes)?;
    Ok(())
}

const FLAC_STREAMINFO_MIN_BLOCK_OFFSET: usize = 8;
const FLAC_STREAMINFO_MAX_BLOCK_OFFSET: usize = 10;

/// flacenc records the final short block as the minimum block size, but the FLAC spec excludes the
/// last block from that field. Strict decoders (symphonia included) otherwise treat the stream as
/// variable-block-size and reject every frame, so copy the maximum over the minimum.
fn fix_flac_min_block_size(bytes: &mut [u8]) {
    if bytes.len() < FLAC_STREAMINFO_MAX_BLOCK_OFFSET + 2 {
        return;
    }
    let max_block = [
        bytes[FLAC_STREAMINFO_MAX_BLOCK_OFFSET],
        bytes[FLAC_STREAMINFO_MAX_BLOCK_OFFSET + 1],
    ];
    bytes[FLAC_STREAMINFO_MIN_BLOCK_OFFSET..FLAC_STREAMINFO_MIN_BLOCK_OFFSET + 2]
        .copy_from_slice(&max_block);
}

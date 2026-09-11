use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use flacenc::component::BitRepr;
use flacenc::error::Verify;

use super::dither::Dither;
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

/// Everything the writers need beyond the audio itself.
#[derive(Clone, Debug, Default)]
pub struct SaveOptions {
    /// Loop region in frames, written as sampler metadata when the format carries it.
    pub loop_points: Option<Range<usize>>,
}

pub fn save(
    path: &Path,
    clip: &AudioClip,
    format: ExportFormat,
    depth: BitDepth,
    options: &SaveOptions,
) -> Result<()> {
    if clip.channel_count() == 0 {
        bail!("nothing to save");
    }
    if !format.supports(depth) {
        bail!("{} does not support {}", format.label(), depth.label());
    }
    // A loop past the end of the audio would point a sampler at frames that are not there.
    let loop_points = options
        .loop_points
        .clone()
        .map(|range| clip.clamp_range(&range))
        .filter(|range| !range.is_empty());
    match format {
        ExportFormat::Wav => save_wav(path, clip, depth, loop_points.as_ref()),
        ExportFormat::Aiff => save_aiff(path, clip, depth, loop_points.as_ref()),
        // FLAC carries loops only in Vorbis comments, which no sampler agrees on; skipped.
        ExportFormat::Flac => save_flac(path, clip, depth),
    }
    .with_context(|| format!("writing {}", path.display()))
}

/// Rounds float samples to a fixed-point word, adding TPDF dither on the way.
///
/// One quantizer per file, so the noise never repeats within an export.
struct Quantizer {
    bits: u16,
    dither: Dither,
}

impl Quantizer {
    fn new(bits: u16) -> Self {
        Self {
            bits,
            dither: Dither::new(bits),
        }
    }

    fn quantize(&mut self, sample: f32) -> i32 {
        let scale = (1_i64 << (self.bits - 1)) as f32;
        let max = scale - 1.0;
        // Dither is added in full-scale units before scaling, so it stays at ±1 LSB
        // whatever the word length; clamping after keeps it from pushing a full-scale
        // sample past the last representable code.
        let dithered = sample + self.dither.next();
        (dithered * scale).round().clamp(-scale, max) as i32
    }
}

fn save_wav(
    path: &Path,
    clip: &AudioClip,
    depth: BitDepth,
    loop_points: Option<&Range<usize>>,
) -> Result<()> {
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
    let mut quantizer = Quantizer::new(depth.bits());
    for sample in clip.to_interleaved() {
        match depth {
            BitDepth::Float32 => writer.write_sample(sample)?,
            BitDepth::Int16 => writer.write_sample(quantizer.quantize(sample) as i16)?,
            BitDepth::Int24 => writer.write_sample(quantizer.quantize(sample))?,
        }
    }
    writer.finalize()?;
    if let Some(range) = loop_points {
        append_riff_chunk(path, WAV_SAMPLER_CHUNK, &smpl_payload(clip.sample_rate, range))?;
    }
    Ok(())
}

const WAV_SAMPLER_CHUNK: &[u8; 4] = b"smpl";
const MIDI_UNITY_NOTE: u32 = 60;
const SMPL_LOOP_FORWARD: u32 = 0;
const SMPL_LOOP_INFINITE: u32 = 0;
const NANOSECONDS_PER_SECOND: u32 = 1_000_000_000;

/// One forward loop in a `smpl` chunk, as the sampler spec lays it out.
fn smpl_payload(sample_rate: u32, range: &Range<usize>) -> Vec<u8> {
    let sample_period = if sample_rate == 0 {
        0
    } else {
        NANOSECONDS_PER_SECOND / sample_rate
    };
    let mut payload = Vec::with_capacity(60);
    for field in [
        0,                  // manufacturer
        0,                  // product
        sample_period,      // sample period, ns
        MIDI_UNITY_NOTE,    // MIDI unity note
        0,                  // MIDI pitch fraction
        0,                  // SMPTE format
        0,                  // SMPTE offset
        1,                  // loop count
        0,                  // sampler-specific bytes
        0,                  // loop id
        SMPL_LOOP_FORWARD,  // loop type
        range.start as u32, // first frame of the loop
        // The spec counts the end as the last frame *played*, not one past it.
        range.end.saturating_sub(1) as u32,
        0,                    // fractional loop length
        SMPL_LOOP_INFINITE,   // play count
    ] {
        payload.extend_from_slice(&(field as u32).to_le_bytes());
    }
    payload
}

/// Appends a chunk to a finished RIFF file and corrects the file size in its header.
///
/// hound has no way to write extra chunks and does not pad an odd-length `data` chunk, so
/// the pad byte is written here before the new chunk to keep it word-aligned.
fn append_riff_chunk(path: &Path, id: &[u8; 4], payload: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let end = file.seek(SeekFrom::End(0))?;
    if !end.is_multiple_of(2) {
        file.write_all(&[0])?;
    }
    file.write_all(id)?;
    file.write_all(&(payload.len() as u32).to_le_bytes())?;
    file.write_all(payload)?;
    if !payload.len().is_multiple_of(2) {
        file.write_all(&[0])?;
    }
    let total = file.seek(SeekFrom::End(0))?;
    let riff_size = total
        .checked_sub(8)
        .ok_or_else(|| anyhow!("wav file is too short to patch"))? as u32;
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.flush()?;
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

fn save_aiff(
    path: &Path,
    clip: &AudioClip,
    depth: BitDepth,
    loop_points: Option<&Range<usize>>,
) -> Result<()> {
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
    // Markers and the instrument chunk are conventionally written ahead of the sound data.
    let loop_chunks = loop_points.map(aiff_loop_chunks).unwrap_or_default();
    let body_len = 4 + version.len() + common.len() + loop_chunks.len() + sound.len();

    let mut out = BufWriter::new(File::create(path)?);
    out.write_all(AIFF_FORM)?;
    out.write_all(&(body_len as u32).to_be_bytes())?;
    out.write_all(form_type)?;
    out.write_all(&version)?;
    out.write_all(&common)?;
    out.write_all(&loop_chunks)?;
    out.write_all(&sound)?;
    out.flush()?;
    Ok(())
}

const AIFF_MARKER_CHUNK: &[u8; 4] = b"MARK";
const AIFF_INSTRUMENT_CHUNK: &[u8; 4] = b"INST";
const LOOP_START_MARKER: u16 = 1;
const LOOP_END_MARKER: u16 = 2;
const AIFF_LOOP_FORWARD: i16 = 1;
const AIFF_NO_LOOP: i16 = 0;
const AIFF_BASE_NOTE: i8 = 60;

/// MARK naming the two loop points, then INST pointing its sustain loop at them.
fn aiff_loop_chunks(range: &Range<usize>) -> Vec<u8> {
    let mut markers = Vec::new();
    markers.extend_from_slice(&2_u16.to_be_bytes());
    push_aiff_marker(&mut markers, LOOP_START_MARKER, range.start, "loop start");
    // AIFF marks the frame the loop returns from, which is one past the last frame played.
    push_aiff_marker(&mut markers, LOOP_END_MARKER, range.end, "loop end");

    let mut instrument = Vec::new();
    for byte in [AIFF_BASE_NOTE, 0, 0, 127, 0, 127] {
        instrument.push(byte as u8);
    }
    instrument.extend_from_slice(&0_i16.to_be_bytes()); // gain, dB
    instrument.extend_from_slice(&AIFF_LOOP_FORWARD.to_be_bytes());
    instrument.extend_from_slice(&LOOP_START_MARKER.to_be_bytes());
    instrument.extend_from_slice(&LOOP_END_MARKER.to_be_bytes());
    instrument.extend_from_slice(&AIFF_NO_LOOP.to_be_bytes()); // release loop: none
    instrument.extend_from_slice(&0_u16.to_be_bytes());
    instrument.extend_from_slice(&0_u16.to_be_bytes());

    let mut chunks = aiff_chunk(AIFF_MARKER_CHUNK, &markers);
    chunks.extend(aiff_chunk(AIFF_INSTRUMENT_CHUNK, &instrument));
    chunks
}

/// Marker id, position, then the name as a pascal string padded to an even length.
fn push_aiff_marker(out: &mut Vec<u8>, id: u16, position: usize, name: &str) {
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&(position as u32).to_be_bytes());
    let bytes = name.as_bytes();
    out.push(bytes.len() as u8);
    out.extend_from_slice(bytes);
    if !out.len().is_multiple_of(2) {
        out.push(0);
    }
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
    let mut quantizer = Quantizer::new(depth.bits());
    for sample in clip.to_interleaved() {
        match depth {
            BitDepth::Int16 => {
                payload.extend_from_slice(&(quantizer.quantize(sample) as i16).to_be_bytes())
            }
            BitDepth::Int24 => {
                payload.extend_from_slice(&quantizer.quantize(sample).to_be_bytes()[1..])
            }
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
    let mut quantizer = Quantizer::new(bits as u16);
    let samples: Vec<i32> = clip
        .to_interleaved()
        .iter()
        .map(|s| quantizer.quantize(*s))
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

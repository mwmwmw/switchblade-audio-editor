//! Sidecar cache of analysis results, written next to the audio file (or in the user cache
//! directory when that is not writable) and validated against the file's size and mtime.

mod codec;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{anyhow, bail, Context, Result};

use crate::analysis::anomalies::{Anomaly, AnomalyKind, AnomalyReport, AnomalySettings};
use crate::analysis::beats::{Beat, BeatReport, BeatSettings};
use crate::analysis::loudness::LoudnessReport;
use crate::analysis::peaks::PeakMipmap;
use crate::analysis::report::AnalysisReport;
use crate::analysis::spectrum::{BandSpectrum, BAND_COUNT};
use codec::{Reader, Writer};

const MAGIC: &[u8; 4] = b"SBPK";
/// Bumped to 2 when detected beats joined the report; older sidecars are simply rebuilt.
const FORMAT_VERSION: u32 = 2;
pub const SIDECAR_EXTENSION: &str = "sbpk";
const APP_CACHE_DIR: &str = "switchblade";
const KIND_CLIP: u8 = 0;
const KIND_ZERO_RUN: u8 = 1;
const KIND_DISCONTINUITY: u8 = 2;

/// Identifies the exact on-disk file a cache entry was computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fingerprint {
    size: u64,
    modified_secs: u64,
    modified_nanos: u32,
}

impl Fingerprint {
    pub fn of(path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path)?;
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Ok(Self {
            size: metadata.len(),
            modified_secs: modified.as_secs(),
            modified_nanos: modified.subsec_nanos(),
        })
    }
}

pub fn load(audio_path: &Path) -> Option<AnalysisReport> {
    let fingerprint = Fingerprint::of(audio_path).ok()?;
    sidecar_paths(audio_path).into_iter().find_map(|sidecar| {
        let bytes = std::fs::read(&sidecar).ok()?;
        match decode(&bytes, fingerprint) {
            Ok(report) => Some(report),
            Err(error) => {
                log::info!("ignoring cache {}: {error}", sidecar.display());
                None
            }
        }
    })
}

pub fn store(audio_path: &Path, report: &AnalysisReport) -> Result<PathBuf> {
    let fingerprint = Fingerprint::of(audio_path)?;
    let bytes = encode(fingerprint, report);
    let mut last_error = None;
    for sidecar in sidecar_paths(audio_path) {
        if let Some(parent) = sidecar.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&sidecar, &bytes) {
            Ok(()) => return Ok(sidecar),
            Err(error) => last_error = Some(error),
        }
    }
    Err(anyhow!(
        "could not write cache: {}",
        last_error.map_or("no location".into(), |e| e.to_string())
    ))
}

fn sidecar_paths(audio_path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![next_to_file(audio_path)];
    paths.extend(fallback_path(audio_path));
    paths
}

fn next_to_file(audio_path: &Path) -> PathBuf {
    let mut name = audio_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{SIDECAR_EXTENSION}"));
    audio_path.with_file_name(name)
}

fn fallback_path(audio_path: &Path) -> Option<PathBuf> {
    let canonical = audio_path
        .canonicalize()
        .unwrap_or_else(|_| audio_path.to_path_buf());
    let mut hasher = DefaultHasher::new();
    canonical.hash(&mut hasher);
    Some(
        dirs::cache_dir()?
            .join(APP_CACHE_DIR)
            .join(format!("{:016x}.{SIDECAR_EXTENSION}", hasher.finish())),
    )
}

fn encode(fingerprint: Fingerprint, report: &AnalysisReport) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.bytes(MAGIC);
    writer.u32(FORMAT_VERSION);
    writer.u64(fingerprint.size);
    writer.u64(fingerprint.modified_secs);
    writer.u32(fingerprint.modified_nanos);
    encode_settings(&mut writer, &report.settings);
    encode_anomalies(&mut writer, &report.anomalies);
    encode_beats(&mut writer, report.beat_settings, &report.beats);
    encode_loudness(&mut writer, report.loudness.as_ref());
    writer.f32_slice(&report.spectrum.levels_db);
    encode_peaks(&mut writer, &report.peaks);
    writer.into_bytes()
}

fn decode(bytes: &[u8], expected: Fingerprint) -> Result<AnalysisReport> {
    let mut reader = Reader::new(bytes);
    if reader.bytes(4)? != MAGIC {
        bail!("not a Switchblade cache");
    }
    if reader.u32()? != FORMAT_VERSION {
        bail!("unsupported cache version");
    }
    let fingerprint = Fingerprint {
        size: reader.u64()?,
        modified_secs: reader.u64()?,
        modified_nanos: reader.u32()?,
    };
    if fingerprint != expected {
        bail!("audio file changed since the cache was written");
    }
    let settings = decode_settings(&mut reader)?;
    let anomalies = decode_anomalies(&mut reader)?;
    let (beat_settings, beats) = decode_beats(&mut reader)?;
    let report = AnalysisReport {
        settings,
        anomalies,
        beat_settings,
        beats,
        loudness: decode_loudness(&mut reader)?,
        spectrum: decode_spectrum(&mut reader)?,
        peaks: decode_peaks(&mut reader)?,
    };
    if !reader.is_at_end() {
        bail!("trailing bytes in cache");
    }
    Ok(report)
}

fn encode_settings(writer: &mut Writer, settings: &AnomalySettings) {
    writer.f32(settings.clip_threshold);
    writer.u64(settings.clip_min_run as u64);
    writer.u64(settings.zero_min_run as u64);
    writer.f32(settings.discontinuity_jump);
}

fn decode_settings(reader: &mut Reader) -> Result<AnomalySettings> {
    Ok(AnomalySettings {
        clip_threshold: reader.f32()?,
        clip_min_run: reader.u64()? as usize,
        zero_min_run: reader.u64()? as usize,
        discontinuity_jump: reader.f32()?,
    })
}

fn encode_beats(writer: &mut Writer, settings: BeatSettings, report: &BeatReport) {
    writer.u8(settings.sensitivity);
    // BPM is optional; f32::NAN stands in for "could not be estimated".
    writer.f32(report.bpm.unwrap_or(f32::NAN));
    writer.u64(report.beats.len() as u64);
    for beat in &report.beats {
        writer.u64(beat.frame as u64);
        writer.f32(beat.strength);
    }
}

fn decode_beats(reader: &mut Reader) -> Result<(BeatSettings, BeatReport)> {
    let settings = BeatSettings {
        sensitivity: reader.u8()?,
    };
    let bpm = reader.f32()?;
    let count = reader.len_prefix()?;
    let beats = (0..count)
        .map(|_| {
            Ok(Beat {
                frame: reader.u64()? as usize,
                strength: reader.f32()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        settings,
        BeatReport {
            beats,
            bpm: bpm.is_finite().then_some(bpm),
        },
    ))
}

fn encode_anomalies(writer: &mut Writer, report: &AnomalyReport) {
    writer.u64(report.anomalies.len() as u64);
    for anomaly in &report.anomalies {
        writer.u8(kind_code(anomaly.kind));
        writer.u32(anomaly.channel as u32);
        writer.u64(anomaly.range.start as u64);
        writer.u64(anomaly.range.end as u64);
    }
}

fn decode_anomalies(reader: &mut Reader) -> Result<AnomalyReport> {
    let count = reader.len_prefix()?;
    let anomalies = (0..count)
        .map(|_| {
            Ok(Anomaly {
                kind: kind_from_code(reader.u8()?)?,
                channel: reader.u32()? as usize,
                range: reader.u64()? as usize..reader.u64()? as usize,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AnomalyReport { anomalies })
}

fn kind_code(kind: AnomalyKind) -> u8 {
    match kind {
        AnomalyKind::Clip => KIND_CLIP,
        AnomalyKind::ZeroRun => KIND_ZERO_RUN,
        AnomalyKind::Discontinuity => KIND_DISCONTINUITY,
    }
}

fn kind_from_code(code: u8) -> Result<AnomalyKind> {
    match code {
        KIND_CLIP => Ok(AnomalyKind::Clip),
        KIND_ZERO_RUN => Ok(AnomalyKind::ZeroRun),
        KIND_DISCONTINUITY => Ok(AnomalyKind::Discontinuity),
        _ => bail!("unknown anomaly kind {code}"),
    }
}

fn encode_loudness(writer: &mut Writer, loudness: Option<&LoudnessReport>) {
    writer.u8(u8::from(loudness.is_some()));
    if let Some(report) = loudness {
        writer.f64(report.integrated_lufs);
        writer.f64(report.momentary_max_lufs);
        writer.f64(report.short_term_max_lufs);
        writer.f64(report.loudness_range_lu);
        writer.f64(report.true_peak_dbtp);
        writer.f32(report.sample_peak_dbfs);
    }
}

fn decode_loudness(reader: &mut Reader) -> Result<Option<LoudnessReport>> {
    if reader.u8()? == 0 {
        return Ok(None);
    }
    Ok(Some(LoudnessReport {
        integrated_lufs: reader.f64()?,
        momentary_max_lufs: reader.f64()?,
        short_term_max_lufs: reader.f64()?,
        loudness_range_lu: reader.f64()?,
        true_peak_dbtp: reader.f64()?,
        sample_peak_dbfs: reader.f32()?,
    }))
}

fn decode_spectrum(reader: &mut Reader) -> Result<BandSpectrum> {
    let levels = reader.f32_vec()?;
    let levels_db: [f32; BAND_COUNT] = levels
        .try_into()
        .map_err(|_| anyhow!("spectrum band count mismatch"))?;
    Ok(BandSpectrum { levels_db })
}

fn encode_peaks(writer: &mut Writer, peaks: &PeakMipmap) {
    writer.u64(peaks.bucket as u64);
    writer.u64(peaks.mins.len() as u64);
    for (mins, maxs) in peaks.mins.iter().zip(&peaks.maxs) {
        writer.f32_slice(mins);
        writer.f32_slice(maxs);
    }
}

fn decode_peaks(reader: &mut Reader) -> Result<PeakMipmap> {
    let bucket = reader.u64()? as usize;
    let channels = reader.len_prefix()?;
    let mut peaks = PeakMipmap {
        bucket,
        mins: Vec::with_capacity(channels),
        maxs: Vec::with_capacity(channels),
    };
    for _ in 0..channels {
        peaks.mins.push(reader.f32_vec().context("peak minima")?);
        peaks.maxs.push(reader.f32_vec().context("peak maxima")?);
    }
    Ok(peaks)
}

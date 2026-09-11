use anyhow::{anyhow, Result};
use ebur128::{EbuR128, Mode};

use crate::audio::db::linear_to_db;
use crate::audio::AudioClip;

const OFFLINE_BLOCK_FRAMES: usize = 4096;
const ABSOLUTE_SILENCE_LUFS: f64 = -70.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    pub momentary_max_lufs: f64,
    pub short_term_max_lufs: f64,
    pub loudness_range_lu: f64,
    pub true_peak_dbtp: f64,
    pub sample_peak_dbfs: f32,
}

pub fn measure(clip: &AudioClip) -> Result<LoudnessReport> {
    if clip.is_empty() {
        return Ok(LoudnessReport::default());
    }
    let mut meter = new_meter(clip.channel_count() as u32, clip.sample_rate, Mode::all())?;
    let mut momentary_max = ABSOLUTE_SILENCE_LUFS;
    let mut short_term_max = ABSOLUTE_SILENCE_LUFS;
    for start in (0..clip.frames()).step_by(OFFLINE_BLOCK_FRAMES) {
        let end = (start + OFFLINE_BLOCK_FRAMES).min(clip.frames());
        let planes: Vec<&[f32]> = clip.channels.iter().map(|c| &c[start..end]).collect();
        meter.add_frames_planar_f32(&planes).map_err(meter_error)?;
        momentary_max =
            momentary_max.max(meter.loudness_momentary().unwrap_or(ABSOLUTE_SILENCE_LUFS));
        short_term_max =
            short_term_max.max(meter.loudness_shortterm().unwrap_or(ABSOLUTE_SILENCE_LUFS));
    }
    Ok(LoudnessReport {
        integrated_lufs: meter.loudness_global().map_err(meter_error)?,
        momentary_max_lufs: momentary_max,
        short_term_max_lufs: short_term_max,
        loudness_range_lu: meter.loudness_range().map_err(meter_error)?,
        true_peak_dbtp: max_true_peak_dbtp(&meter, clip.channel_count()),
        sample_peak_dbfs: linear_to_db(clip.peak(&clip.full_range())),
    })
}

fn max_true_peak_dbtp(meter: &EbuR128, channels: usize) -> f64 {
    let peak = (0..channels as u32)
        .filter_map(|channel| meter.true_peak(channel).ok())
        .fold(0.0_f64, f64::max);
    linear_to_db(peak as f32) as f64
}

/// Continuously fed meter for the realtime engine.
pub struct RealtimeLoudness {
    meter: EbuR128,
}

impl RealtimeLoudness {
    pub fn new(channels: u32, sample_rate: u32) -> Result<Self> {
        let mode = Mode::M | Mode::S | Mode::I | Mode::TRUE_PEAK;
        Ok(Self {
            meter: new_meter(channels, sample_rate, mode)?,
        })
    }

    pub fn push(&mut self, planes: &[&[f32]]) {
        let _ = self.meter.add_frames_planar_f32(planes);
    }

    pub fn momentary(&self) -> f64 {
        self.meter
            .loudness_momentary()
            .unwrap_or(ABSOLUTE_SILENCE_LUFS)
    }

    pub fn short_term(&self) -> f64 {
        self.meter
            .loudness_shortterm()
            .unwrap_or(ABSOLUTE_SILENCE_LUFS)
    }

    pub fn integrated(&self) -> f64 {
        self.meter
            .loudness_global()
            .unwrap_or(ABSOLUTE_SILENCE_LUFS)
    }

    pub fn true_peak_dbtp(&self) -> f64 {
        max_true_peak_dbtp(&self.meter, self.meter.channels() as usize)
    }
}

fn new_meter(channels: u32, sample_rate: u32, mode: Mode) -> Result<EbuR128> {
    EbuR128::new(channels, sample_rate, mode).map_err(meter_error)
}

fn meter_error(error: ebur128::Error) -> anyhow::Error {
    anyhow!("loudness meter: {error:?}")
}

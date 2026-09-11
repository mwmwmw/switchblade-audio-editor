use crate::audio::db::{linear_to_db, SILENCE_DB};

pub const VU_REFERENCE_PRESETS_DBFS: &[f32] = &[-8.0, -10.0, -12.0, -14.0, -18.0, -20.0];
pub const DEFAULT_VU_REFERENCE_DBFS: f32 = -18.0;
/// A VU meter reaches 99% of a step in 300 ms; that is a 65 ms first-order time constant.
const VU_TIME_CONSTANT_SECONDS: f32 = 0.065;
pub const VU_MIN_DB: f32 = -20.0;
pub const VU_MAX_DB: f32 = 3.0;
pub const MAX_METER_CHANNELS: usize = 8;

/// Per-channel instantaneous peak for the most recent block.
pub struct PeakMeter {
    pub peaks: [f32; MAX_METER_CHANNELS],
    pub channel_count: usize,
}

impl PeakMeter {
    pub fn new(channel_count: usize) -> Self {
        Self {
            peaks: [0.0; MAX_METER_CHANNELS],
            channel_count: channel_count.min(MAX_METER_CHANNELS),
        }
    }

    pub fn measure(&mut self, planes: &[&[f32]]) {
        for (index, plane) in planes.iter().take(self.channel_count).enumerate() {
            self.peaks[index] = plane.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
        }
    }

    pub fn peaks_dbfs(&self) -> [f32; MAX_METER_CHANNELS] {
        let mut out = [SILENCE_DB; MAX_METER_CHANNELS];
        for (db, peak) in out.iter_mut().zip(&self.peaks).take(self.channel_count) {
            *db = linear_to_db(*peak);
        }
        out
    }
}

/// Mono-summed RMS with VU ballistics, read out relative to a configurable 0 VU reference.
pub struct VuMeter {
    smoothed_power: f32,
    sample_rate: f32,
}

impl VuMeter {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            smoothed_power: 0.0,
            sample_rate: sample_rate as f32,
        }
    }

    pub fn measure(&mut self, planes: &[&[f32]]) {
        let frames = planes.first().map_or(0, |p| p.len());
        if frames == 0 || planes.is_empty() {
            return;
        }
        let block_power = mean_power(planes);
        let block_seconds = frames as f32 / self.sample_rate;
        let alpha = 1.0 - (-block_seconds / VU_TIME_CONSTANT_SECONDS).exp();
        self.smoothed_power += alpha * (block_power - self.smoothed_power);
    }

    /// Level in VU: 0 VU corresponds to a sine at `reference_dbfs`.
    pub fn level_vu(&self, reference_dbfs: f32) -> f32 {
        let rms_dbfs = linear_to_db(self.smoothed_power.sqrt());
        let sine_peak_to_rms_db = 20.0 * std::f32::consts::SQRT_2.log10();
        (rms_dbfs + sine_peak_to_rms_db - reference_dbfs).clamp(VU_MIN_DB, VU_MAX_DB)
    }
}

/// Accumulated in f64 so a long block's total keeps its precision before it becomes a dB
/// readout; squares of small samples are exactly where an f32 sum loses the quiet ones.
fn mean_power(planes: &[&[f32]]) -> f32 {
    let total: f64 = planes
        .iter()
        .flat_map(|p| p.iter())
        .map(|s| (*s as f64) * (*s as f64))
        .sum();
    let count: usize = planes.iter().map(|p| p.len()).sum();
    (total / count.max(1) as f64) as f32
}

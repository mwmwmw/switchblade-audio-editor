use std::ops::Range;

use super::db::{db_to_linear, linear_to_db};
use super::AudioClip;

pub const PEAK_PRESETS_DBFS: &[f32] = &[0.0, -0.03, -3.0, -6.0, -12.0, -18.0];

/// Scales `range` so its peak lands on `target_dbfs`. Returns the gain applied, in dB.
pub fn normalize_peak(clip: &mut AudioClip, range: &Range<usize>, target_dbfs: f32) -> Option<f32> {
    let peak = clip.peak(range);
    if peak <= 0.0 {
        return None;
    }
    let gain = db_to_linear(target_dbfs) / peak;
    clip.apply_gain(range, gain);
    Some(linear_to_db(gain))
}

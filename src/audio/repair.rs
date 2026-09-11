//! Repairs the sample jumps that the anomaly scan highlights.
//!
//! A discontinuity is a step between two neighbouring samples large enough that it clicks —
//! a bad edit, a dropped packet, a splice on a non-zero crossing. The repair rewrites a short
//! window around the step, blending the signal into a straight line across it. The blend is
//! weighted by a raised cosine that reaches full strength at the step and falls to nothing at
//! the window edges, so the samples either side are left exactly as they were and the repair
//! cannot introduce a fresh discontinuity of its own.

use std::ops::Range;

use super::AudioClip;

/// Total window rewritten around a jump. Short enough to be inaudible on transient material,
/// long enough to flatten a step at the sample rates this app sees.
pub const DEFAULT_WINDOW_MS: f64 = 1.0;
/// Below this there are too few samples either side to blend, so the repair is skipped.
const MIN_HALF_WINDOW: usize = 4;

/// Smooths every jump of at least `jump_threshold` within `range`. Returns how many it fixed.
pub fn repair_discontinuities(
    clip: &mut AudioClip,
    range: &Range<usize>,
    jump_threshold: f32,
    window_ms: f64,
) -> usize {
    let half = half_window(clip.sample_rate, window_ms);
    let range = clip.clamp_range(range);
    let mut repaired = 0;
    for channel in &mut clip.channels {
        for at in jumps(channel, &range, jump_threshold) {
            if smooth_jump(channel, at, half) {
                repaired += 1;
            }
        }
    }
    repaired
}

/// Counts the jumps that `repair_discontinuities` would act on, without touching the audio.
pub fn count_discontinuities(clip: &AudioClip, range: &Range<usize>, jump_threshold: f32) -> usize {
    let range = clip.clamp_range(range);
    clip.channels
        .iter()
        .map(|channel| jumps(channel, &range, jump_threshold).len())
        .sum()
}

fn half_window(sample_rate: u32, window_ms: f64) -> usize {
    let half = (sample_rate as f64 * window_ms / 2000.0).round() as usize;
    half.max(MIN_HALF_WINDOW)
}

/// Indices of the first sample *after* each jump, matching how the anomaly scan reports them.
fn jumps(samples: &[f32], range: &Range<usize>, threshold: f32) -> Vec<usize> {
    let first = range.start.max(1);
    let last = range.end.min(samples.len());
    (first..last)
        .filter(|index| (samples[*index] - samples[index - 1]).abs() >= threshold)
        .collect()
}

/// Blends `samples` into a straight line across the step at `at`. False if it had no room.
fn smooth_jump(samples: &mut [f32], at: usize, half: usize) -> bool {
    let Some(last_index) = samples.len().checked_sub(1) else {
        return false;
    };
    let start = at.saturating_sub(half);
    let end = (at + half).min(last_index);
    if end <= start + 1 {
        return false;
    }
    // The anchors stay untouched, so the repaired stretch still meets the signal either side.
    let (left, right) = (samples[start], samples[end]);
    let span = (end - start) as f32;
    for index in start + 1..end {
        let t = (index - start) as f32 / span;
        let ramp = left + (right - left) * t;
        // sin²: zero at both anchors, one in the middle where the step is.
        let weight = (std::f32::consts::PI * t).sin().powi(2);
        samples[index] = samples[index] * (1.0 - weight) + ramp * weight;
    }
    true
}

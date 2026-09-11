use std::ops::Range;

use super::AudioClip;

/// Per-channel DC offset over a range, as a fraction of full scale.
///
/// The mean is accumulated in f64: a long file holds millions of samples, and an f32
/// running total loses the offset itself in rounding long before the end.
pub fn measure(clip: &AudioClip, range: &Range<usize>) -> Vec<f32> {
    let range = clip.clamp_range(range);
    if range.is_empty() {
        return vec![0.0; clip.channel_count()];
    }
    clip.channels
        .iter()
        .map(|channel| {
            let sum: f64 = channel[range.clone()].iter().map(|s| *s as f64).sum();
            (sum / range.len() as f64) as f32
        })
        .collect()
}

/// Subtracts each channel's own offset over `range`. Returns the offsets removed.
///
/// Channels are corrected independently because an offset usually comes from the converter
/// that captured them, so the two sides of a stereo file rarely drift by the same amount.
pub fn remove(clip: &mut AudioClip, range: &Range<usize>) -> Vec<f32> {
    let offsets = measure(clip, range);
    let range = clip.clamp_range(range);
    for (channel, offset) in clip.channels.iter_mut().zip(&offsets) {
        if *offset == 0.0 {
            continue;
        }
        for sample in &mut channel[range.clone()] {
            *sample -= offset;
        }
    }
    offsets
}

/// Largest offset across channels, for deciding whether removal is worth offering.
pub fn largest(offsets: &[f32]) -> f32 {
    offsets.iter().fold(0.0_f32, |worst, o| worst.max(o.abs()))
}

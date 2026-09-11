//! Makes a loop region join itself seamlessly.
//!
//! A loop clicks when the waveform at the loop end does not continue into the waveform at the
//! loop start. The fix is to crossfade the audio that *precedes* the loop into the loop's own
//! tail: by the time playback reaches the end of the loop it is already playing what comes
//! immediately before the start, so wrapping round is continuous.
//!
//! The crossfade is a Hann pair — `sin²` rising against `cos²` falling. They sum to one, so
//! correlated material keeps its level through the fade instead of dipping in the middle.

use std::ops::Range;

use anyhow::{bail, Result};

use super::AudioClip;

/// Crossfade length when nothing else is asked for. Long enough to hide a mismatched phase,
/// short enough not to smear a transient sitting near the loop end.
pub const DEFAULT_FADE_MS: f64 = 20.0;
const MIN_FADE_FRAMES: usize = 16;

pub struct LoopFade {
    /// Frames actually crossfaded, which can be shorter than asked for.
    pub frames: usize,
}

/// Crossfades the run-up to `loop_range` over the end of it, in place.
///
/// The audio before the loop is read, not written: only the loop's tail changes, so the
/// region outside the loop plays exactly as it did before.
pub fn crossfade_loop(
    clip: &mut AudioClip,
    loop_range: &Range<usize>,
    fade_ms: f64,
) -> Result<LoopFade> {
    let range = clip.clamp_range(loop_range);
    if range.is_empty() {
        bail!("select the loop region first");
    }
    let wanted = (clip.sample_rate as f64 * fade_ms / 1000.0).round() as usize;
    // The fade needs that many frames of run-up before the loop and has to stay inside the
    // loop itself, so the shorter of the two caps it.
    let fade = wanted.min(range.start).min(range.len() - 1);
    if fade < MIN_FADE_FRAMES {
        if range.start < MIN_FADE_FRAMES {
            bail!("no audio before the loop to crossfade from");
        }
        bail!("loop is too short to crossfade");
    }
    for channel in &mut clip.channels {
        crossfade_channel(channel, &range, fade);
    }
    Ok(LoopFade { frames: fade })
}

fn crossfade_channel(samples: &mut [f32], range: &Range<usize>, fade: usize) {
    // Copied up front: the run-up sits earlier in the same buffer we are about to write into.
    let run_up: Vec<f32> = samples[range.start - fade..range.start].to_vec();
    let tail_start = range.end - fade;
    for (offset, incoming) in run_up.iter().enumerate() {
        let t = (offset + 1) as f32 / (fade + 1) as f32;
        let rising = (std::f32::consts::FRAC_PI_2 * t).sin().powi(2);
        let outgoing = samples[tail_start + offset];
        samples[tail_start + offset] = outgoing * (1.0 - rising) + incoming * rising;
    }
}

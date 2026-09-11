use std::ops::Range;

use super::AudioClip;

const EXPONENTIAL_STEEPNESS: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeCurve {
    Linear,
    EqualPower,
    Exponential,
    Logarithmic,
    SCurve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeDirection {
    In,
    Out,
}

impl FadeCurve {
    pub const ALL: [FadeCurve; 5] = [
        FadeCurve::Linear,
        FadeCurve::EqualPower,
        FadeCurve::Exponential,
        FadeCurve::Logarithmic,
        FadeCurve::SCurve,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FadeCurve::Linear => "Linear",
            FadeCurve::EqualPower => "Equal power",
            FadeCurve::Exponential => "Exponential",
            FadeCurve::Logarithmic => "Logarithmic",
            FadeCurve::SCurve => "S-curve",
        }
    }

    /// Gain of a fade-in at normalised position `t` in `0..=1`.
    pub fn gain_at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => t,
            FadeCurve::EqualPower => (t * std::f32::consts::FRAC_PI_2).sin(),
            FadeCurve::Exponential => exponential_rise(t),
            FadeCurve::Logarithmic => 1.0 - exponential_rise(1.0 - t),
            FadeCurve::SCurve => t * t * (3.0 - 2.0 * t),
        }
    }
}

fn exponential_rise(t: f32) -> f32 {
    ((EXPONENTIAL_STEEPNESS * t).exp() - 1.0) / (EXPONENTIAL_STEEPNESS.exp() - 1.0)
}

pub fn apply_fade(
    clip: &mut AudioClip,
    range: &Range<usize>,
    curve: FadeCurve,
    direction: FadeDirection,
) {
    let range = clip.clamp_range(range);
    let length = range.len();
    if length < 2 {
        return;
    }
    for channel in &mut clip.channels {
        for (index, sample) in channel[range.clone()].iter_mut().enumerate() {
            let t = index as f32 / (length - 1) as f32;
            let position = match direction {
                FadeDirection::In => t,
                FadeDirection::Out => 1.0 - t,
            };
            *sample *= curve.gain_at(position);
        }
    }
}

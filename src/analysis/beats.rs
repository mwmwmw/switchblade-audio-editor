//! Onset and tempo detection, ported from the m8-groove extractor's JavaScript.
//!
//! The chain is the standard one for percussive material:
//!
//! 1. `onset_envelope` slides a Hann-windowed FFT over a mono mixdown and sums, per frame,
//!    how much each bin's magnitude *grew* since the previous frame. Drops are ignored, so
//!    the curve spikes on attacks whatever their pitch.
//! 2. `pick_onsets` turns that curve into discrete hits: local maxima that also clear their
//!    own neighbourhood average, refined to sub-frame accuracy.
//! 3. `estimate_bpm` autocorrelates the curve and scores every beat-length lag.
//!
//! Sustained material gives a mushy envelope and unreliable hits — this is aimed at drums,
//! loops and anything else with clear attacks.

use std::f32::consts::PI;

use rustfft::{num_complex::Complex, FftPlanner};

use crate::audio::AudioClip;

const FFT_SIZE: usize = 1024;
const HOP_SIZE: usize = 256;
const MIN_BPM: f64 = 55.0;
const MAX_BPM: f64 = 220.0;
/// Flux is normalised against this percentile so the thresholds behave the same at any level.
const NORMALISE_PERCENTILE: f32 = 0.95;
const FLUX_CEILING: f32 = 4.0;
/// Radius of the local-maximum test, in seconds.
const LOCAL_MAX_SECONDS: f64 = 0.03;
/// Radius of the local-average test, in seconds.
const LOCAL_AVERAGE_SECONDS: f64 = 0.35;
/// Hits closer together than this collapse into the stronger one.
const MIN_HIT_GAP_SECONDS: f64 = 0.05;
/// Below this many frames there is not enough signal to analyse.
const MIN_ANALYSIS_FRAMES: usize = 2048;
/// The tempo prior is centred here, which breaks ties between a tempo and half or double it.
const TEMPO_PRIOR_CENTRE_BPM: f64 = 120.0;
const TEMPO_PRIOR_WIDTH: f64 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatSettings {
    /// 1 is strict, 10 permissive. The m8-groove slider's default is 6.
    pub sensitivity: u8,
}

impl Default for BeatSettings {
    fn default() -> Self {
        Self { sensitivity: 6 }
    }
}

impl BeatSettings {
    pub const MIN_SENSITIVITY: u8 = 1;
    pub const MAX_SENSITIVITY: u8 = 10;

    fn threshold(self) -> f32 {
        let sensitivity = self
            .sensitivity
            .clamp(Self::MIN_SENSITIVITY, Self::MAX_SENSITIVITY);
        0.04 + (10 - sensitivity) as f32 * 0.045
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beat {
    /// Position in the clip, in frames.
    pub frame: usize,
    /// How far the flux peak stood above its neighbourhood; bigger means a firmer hit.
    pub strength: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeatReport {
    pub beats: Vec<Beat>,
    /// None when the envelope was too short or too flat to autocorrelate.
    pub bpm: Option<f32>,
}

impl BeatReport {
    /// The detected hit nearest `frame`, if one lies within `tolerance` frames.
    pub fn nearest(&self, frame: usize, tolerance: usize) -> Option<usize> {
        self.beats
            .iter()
            .map(|beat| beat.frame)
            .min_by_key(|candidate| candidate.abs_diff(frame))
            .filter(|candidate| candidate.abs_diff(frame) <= tolerance)
    }
}

pub fn detect(clip: &AudioClip, settings: &BeatSettings) -> BeatReport {
    let mono = mono_mixdown(clip);
    if mono.len() < MIN_ANALYSIS_FRAMES || clip.sample_rate == 0 {
        return BeatReport::default();
    }
    let envelope = onset_envelope(&mono);
    let hop_seconds = HOP_SIZE as f64 / clip.sample_rate as f64;
    let last_frame = clip.frames();
    let beats = pick_onsets(&envelope, hop_seconds, *settings)
        .into_iter()
        .map(|(frame, strength)| Beat {
            frame: (frame_to_sample(frame).round().max(0.0) as usize).min(last_frame),
            strength,
        })
        .collect();
    BeatReport {
        beats,
        bpm: estimate_bpm(&envelope.flux, hop_seconds).map(|bpm| bpm as f32),
    }
}

fn mono_mixdown(clip: &AudioClip) -> Vec<f32> {
    match clip.channels.len() {
        0 => Vec::new(),
        1 => clip.channels[0].clone(),
        count => {
            let scale = 1.0 / count as f32;
            (0..clip.frames())
                .map(|frame| {
                    clip.channels
                        .iter()
                        .map(|channel| channel[frame])
                        .sum::<f32>()
                        * scale
                })
                .collect()
        }
    }
}

struct Envelope {
    flux: Vec<f32>,
}

/// Where in the original audio a flux peak at `frame` actually happened, in frames.
///
/// The JS this is ported from times a hit at its window's start and lets the downstream grid
/// fit absorb the resulting bias. Snapping has no grid fit to absorb anything, so the hit is
/// placed properly here: window `f` spans padded samples `[f·HOP, f·HOP + FFT_SIZE)`, and the
/// only part it did not share with window `f-1` is its last HOP samples. New energy has to be
/// in there, so the attack is timed to the middle of that slice. Undoing the FFT_SIZE lead-in
/// leaves `(f - 0.5)·HOP`, which also keeps a hit at sample 0 from timing out negative.
fn frame_to_sample(frame: f64) -> f64 {
    (frame - 0.5) * HOP_SIZE as f64
}

/// Spectral-flux onset envelope.
///
/// The samples get a silent lead-in of `FFT_SIZE` zeros first. Without it a hit at the very
/// first sample is already present in frame 0's spectrum — and frame 0's flux is forced to
/// zero — so a pre-trimmed loop would lose its first hit. Frame times therefore run one
/// lead-in early and callers subtract it.
fn onset_envelope(samples: &[f32]) -> Envelope {
    let mut padded = vec![0.0; FFT_SIZE + samples.len()];
    padded[FFT_SIZE..].copy_from_slice(samples);
    let frame_count = (padded.len() - FFT_SIZE) / HOP_SIZE + 1;
    let window = hann_window();
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
    let mut buffer = vec![Complex::new(0.0, 0.0); FFT_SIZE];
    let mut previous = vec![0.0_f32; FFT_SIZE / 2];
    let mut flux = vec![0.0_f32; frame_count];

    for frame in 0..frame_count {
        let start = frame * HOP_SIZE;
        for (index, slot) in buffer.iter_mut().enumerate() {
            *slot = Complex::new(padded[start + index] * window[index], 0.0);
        }
        fft.process(&mut buffer);
        let mut growth = 0.0;
        for (bin, magnitude) in buffer[..FFT_SIZE / 2].iter().enumerate() {
            let magnitude = magnitude.norm();
            let increase = magnitude - previous[bin];
            if increase > 0.0 {
                growth += increase;
            }
            previous[bin] = magnitude;
        }
        // Frame 0 has no predecessor, so every bin would read as growth.
        flux[frame] = if frame == 0 { 0.0 } else { growth };
    }
    normalise(&mut flux);
    Envelope { flux }
}

fn hann_window() -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|index| 0.5 - 0.5 * (2.0 * PI * index as f32 / (FFT_SIZE - 1) as f32).cos())
        .collect()
}

/// Scales the curve by its 95th percentile and clips the result, so one enormous transient
/// cannot flatten everything else into the noise.
fn normalise(flux: &mut [f32]) {
    if flux.is_empty() {
        return;
    }
    let mut sorted = flux.to_vec();
    sorted.sort_by(f32::total_cmp);
    let index = (NORMALISE_PERCENTILE * (sorted.len() - 1) as f32) as usize;
    let reference = sorted[index];
    let reference = if reference > 0.0 { reference } else { 1.0 };
    for value in flux {
        *value = (*value / reference).min(FLUX_CEILING);
    }
}

/// Peak-picks the envelope into hits, returned as (seconds, strength) pairs.
///
/// A frame counts as a hit when it is the local maximum within ±30 ms *and* clears the
/// ±350 ms local average by the sensitivity threshold.
fn pick_onsets(envelope: &Envelope, hop_seconds: f64, settings: BeatSettings) -> Vec<(f64, f32)> {
    let min_gap_frames = MIN_HIT_GAP_SECONDS / hop_seconds;
    let flux = &envelope.flux;
    let frame_count = flux.len();
    if frame_count < 3 {
        return Vec::new();
    }
    let local_max_radius = ((LOCAL_MAX_SECONDS / hop_seconds).round() as usize).max(1);
    let average_radius = ((LOCAL_AVERAGE_SECONDS / hop_seconds).round() as usize).max(2);
    let threshold = settings.threshold();
    let mut candidates: Vec<(f64, f32)> = Vec::new();

    for frame in 1..frame_count - 1 {
        let value = flux[frame];
        if value < threshold {
            continue;
        }
        let window = neighbourhood(frame, local_max_radius, frame_count);
        // The index tie-break keeps a plateau from registering as several hits.
        let is_local_max = !window
            .clone()
            .any(|other| flux[other] > value || (flux[other] == value && other < frame));
        if !is_local_max {
            continue;
        }
        let window = neighbourhood(frame, average_radius, frame_count);
        let count = window.len();
        let local_average = window.map(|index| flux[index]).sum::<f32>() / count as f32;
        if value <= local_average + threshold {
            continue;
        }
        let shift = parabolic_shift(flux[frame - 1], value, flux[frame + 1]);
        candidates.push((frame as f64 + shift as f64, value - local_average));
    }
    merge_close_hits(candidates, min_gap_frames)
}

fn neighbourhood(centre: usize, radius: usize, count: usize) -> std::ops::Range<usize> {
    centre.saturating_sub(radius)..(centre + radius + 1).min(count)
}

/// Sub-sample peak position from three points, in samples either side of the centre.
///
/// Returns zero when the three points do not form a peak, which keeps a flat or noisy
/// neighbourhood from throwing the estimate more than half a frame.
fn parabolic_shift(left: f32, peak: f32, right: f32) -> f32 {
    let curvature = left - 2.0 * peak + right;
    if curvature == 0.0 {
        return 0.0;
    }
    let shift = (left - right) / (2.0 * curvature);
    if !shift.is_finite() || shift.abs() > 0.5 {
        0.0
    } else {
        shift
    }
}

fn merge_close_hits(candidates: Vec<(f64, f32)>, min_gap_frames: f64) -> Vec<(f64, f32)> {
    let mut hits: Vec<(f64, f32)> = Vec::with_capacity(candidates.len());
    for hit in candidates {
        match hits.last_mut() {
            Some(last) if hit.0 - last.0 < min_gap_frames => {
                if hit.1 > last.1 {
                    *last = hit;
                }
            }
            _ => hits.push(hit),
        }
    }
    hits
}

/// Tempo via autocorrelation of the onset envelope.
///
/// Every beat-length lag is scored by its correlation plus half the correlation at twice the
/// lag, so a true beat outscores its own subdivisions, then weighted by a log-normal prior
/// centred on 120 BPM to break octave ties. The winner is refined by parabolic interpolation.
fn estimate_bpm(flux: &[f32], hop_seconds: f64) -> Option<f64> {
    let frame_count = flux.len();
    if frame_count < 4 {
        return None;
    }
    let mean = flux.iter().map(|v| *v as f64).sum::<f64>() / frame_count as f64;
    let centered: Vec<f64> = flux.iter().map(|v| *v as f64 - mean).collect();

    let min_lag = ((60.0 / MAX_BPM / hop_seconds).round() as usize).max(2);
    let max_lag = ((60.0 / MIN_BPM / hop_seconds).round() as usize).min(frame_count / 2);
    if max_lag <= min_lag + 2 {
        return None;
    }
    let variance = centered.iter().map(|v| v * v).sum::<f64>() / frame_count as f64;
    if variance <= 0.0 {
        return None;
    }
    // Lags beyond max_lag are still needed for the harmonic term at twice the lag.
    let lag_cap = (2 * max_lag).min(frame_count - 1);
    let mut correlation = vec![0.0_f64; lag_cap + 1];
    for (lag, slot) in correlation.iter_mut().enumerate().skip(min_lag) {
        let overlap: f64 = (0..frame_count - lag)
            .map(|index| centered[index] * centered[index + lag])
            .sum();
        // Dividing by the overlap length keeps long lags from being penalised for it.
        *slot = overlap / (frame_count - lag) as f64 / variance;
    }

    let mut best_lag = None;
    let mut best_score = f64::NEG_INFINITY;
    for lag in min_lag..=max_lag {
        let harmonic = correlation[lag]
            + 0.5
                * if 2 * lag <= lag_cap {
                    correlation[2 * lag]
                } else {
                    0.0
                };
        let bpm = 60.0 / (lag as f64 * hop_seconds);
        let prior =
            (-0.5 * ((bpm / TEMPO_PRIOR_CENTRE_BPM).log2() / TEMPO_PRIOR_WIDTH).powi(2)).exp();
        let score = harmonic * prior;
        if score > best_score {
            best_score = score;
            best_lag = Some(lag);
        }
    }
    let best_lag = best_lag?;
    let shift = if best_lag > min_lag && best_lag < max_lag {
        parabolic_shift(
            correlation[best_lag - 1] as f32,
            correlation[best_lag] as f32,
            correlation[best_lag + 1] as f32,
        ) as f64
    } else {
        0.0
    };
    Some(60.0 / ((best_lag as f64 + shift) * hop_seconds))
}

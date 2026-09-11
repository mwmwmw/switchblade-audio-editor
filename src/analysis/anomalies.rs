use std::ops::Range;

use crate::audio::AudioClip;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnomalySettings {
    /// Absolute sample value at or above which a sample counts as clipped.
    pub clip_threshold: f32,
    pub clip_min_run: usize,
    /// Consecutive exact zeros needed before a run is flagged as a dropout.
    pub zero_min_run: usize,
    /// Sample-to-sample jump that counts as a discontinuity.
    pub discontinuity_jump: f32,
}

impl Default for AnomalySettings {
    fn default() -> Self {
        Self {
            clip_threshold: 0.999,
            clip_min_run: 2,
            zero_min_run: 32,
            discontinuity_jump: 0.5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnomalyKind {
    Clip,
    ZeroRun,
    Discontinuity,
}

impl AnomalyKind {
    pub const ALL: [AnomalyKind; 3] = [
        AnomalyKind::Clip,
        AnomalyKind::ZeroRun,
        AnomalyKind::Discontinuity,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AnomalyKind::Clip => "Clips",
            AnomalyKind::ZeroRun => "Zero runs",
            AnomalyKind::Discontinuity => "Discontinuities",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Anomaly {
    pub kind: AnomalyKind,
    pub channel: usize,
    pub range: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct AnomalyReport {
    pub anomalies: Vec<Anomaly>,
}

impl AnomalyReport {
    pub fn count(&self, kind: AnomalyKind) -> usize {
        self.anomalies.iter().filter(|a| a.kind == kind).count()
    }
}

pub fn scan(clip: &AudioClip, settings: &AnomalySettings) -> AnomalyReport {
    let mut anomalies = Vec::new();
    for (channel, samples) in clip.channels.iter().enumerate() {
        push_runs(
            &mut anomalies,
            AnomalyKind::Clip,
            channel,
            clip_runs(samples, settings),
        );
        push_runs(
            &mut anomalies,
            AnomalyKind::ZeroRun,
            channel,
            zero_runs(samples, settings),
        );
        push_runs(
            &mut anomalies,
            AnomalyKind::Discontinuity,
            channel,
            discontinuities(samples, settings),
        );
    }
    anomalies.sort_by_key(|a| a.range.start);
    AnomalyReport { anomalies }
}

fn push_runs(out: &mut Vec<Anomaly>, kind: AnomalyKind, channel: usize, runs: Vec<Range<usize>>) {
    out.extend(runs.into_iter().map(|range| Anomaly {
        kind,
        channel,
        range,
    }));
}

fn clip_runs(samples: &[f32], settings: &AnomalySettings) -> Vec<Range<usize>> {
    let threshold = settings.clip_threshold;
    find_runs(samples, settings.clip_min_run, |s| s.abs() >= threshold)
}

fn zero_runs(samples: &[f32], settings: &AnomalySettings) -> Vec<Range<usize>> {
    find_runs(samples, settings.zero_min_run, |s| s == 0.0)
}

fn discontinuities(samples: &[f32], settings: &AnomalySettings) -> Vec<Range<usize>> {
    samples
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| (pair[1] - pair[0]).abs() >= settings.discontinuity_jump)
        .map(|(index, _)| index + 1..index + 2)
        .collect()
}

/// Ranges of consecutive samples matching `predicate`, at least `min_run` long.
fn find_runs(
    samples: &[f32],
    min_run: usize,
    predicate: impl Fn(f32) -> bool,
) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut run_start: Option<usize> = None;
    for (index, sample) in samples.iter().enumerate() {
        match (predicate(*sample), run_start) {
            (true, None) => run_start = Some(index),
            (false, Some(start)) => {
                push_if_long_enough(&mut runs, start..index, min_run);
                run_start = None;
            }
            _ => {}
        }
    }
    if let Some(start) = run_start {
        push_if_long_enough(&mut runs, start..samples.len(), min_run);
    }
    runs
}

fn push_if_long_enough(runs: &mut Vec<Range<usize>>, range: Range<usize>, min_run: usize) {
    if range.len() >= min_run.max(1) {
        runs.push(range);
    }
}

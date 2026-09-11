use super::anomalies::{self, AnomalyReport, AnomalySettings};
use super::loudness::{self, LoudnessReport};
use super::peaks::PeakMipmap;
use super::spectrum::{average_band_spectrum, BandSpectrum};
use crate::audio::AudioClip;

/// Everything derived from a clip that is worth caching between sessions.
#[derive(Clone, Debug, Default)]
pub struct AnalysisReport {
    pub settings: AnomalySettings,
    pub anomalies: AnomalyReport,
    pub loudness: Option<LoudnessReport>,
    pub spectrum: BandSpectrum,
    pub peaks: PeakMipmap,
}

pub fn analyze(clip: &AudioClip, settings: &AnomalySettings) -> AnalysisReport {
    AnalysisReport {
        settings: *settings,
        anomalies: anomalies::scan(clip, settings),
        loudness: loudness::measure(clip).ok(),
        spectrum: average_band_spectrum(clip),
        peaks: PeakMipmap::build(clip),
    }
}

impl AnalysisReport {
    pub fn is_current(&self, clip: &AudioClip, settings: &AnomalySettings) -> bool {
        self.settings == *settings && self.peaks.matches(clip)
    }
}

/// Reuses a cached report, redoing only the anomaly scan when the highlight settings differ.
pub fn analyze_with_cached(
    clip: &AudioClip,
    settings: &AnomalySettings,
    cached: AnalysisReport,
) -> AnalysisReport {
    if cached.is_current(clip, settings) {
        return cached;
    }
    AnalysisReport {
        settings: *settings,
        anomalies: anomalies::scan(clip, settings),
        peaks: if cached.peaks.matches(clip) {
            cached.peaks
        } else {
            PeakMipmap::build(clip)
        },
        ..cached
    }
}

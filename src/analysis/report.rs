use super::anomalies::{self, AnomalyReport, AnomalySettings};
use super::beats::{self, BeatReport, BeatSettings};
use super::loudness::{self, LoudnessReport};
use super::peaks::PeakMipmap;
use super::spectrum::{average_band_spectrum, BandSpectrum};
use crate::audio::AudioClip;

/// Everything derived from a clip that is worth caching between sessions.
#[derive(Clone, Debug, Default)]
pub struct AnalysisReport {
    pub settings: AnomalySettings,
    pub beat_settings: BeatSettings,
    pub anomalies: AnomalyReport,
    pub beats: BeatReport,
    pub loudness: Option<LoudnessReport>,
    pub spectrum: BandSpectrum,
    pub peaks: PeakMipmap,
}

pub fn analyze(clip: &AudioClip, settings: &AnomalySettings) -> AnalysisReport {
    let beat_settings = BeatSettings::default();
    AnalysisReport {
        settings: *settings,
        beat_settings,
        anomalies: anomalies::scan(clip, settings),
        beats: beats::detect(clip, &beat_settings),
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
    let peaks_valid = cached.peaks.matches(clip);
    AnalysisReport {
        settings: *settings,
        anomalies: anomalies::scan(clip, settings),
        // Beat detection is the expensive pass, so it is only redone when the audio itself
        // changed — a different highlight threshold cannot move a drum hit.
        beats: if peaks_valid {
            cached.beats
        } else {
            beats::detect(clip, &cached.beat_settings)
        },
        peaks: if peaks_valid {
            cached.peaks
        } else {
            PeakMipmap::build(clip)
        },
        ..cached
    }
}

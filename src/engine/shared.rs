use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use parking_lot::Mutex;

use crate::analysis::loudness::RealtimeLoudness;
use crate::analysis::meters::{PeakMeter, VuMeter, DEFAULT_VU_REFERENCE_DBFS, MAX_METER_CHANNELS};
use crate::audio::db::SILENCE_DB;
use crate::plugins::stack::ProcessingStack;

pub const NO_SEEK: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MeterSource {
    #[default]
    Idle,
    Playback,
    Input,
}

#[derive(Clone, Copy, Debug)]
pub struct MeterSnapshot {
    pub source: MeterSource,
    pub channel_count: usize,
    pub peak_dbfs: [f32; MAX_METER_CHANNELS],
    pub vu: f32,
    pub lufs_momentary: f64,
    pub lufs_short_term: f64,
    pub lufs_integrated: f64,
    pub true_peak_dbtp: f64,
}

impl Default for MeterSnapshot {
    fn default() -> Self {
        Self {
            source: MeterSource::Idle,
            channel_count: 2,
            peak_dbfs: [SILENCE_DB; MAX_METER_CHANNELS],
            vu: crate::analysis::meters::VU_MIN_DB,
            lufs_momentary: -70.0,
            lufs_short_term: -70.0,
            lufs_integrated: -70.0,
            true_peak_dbtp: SILENCE_DB as f64,
        }
    }
}

/// State shared between the UI thread and the audio callbacks.
pub struct SharedState {
    /// Playback position in document frames.
    pub play_position: AtomicUsize,
    pub seek_request: AtomicUsize,
    pub playing: AtomicBool,
    pub recording: AtomicBool,
    vu_reference_bits: AtomicU32,
    pub meters: Mutex<MeterSnapshot>,
    pub stack: Mutex<ProcessingStack>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            play_position: AtomicUsize::new(0),
            seek_request: AtomicUsize::new(NO_SEEK),
            playing: AtomicBool::new(false),
            recording: AtomicBool::new(false),
            vu_reference_bits: AtomicU32::new(DEFAULT_VU_REFERENCE_DBFS.to_bits()),
            meters: Mutex::new(MeterSnapshot::default()),
            stack: Mutex::new(ProcessingStack::default()),
        }
    }
}

impl SharedState {
    pub fn vu_reference_dbfs(&self) -> f32 {
        f32::from_bits(self.vu_reference_bits.load(Ordering::Relaxed))
    }

    pub fn set_vu_reference_dbfs(&self, value: f32) {
        self.vu_reference_bits
            .store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::Relaxed)
    }

    pub fn request_seek(&self, frame: usize) {
        self.seek_request.store(frame, Ordering::Relaxed);
    }

    pub fn publish_meters(&self, snapshot: MeterSnapshot) {
        if let Some(mut meters) = self.meters.try_lock() {
            *meters = snapshot;
        }
    }

    pub fn clear_meters(&self) {
        *self.meters.lock() = MeterSnapshot::default();
    }
}

/// Integrated loudness and true peak walk the whole gating history, so refresh them less often.
const SLOW_READOUT_INTERVAL: u32 = 8;

/// Peak, VU and LUFS meters run together on the audio thread.
pub struct MeterChain {
    source: MeterSource,
    peak: PeakMeter,
    vu: VuMeter,
    loudness: Option<RealtimeLoudness>,
    blocks: u32,
    integrated: f64,
    true_peak_dbtp: f64,
}

impl MeterChain {
    pub fn new(source: MeterSource, channels: usize, sample_rate: u32) -> Self {
        Self {
            source,
            peak: PeakMeter::new(channels),
            vu: VuMeter::new(sample_rate),
            loudness: RealtimeLoudness::new(channels as u32, sample_rate).ok(),
            blocks: 0,
            integrated: -70.0,
            true_peak_dbtp: SILENCE_DB as f64,
        }
    }

    fn refresh_slow_readouts(&mut self) {
        self.blocks = self.blocks.wrapping_add(1);
        if !self.blocks.is_multiple_of(SLOW_READOUT_INTERVAL) {
            return;
        }
        if let Some(loudness) = &self.loudness {
            self.integrated = loudness.integrated();
            self.true_peak_dbtp = loudness.true_peak_dbtp();
        }
    }

    pub fn measure(&mut self, planes: &[&[f32]], vu_reference_dbfs: f32) -> MeterSnapshot {
        self.peak.measure(planes);
        self.vu.measure(planes);
        if let Some(loudness) = &mut self.loudness {
            loudness.push(planes);
        }
        self.refresh_slow_readouts();
        MeterSnapshot {
            source: self.source,
            channel_count: planes.len().min(MAX_METER_CHANNELS),
            peak_dbfs: self.peak.peaks_dbfs(),
            vu: self.vu.level_vu(vu_reference_dbfs),
            lufs_momentary: self
                .loudness
                .as_ref()
                .map_or(-70.0, RealtimeLoudness::momentary),
            lufs_short_term: self
                .loudness
                .as_ref()
                .map_or(-70.0, RealtimeLoudness::short_term),
            lufs_integrated: self.integrated,
            true_peak_dbtp: self.true_peak_dbtp,
        }
    }
}

use std::ops::Range;
use std::sync::Arc;

use crossbeam_channel::Receiver;

use crate::analysis::loudness::{self, LoudnessReport};
use crate::analysis::spectrum::{average_band_spectrum, BandSpectrum};
use crate::audio::AudioClip;
use crate::document::Document;

#[derive(Clone, PartialEq, Eq)]
struct RegionKey {
    version: u64,
    range: Range<usize>,
}

pub struct RegionReport {
    pub spectrum: BandSpectrum,
    pub loudness: Option<LoudnessReport>,
}

/// Re-runs the spectrum and loudness measurements on the current selection, off the UI thread.
#[derive(Default)]
pub struct RegionAnalysis {
    current: Option<(RegionKey, RegionReport)>,
    pending: Option<(RegionKey, Receiver<RegionReport>)>,
}

impl RegionAnalysis {
    /// Starts a job for the current selection, so the readouts track a drag as it happens.
    ///
    /// Only one job runs at a time: a drag that moves the edges every frame queues nothing, it
    /// just measures again from wherever the selection has got to once the last job lands.
    pub fn ensure(&mut self, doc: &Document, ctx: &egui::Context) {
        let Some(range) = doc.selection.clone() else {
            return;
        };
        let key = RegionKey {
            version: doc.version,
            range,
        };
        let already_known = self.current.as_ref().is_some_and(|(k, _)| *k == key);
        if already_known || self.pending.is_some() {
            return;
        }
        self.pending = Some((
            key.clone(),
            spawn(Arc::clone(&doc.clip), key.range, ctx.clone()),
        ));
    }

    pub fn poll(&mut self) {
        let Some((key, rx)) = &self.pending else {
            return;
        };
        if let Ok(report) = rx.try_recv() {
            self.current = Some((key.clone(), report));
            self.pending = None;
        }
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The report for the document's current selection.
    ///
    /// While a job is in flight the last report for this version stands in, so a drag updates
    /// the readouts continuously instead of blanking them between measurements.
    pub fn for_selection(&self, doc: &Document) -> Option<&RegionReport> {
        let selection = doc.selection.as_ref()?;
        let (key, report) = self.current.as_ref()?;
        let usable =
            key.version == doc.version && (key.range == *selection || self.pending.is_some());
        usable.then_some(report)
    }
}

fn spawn(clip: Arc<AudioClip>, range: Range<usize>, ctx: egui::Context) -> Receiver<RegionReport> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("switchblade-region".into())
        .spawn(move || {
            let region = clip.slice(&range);
            let report = RegionReport {
                spectrum: average_band_spectrum(&region),
                loudness: loudness::measure(&region).ok(),
            };
            let _ = tx.send(report);
            ctx.request_repaint();
        })
        .expect("spawning region analysis thread");
    rx
}

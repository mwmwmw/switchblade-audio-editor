use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::Receiver;

use crate::analysis::anomalies::AnomalySettings;
use crate::analysis::report::{analyze, analyze_with_cached, AnalysisReport};
use crate::audio::AudioClip;
use crate::cache;
use crate::document::Document;

pub struct AnalysisResult {
    pub version: u64,
    pub report: AnalysisReport,
}

/// Runs the offline analyses on a background thread whenever the document changes,
/// seeding from and writing back to the sidecar cache when the document mirrors a file on disk.
#[derive(Default)]
pub struct AnalysisState {
    pub current: Option<AnalysisResult>,
    pending: Option<Receiver<AnalysisResult>>,
    requested_version: u64,
    seed: Option<(u64, AnalysisReport)>,
}

impl AnalysisState {
    /// Registers a cached report for a freshly installed document version.
    pub fn seed(&mut self, version: u64, report: AnalysisReport) {
        self.seed = Some((version, report));
    }

    pub fn ensure(&mut self, doc: &Document, settings: &AnomalySettings, ctx: &egui::Context) {
        if self.requested_version == doc.version {
            return;
        }
        self.requested_version = doc.version;
        let cached = self
            .seed
            .take()
            .filter(|(version, _)| *version == doc.version)
            .map(|(_, report)| report);
        let persist_to = if doc.dirty { None } else { doc.path.clone() };
        let job = Job {
            clip: Arc::clone(&doc.clip),
            version: doc.version,
            settings: *settings,
            cached,
            persist_to,
        };
        self.pending = Some(job.spawn(ctx.clone()));
    }

    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        if let Ok(result) = rx.try_recv() {
            if result.version == self.requested_version {
                self.current = Some(result);
                self.pending = None;
            }
        }
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn for_version(&self, version: u64) -> Option<&AnalysisReport> {
        self.current
            .as_ref()
            .filter(|r| r.version == version)
            .map(|r| &r.report)
    }

    pub fn invalidate(&mut self) {
        self.requested_version = 0;
    }

    /// Writes the current report to the sidecar for `path`, e.g. right after the file was saved.
    pub fn persist(&self, version: u64, path: PathBuf) {
        let Some(report) = self.for_version(version).cloned() else {
            return;
        };
        std::thread::spawn(move || store_cache(&path, &report));
    }
}

struct Job {
    clip: Arc<AudioClip>,
    version: u64,
    settings: AnomalySettings,
    cached: Option<AnalysisReport>,
    persist_to: Option<PathBuf>,
}

impl Job {
    fn spawn(self, ctx: egui::Context) -> Receiver<AnalysisResult> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::Builder::new()
            .name("switchblade-analysis".into())
            .spawn(move || {
                let report = self.run();
                let _ = tx.send(AnalysisResult {
                    version: self.version,
                    report,
                });
                ctx.request_repaint();
            })
            .expect("spawning analysis thread");
        rx
    }

    fn run(&self) -> AnalysisReport {
        let (report, recomputed) = match self.cached.clone() {
            Some(cached) if cached.is_current(&self.clip, &self.settings) => (cached, false),
            Some(cached) => (
                analyze_with_cached(&self.clip, &self.settings, cached),
                true,
            ),
            None => (analyze(&self.clip, &self.settings), true),
        };
        if let (true, Some(path)) = (recomputed, &self.persist_to) {
            store_cache(path, &report);
        }
        report
    }
}

fn store_cache(path: &Path, report: &AnalysisReport) {
    match cache::store(path, report) {
        Ok(sidecar) => log::info!("wrote analysis cache {}", sidecar.display()),
        Err(error) => log::warn!("analysis cache not written: {error}"),
    }
}

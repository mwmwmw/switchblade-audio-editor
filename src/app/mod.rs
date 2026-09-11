mod actions;
mod analysis_job;
mod debug_capture;
mod dialogs;
mod format;
mod menu;
mod meters_panel;
mod plugins_panel;
mod region_job;
mod shortcuts;
mod theme;
mod tonal_panel;
mod transport;
mod waveform;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use egui::Ui;

use crate::analysis::anomalies::{AnomalyKind, AnomalySettings};
use crate::audio::encode::{BitDepth, ExportFormat};
use crate::audio::AudioClip;
use crate::document::Document;
use crate::engine::{Engine, EngineEvent};
use actions::PendingLoad;
use analysis_job::AnalysisState;
use debug_capture::DebugCapture;
use dialogs::Dialogs;
use meters_panel::MeterPanel;
use plugins_panel::{PluginAction, PluginPanel};
use region_job::RegionAnalysis;
use tonal_panel::TonalPanel;
use transport::DeviceState;
use waveform::WaveView;

struct AnalysisScope<'a> {
    label: String,
    spectrum: Option<&'a crate::analysis::spectrum::BandSpectrum>,
    loudness: Option<&'a crate::analysis::loudness::LoudnessReport>,
}

/// Offline readouts follow the selection when there is one, otherwise the whole file.
fn analysis_scope<'a>(
    doc: &'a Document,
    analysis: &'a AnalysisState,
    region: &'a RegionAnalysis,
) -> AnalysisScope<'a> {
    if let Some(selection) = &doc.selection {
        let rate = doc.clip.sample_rate;
        let label = format!(
            "selection {} – {}",
            format::time(selection.start, rate),
            format::time(selection.end, rate)
        );
        let report = region.for_selection(doc);
        return AnalysisScope {
            label,
            spectrum: report.map(|r| &r.spectrum),
            loudness: report.and_then(|r| r.loudness.as_ref()),
        };
    }
    let report = analysis.for_version(doc.version);
    AnalysisScope {
        label: "file".into(),
        spectrum: report.map(|r| &r.spectrum),
        loudness: report.and_then(|r| r.loudness.as_ref()),
    }
}

const REPAINT_INTERVAL: Duration = Duration::from_millis(16);
const SIDE_PANEL_WIDTH: f32 = 320.0;
const SIDE_PANEL_MIN_WIDTH: f32 = 260.0;
const SIDE_PANEL_MAX_WIDTH: f32 = 640.0;
const SIDE_PANEL_PADDING: f32 = 10.0;

pub struct SwitchbladeApp {
    pub(super) doc: Document,
    pub(super) engine: Engine,
    pub(super) view: WaveView,
    pub(super) analysis: AnalysisState,
    pub(super) region: RegionAnalysis,
    pub(super) anomaly_settings: AnomalySettings,
    pub(super) dialogs: Dialogs,
    pub(super) plugins: PluginPanel,
    pub(super) tonal: TonalPanel,
    pub(super) meters: MeterPanel,
    pub(super) devices: DeviceState,
    pub(super) clipboard: Option<AudioClip>,
    pub(super) pending_load: PendingLoad,
    pub(super) status: String,
    pub(super) status_is_error: bool,
    pub(super) loop_playback: bool,
    pub(super) snap_to_beats: bool,
    pub(super) show_side_panel: bool,
    pub(super) export_format: ExportFormat,
    pub(super) export_depth: BitDepth,
    pub(super) last_wave_width: f32,
    pub(super) quit_requested: bool,
    launched_at: Option<Instant>,
    debug_capture: Option<DebugCapture>,
}

impl SwitchbladeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);
        let mut app = Self {
            doc: Document::default(),
            engine: Engine::start(),
            view: WaveView::default(),
            analysis: AnalysisState::default(),
            region: RegionAnalysis::default(),
            anomaly_settings: AnomalySettings::default(),
            dialogs: Dialogs::default(),
            plugins: PluginPanel::default(),
            tonal: TonalPanel::default(),
            meters: MeterPanel::default(),
            devices: DeviceState::default(),
            clipboard: None,
            pending_load: None,
            status: "Open a file, drop one here, or press R to record".into(),
            status_is_error: false,
            loop_playback: false,
            snap_to_beats: false,
            show_side_panel: true,
            export_format: ExportFormat::Wav,
            export_depth: BitDepth::Int24,
            last_wave_width: 1.0,
            quit_requested: false,
            launched_at: Some(Instant::now()),
            debug_capture: DebugCapture::from_env(),
        };
        if let Some(path) = initial_file {
            app.open_path(&path);
        }
        app
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        self.poll_load();
        self.analysis.ensure(&self.doc, &self.anomaly_settings, ctx);
        self.analysis.poll();
        self.region.ensure(&self.doc, ctx);
        self.region.poll();
        self.devices.poll();
        self.plugins.poll();
        self.plugins.tick_editors(&self.engine.shared);
        self.tonal.poll();
        for event in self.engine.poll_events() {
            self.handle_engine_event(event);
        }
        self.handle_dropped_files(ctx);
    }

    fn handle_engine_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Recorded(clip) => self.finish_recording(clip),
            EngineEvent::PlaybackFinished => self.set_status("Playback finished"),
            EngineEvent::StreamOpened {
                sample_rate,
                channels,
            } => self.set_status(format!(
                "Playing · device {} · {}",
                format::sample_rate(sample_rate),
                format::channels(channels)
            )),
            EngineEvent::Error(message) => self.set_error(message),
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(path) = dropped {
            self.open_path(&path);
        }
    }

    fn needs_animation(&self) -> bool {
        self.engine.shared.is_playing()
            || self.engine.shared.is_recording()
            || self.analysis.is_pending()
            || self.region.is_pending()
            || self.pending_load.is_some()
    }

    fn side_panel(&mut self, ui: &mut Ui) {
        let mut apply_stack = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let scope = analysis_scope(&self.doc, &self.analysis, &self.region);
                egui::CollapsingHeader::new("Meters")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.meters
                            .show(ui, &self.engine.shared, scope.loudness, &scope.label);
                    });
                egui::CollapsingHeader::new("Tonal balance")
                    .default_open(true)
                    .show(ui, |ui| {
                        self.tonal.show(ui, scope.spectrum, &scope.label);
                    });
                egui::CollapsingHeader::new("Plugins")
                    .default_open(false)
                    .show(ui, |ui| {
                        if let Some(PluginAction::ApplyStack) =
                            self.plugins.show(ui, &self.engine.shared)
                        {
                            apply_stack = true;
                        }
                    });
            });
        if apply_stack {
            self.perform(actions::Action::ApplyStack);
        }
    }

    fn waveform_area(&mut self, ui: &mut Ui) {
        self.last_wave_width = ui.available_width();
        let playing = self.engine.shared.is_playing();
        let play_position =
            playing.then(|| self.engine.shared.play_position.load(Ordering::Relaxed));
        let analysis = self.analysis.for_version(self.doc.version);
        let response = waveform::show(
            ui,
            &mut self.view,
            &mut self.doc,
            analysis,
            play_position,
            self.snap_to_beats,
        );
        if let Some(frame) = response.seek_to {
            self.engine.send(crate::engine::EngineCommand::Seek(frame));
        }
    }

    fn status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let color = if self.status_is_error {
                theme::STATUS_ERROR
            } else {
                ui.visuals().text_color()
            };
            ui.label(egui::RichText::new(&self.status).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.anomaly_summary(ui);
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(self.file_summary())
                        .color(theme::STATUS_MUTED)
                        .monospace(),
                );
            });
        });
    }

    fn file_summary(&self) -> String {
        let clip = &self.doc.clip;
        let mut summary = format!(
            "{} · {} · {} · {}",
            self.doc.title(),
            format::sample_rate(clip.sample_rate),
            format::channels(clip.channel_count()),
            format::time(clip.frames(), clip.sample_rate)
        );
        if let Some(bpm) = self.detected_bpm() {
            summary.push_str(&format!(" · {bpm:.1} BPM"));
        }
        summary
    }

    fn detected_bpm(&self) -> Option<f32> {
        self.analysis.for_version(self.doc.version)?.beats.bpm
    }

    fn anomaly_summary(&self, ui: &mut Ui) {
        let Some(analysis) = self.analysis.for_version(self.doc.version) else {
            ui.label(egui::RichText::new("analysing…").color(theme::STATUS_MUTED));
            return;
        };
        for kind in AnomalyKind::ALL.iter().rev() {
            let count = analysis.anomalies.count(*kind);
            let color = match kind {
                AnomalyKind::Clip => theme::CLIP,
                AnomalyKind::ZeroRun => theme::ZERO_RUN,
                AnomalyKind::Discontinuity => theme::DISCONTINUITY,
            };
            let color = if count == 0 {
                theme::STATUS_MUTED
            } else {
                color
            };
            ui.label(egui::RichText::new(format!("{} {count}", kind.label())).color(color));
        }
    }

    fn dialogs_ui(&mut self, ctx: &egui::Context) {
        if let Some(target) = dialogs::run(&mut self.dialogs.normalize, ctx) {
            self.normalize(target);
        }
        if let Some((curve, direction, scope)) = dialogs::run(&mut self.dialogs.fade, ctx) {
            self.fade(curve, direction, scope);
        }
        if let Some((rate, quality)) = dialogs::run(&mut self.dialogs.resample, ctx) {
            self.resample_to(rate, quality);
        }
        if let Some((format, depth)) = dialogs::run(&mut self.dialogs.export, ctx) {
            self.export(format, depth);
        }
        if dialogs::anomaly_settings_window(
            ctx,
            &mut self.dialogs.anomaly_settings_open,
            &mut self.anomaly_settings,
        ) {
            self.analysis.invalidate();
        }
        dialogs::about_window(ctx, &mut self.dialogs.about_open);
    }
}

impl eframe::App for SwitchbladeApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if let Some(launched_at) = self.launched_at.take() {
            log::info!(
                "first frame {} ms after app creation",
                launched_at.elapsed().as_millis()
            );
        }
        let ctx = ui.ctx().clone();
        self.poll_background(&ctx);
        shortcuts::handle(self, &ctx);
        egui::Panel::top("menu_bar")
            .show_separator_line(false)
            .show(ui, |ui| menu::show(self, ui));
        egui::Panel::top("transport")
            .show_separator_line(false)
            .show(ui, |ui| transport::show(self, ui));
        egui::Panel::bottom("status_bar")
            .show_separator_line(false)
            .show(ui, |ui| self.status_bar(ui));
        if self.show_side_panel {
            egui::Panel::right("side_panel")
                .default_size(SIDE_PANEL_WIDTH)
                .size_range(SIDE_PANEL_MIN_WIDTH..=SIDE_PANEL_MAX_WIDTH)
                .resizable(true)
                .show_separator_line(false)
                .frame(
                    egui::Frame::NONE
                        .fill(theme::BACKGROUND)
                        .inner_margin(SIDE_PANEL_PADDING),
                )
                .show(ui, |ui| self.side_panel(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.waveform_area(ui));
        self.dialogs_ui(&ctx);
        // Published after every frame's input so edits to the loop region reach the audio
        // thread immediately, rather than only at the next Play.
        self.engine.shared.set_loop(self.active_loop_range());
        if self.needs_animation() {
            ctx.request_repaint_after(REPAINT_INTERVAL);
        }
        if self
            .debug_capture
            .as_mut()
            .is_some_and(|capture| capture.tick(&ctx))
        {
            self.quit_requested = true;
        }
        if self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

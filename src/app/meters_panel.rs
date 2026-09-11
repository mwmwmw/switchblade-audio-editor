use std::time::{Duration, Instant};

use egui::{pos2, vec2, Align2, FontId, Rect, Stroke, Ui};

use super::{format, theme};
use crate::analysis::loudness::LoudnessReport;
use crate::analysis::meters::{
    MAX_METER_CHANNELS, VU_MAX_DB, VU_MIN_DB, VU_REFERENCE_PRESETS_DBFS,
};
use crate::audio::db::SILENCE_DB;
use crate::engine::{MeterSnapshot, MeterSource, SharedState};

const PEAK_MIN_DB: f32 = -60.0;
const PEAK_BAR_WIDTH: f32 = 18.0;
const PEAK_BAR_HEIGHT: f32 = 150.0;
const PEAK_BAR_GAP: f32 = 4.0;
const PEAK_SCALE_TICKS: &[f32] = &[0.0, -6.0, -12.0, -18.0, -24.0, -36.0, -48.0, -60.0];
const YELLOW_FROM_DB: f32 = -18.0;
const RED_FROM_DB: f32 = -6.0;
const CLIP_INDICATOR_DB: f32 = -0.1;
const HOLD_DURATION: Duration = Duration::from_millis(1800);
const VU_BAR_HEIGHT: f32 = 14.0;
const VU_TICKS: &[f32] = &[
    -20.0, -10.0, -7.0, -5.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0,
];

pub struct MeterPanel {
    hold_dbfs: [f32; MAX_METER_CHANNELS],
    hold_since: [Instant; MAX_METER_CHANNELS],
    clipped: bool,
}

impl Default for MeterPanel {
    fn default() -> Self {
        Self {
            hold_dbfs: [SILENCE_DB; MAX_METER_CHANNELS],
            hold_since: [Instant::now(); MAX_METER_CHANNELS],
            clipped: false,
        }
    }
}

impl MeterPanel {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        shared: &SharedState,
        offline: Option<&LoudnessReport>,
        scope: &str,
    ) {
        let snapshot = *shared.meters.lock();
        self.update_holds(&snapshot);
        ui.horizontal(|ui| {
            self.paint_peak_bars(ui, &snapshot);
            ui.vertical(|ui| {
                self.clip_indicator(ui);
                loudness_readout(ui, &snapshot, offline, scope);
            });
        });
        ui.add_space(4.0);
        vu_meter(ui, shared, &snapshot);
    }

    fn update_holds(&mut self, snapshot: &MeterSnapshot) {
        let now = Instant::now();
        for channel in 0..snapshot.channel_count.min(MAX_METER_CHANNELS) {
            let peak = snapshot.peak_dbfs[channel];
            if peak >= self.hold_dbfs[channel]
                || now.duration_since(self.hold_since[channel]) > HOLD_DURATION
            {
                self.hold_dbfs[channel] = peak;
                self.hold_since[channel] = now;
            }
            if peak >= CLIP_INDICATOR_DB {
                self.clipped = true;
            }
        }
    }

    fn clip_indicator(&mut self, ui: &mut Ui) {
        let label = if self.clipped { "CLIP" } else { "ok" };
        let color = if self.clipped {
            theme::METER_RED
        } else {
            theme::STATUS_MUTED
        };
        if ui
            .add(egui::Button::new(
                egui::RichText::new(label).color(color).monospace().strong(),
            ))
            .on_hover_text("Click to reset")
            .clicked()
        {
            self.clipped = false;
        }
    }

    fn paint_peak_bars(&self, ui: &mut Ui, snapshot: &MeterSnapshot) {
        let channels = snapshot.channel_count.clamp(1, MAX_METER_CHANNELS);
        let width = channels as f32 * (PEAK_BAR_WIDTH + PEAK_BAR_GAP) + 30.0;
        let (rect, painter) =
            ui.allocate_painter(vec2(width, PEAK_BAR_HEIGHT + 16.0), egui::Sense::hover());
        let rect = rect.rect;
        for channel in 0..channels {
            let left = rect.left() + channel as f32 * (PEAK_BAR_WIDTH + PEAK_BAR_GAP);
            let bar = Rect::from_min_size(
                pos2(left, rect.top()),
                vec2(PEAK_BAR_WIDTH, PEAK_BAR_HEIGHT),
            );
            paint_peak_bar(
                &painter,
                &bar,
                snapshot.peak_dbfs[channel],
                self.hold_dbfs[channel],
            );
            painter.text(
                pos2(bar.center().x, bar.bottom() + 2.0),
                Align2::CENTER_TOP,
                format::db(self.hold_dbfs[channel]),
                FontId::monospace(9.0),
                theme::RULER_TEXT,
            );
        }
        let scale_x = rect.left() + channels as f32 * (PEAK_BAR_WIDTH + PEAK_BAR_GAP) + 2.0;
        for tick in PEAK_SCALE_TICKS {
            let y = db_to_y(*tick, rect.top(), PEAK_BAR_HEIGHT);
            painter.text(
                pos2(scale_x, y),
                Align2::LEFT_CENTER,
                format!("{tick:.0}"),
                FontId::monospace(9.0),
                theme::METER_TICK,
            );
        }
    }
}

fn db_to_y(db: f32, top: f32, height: f32) -> f32 {
    let normalised = ((db - PEAK_MIN_DB) / -PEAK_MIN_DB).clamp(0.0, 1.0);
    top + height * (1.0 - normalised)
}

fn paint_peak_bar(painter: &egui::Painter, bar: &Rect, level_db: f32, hold_db: f32) {
    painter.rect_filled(*bar, 2.0, theme::METER_BACKGROUND);
    let level_y = db_to_y(level_db, bar.top(), bar.height());
    let segments = [
        (PEAK_MIN_DB, YELLOW_FROM_DB, theme::METER_GREEN),
        (YELLOW_FROM_DB, RED_FROM_DB, theme::METER_YELLOW),
        (RED_FROM_DB, 0.0, theme::METER_RED),
    ];
    for (from, to, color) in segments {
        let top = db_to_y(to, bar.top(), bar.height()).max(level_y);
        let bottom = db_to_y(from, bar.top(), bar.height());
        if bottom > top {
            painter.rect_filled(
                Rect::from_min_max(pos2(bar.left(), top), pos2(bar.right(), bottom)),
                0.0,
                color,
            );
        }
    }
    if hold_db > PEAK_MIN_DB {
        let y = db_to_y(hold_db, bar.top(), bar.height());
        painter.hline(bar.x_range(), y, Stroke::new(2.0, theme::METER_HOLD));
    }
}

fn loudness_readout(
    ui: &mut Ui,
    snapshot: &MeterSnapshot,
    offline: Option<&LoudnessReport>,
    scope: &str,
) {
    let live = snapshot.source != MeterSource::Idle;
    let source_label = match snapshot.source {
        MeterSource::Playback => "live · playback",
        MeterSource::Input => "live · input",
        MeterSource::Idle => scope,
    };
    ui.label(
        egui::RichText::new(source_label)
            .small()
            .color(theme::STATUS_MUTED),
    );
    let rows: Vec<(&str, String)> = if live {
        vec![
            ("M", format::lufs(snapshot.lufs_momentary)),
            ("S", format::lufs(snapshot.lufs_short_term)),
            ("I", format::lufs(snapshot.lufs_integrated)),
            ("TP", format::db(snapshot.true_peak_dbtp as f32)),
        ]
    } else if let Some(report) = offline {
        vec![
            ("I", format::lufs(report.integrated_lufs)),
            ("S max", format::lufs(report.short_term_max_lufs)),
            ("M max", format::lufs(report.momentary_max_lufs)),
            ("LRA", format!("{:.1} LU", report.loudness_range_lu)),
            ("TP", format::db(report.true_peak_dbtp as f32)),
            ("Peak", format::db(report.sample_peak_dbfs)),
        ]
    } else {
        vec![("I", "…".into())]
    };
    egui::Grid::new("lufs_grid")
        .spacing([10.0, 2.0])
        .show(ui, |ui| {
            for (name, value) in rows {
                ui.label(egui::RichText::new(name).small().color(theme::STATUS_MUTED));
                ui.label(egui::RichText::new(value).monospace());
                ui.end_row();
            }
        });
    ui.label(
        egui::RichText::new("LUFS / dBTP")
            .small()
            .color(theme::STATUS_MUTED),
    );
}

fn vu_meter(ui: &mut Ui, shared: &SharedState, snapshot: &MeterSnapshot) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("VU").small().color(theme::STATUS_MUTED));
        let mut reference = shared.vu_reference_dbfs();
        egui::ComboBox::from_id_salt("vu_reference")
            .selected_text(format!("0 VU = {reference:.0} dBFS"))
            .show_ui(ui, |ui| {
                for preset in VU_REFERENCE_PRESETS_DBFS {
                    ui.selectable_value(&mut reference, *preset, format!("{preset:.0} dBFS"));
                }
            });
        if reference != shared.vu_reference_dbfs() {
            shared.set_vu_reference_dbfs(reference);
        }
        ui.label(egui::RichText::new(format!("{:+.1}", snapshot.vu)).monospace());
    });
    let (response, painter) = ui.allocate_painter(
        vec2(ui.available_width(), VU_BAR_HEIGHT + 14.0),
        egui::Sense::hover(),
    );
    let rect = response.rect;
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), VU_BAR_HEIGHT));
    let to_x = |vu: f32| bar.left() + bar.width() * (vu - VU_MIN_DB) / (VU_MAX_DB - VU_MIN_DB);
    painter.rect_filled(bar, 2.0, theme::METER_BACKGROUND);
    painter.rect_filled(
        Rect::from_min_max(pos2(to_x(0.0), bar.top()), bar.max),
        0.0,
        theme::CLIP_FILL,
    );
    let level_x = to_x(snapshot.vu.clamp(VU_MIN_DB, VU_MAX_DB));
    let color = if snapshot.vu > 0.0 {
        theme::METER_RED
    } else {
        theme::METER_YELLOW
    };
    painter.rect_filled(
        Rect::from_min_max(bar.min, pos2(level_x, bar.bottom())),
        0.0,
        color,
    );
    for tick in VU_TICKS {
        let x = to_x(*tick);
        painter.vline(
            x,
            bar.bottom()..=bar.bottom() + 3.0,
            Stroke::new(1.0, theme::METER_TICK),
        );
        painter.text(
            pos2(x, bar.bottom() + 4.0),
            Align2::CENTER_TOP,
            format!("{tick:.0}"),
            FontId::monospace(8.0),
            theme::METER_TICK,
        );
    }
}

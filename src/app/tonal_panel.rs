use std::path::PathBuf;

use anyhow::Result;
use crossbeam_channel::Receiver;
use egui::{pos2, vec2, Align2, FontId, Rect, Stroke, Ui};

use super::theme;
use crate::analysis::spectrum::{average_band_spectrum, BandSpectrum, BAND_CENTER_HZ, BAND_COUNT};
use crate::analysis::tonal::{TonalBalance, DEFAULT_PHON, PHON_PRESETS};
use crate::audio::decode;

const CHART_HEIGHT: f32 = 120.0;
const MAX_DEVIATION_DB: f32 = 12.0;
const NEUTRAL_BAND_DB: f32 = 3.0;
const GUIDE_LINES_DB: &[f32] = &[-6.0, 6.0];
const LABELLED_BANDS: &[(usize, &str)] = &[
    (0, "20"),
    (4, "50"),
    (7, "100"),
    (10, "200"),
    (14, "500"),
    (17, "1k"),
    (20, "2k"),
    (24, "5k"),
    (27, "10k"),
];
const REGIONS: &[(&str, usize, usize)] = &[
    ("Low", 0, 11),
    ("Low-mid", 12, 17),
    ("High-mid", 18, 23),
    ("High", 24, 28),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TonalMode {
    EqualLoudness,
    Pink,
    Reference,
}

pub struct TonalPanel {
    mode: TonalMode,
    phon: f32,
    reference: Option<(String, BandSpectrum)>,
    loading: Option<Receiver<Result<(String, BandSpectrum)>>>,
    error: Option<String>,
}

impl Default for TonalPanel {
    fn default() -> Self {
        Self {
            mode: TonalMode::EqualLoudness,
            phon: DEFAULT_PHON,
            reference: None,
            loading: None,
            error: None,
        }
    }
}

impl TonalPanel {
    pub fn poll(&mut self) {
        let Some(result) = self.loading.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.loading = None;
        match result {
            Ok(reference) => {
                self.reference = Some(reference);
                self.mode = TonalMode::Reference;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// `scope` names what the spectrum covers, e.g. "file" or "selection".
    pub fn show(&mut self, ui: &mut Ui, spectrum: Option<&BandSpectrum>, scope: &str) {
        self.controls(ui);
        let Some(spectrum) = spectrum else {
            ui.label(
                egui::RichText::new(format!("analysing {scope}…"))
                    .small()
                    .color(theme::STATUS_MUTED),
            );
            return;
        };
        let balance = match (self.mode, &self.reference) {
            (TonalMode::Reference, Some((_, reference))) => {
                TonalBalance::against_reference(spectrum, reference)
            }
            (TonalMode::Pink, _) => TonalBalance::against_pink(spectrum),
            _ => TonalBalance::against_equal_loudness(spectrum, self.phon),
        };
        paint_chart(ui, &balance);
        region_summary(ui, &balance);
        legend(ui, scope);
        if let Some(error) = &self.error {
            ui.label(
                egui::RichText::new(error)
                    .color(theme::STATUS_ERROR)
                    .small(),
            );
        }
    }

    fn controls(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, TonalMode::EqualLoudness, "To the ear")
                .on_hover_text("Perceived loudness per band after the ISO 226 equal-loudness contour");
            ui.selectable_value(&mut self.mode, TonalMode::Pink, "vs pink")
                .on_hover_text("Physical spectrum against pink noise, the usual stand-in for typical program material");
            ui.add_enabled_ui(self.reference.is_some(), |ui| {
                ui.selectable_value(&mut self.mode, TonalMode::Reference, "vs reference");
            });
        });
        ui.horizontal(|ui| match self.mode {
            TonalMode::EqualLoudness => {
                ui.label(
                    egui::RichText::new("listening level")
                        .small()
                        .color(theme::STATUS_MUTED),
                );
                egui::ComboBox::from_id_salt("phon")
                    .selected_text(format!("{:.0} phon", self.phon))
                    .show_ui(ui, |ui| {
                        for phon in PHON_PRESETS {
                            ui.selectable_value(&mut self.phon, *phon, format!("{phon:.0} phon"));
                        }
                    });
            }
            TonalMode::Pink => {
                ui.label(
                    egui::RichText::new("equal energy per third-octave band")
                        .small()
                        .color(theme::STATUS_MUTED),
                );
            }
            TonalMode::Reference => {
                let name = self
                    .reference
                    .as_ref()
                    .map(|(name, _)| name.as_str())
                    .unwrap_or("none");
                ui.label(
                    egui::RichText::new(format!("vs {name}"))
                        .small()
                        .color(theme::STATUS_MUTED),
                );
            }
        });
        let loading = self.loading.is_some();
        if ui
            .add_enabled(
                !loading,
                egui::Button::new(if loading {
                    "Loading…"
                } else {
                    "Load reference track…"
                })
                .small(),
            )
            .clicked()
        {
            self.pick_reference();
        }
    }

    fn pick_reference(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio", decode::SUPPORTED_EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(load_reference(path));
        });
        self.loading = Some(rx);
    }
}

fn load_reference(path: PathBuf) -> Result<(String, BandSpectrum)> {
    let clip = decode::load(&path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("reference")
        .to_string();
    Ok((name, average_band_spectrum(&clip)))
}

fn paint_chart(ui: &mut Ui, balance: &TonalBalance) {
    let (response, painter) = ui.allocate_painter(
        vec2(ui.available_width(), CHART_HEIGHT + 14.0),
        egui::Sense::hover(),
    );
    let rect = response.rect;
    let chart = Rect::from_min_size(rect.min, vec2(rect.width(), CHART_HEIGHT));
    painter.rect_filled(chart, 2.0, theme::METER_BACKGROUND);
    let to_y = |db: f32| {
        chart.center().y
            - db.clamp(-MAX_DEVIATION_DB, MAX_DEVIATION_DB) / MAX_DEVIATION_DB * chart.height()
                / 2.0
    };
    for guide in GUIDE_LINES_DB {
        painter.hline(
            chart.x_range(),
            to_y(*guide),
            Stroke::new(1.0, theme::GRID_LINE),
        );
    }
    painter.hline(
        chart.x_range(),
        chart.center().y,
        Stroke::new(1.0, theme::CENTER_LINE),
    );
    let slot = chart.width() / BAND_COUNT as f32;
    for (index, deviation) in balance.deviation_db.iter().enumerate() {
        let x0 = chart.left() + index as f32 * slot + 1.0;
        let bar = Rect::from_two_pos(
            pos2(x0, chart.center().y),
            pos2(x0 + slot - 2.0, to_y(*deviation)),
        );
        painter.rect_filled(bar, 1.0, deviation_color(*deviation));
    }
    for (index, label) in LABELLED_BANDS {
        let x = chart.left() + (*index as f32 + 0.5) * slot;
        painter.text(
            pos2(x, chart.bottom() + 2.0),
            Align2::CENTER_TOP,
            *label,
            FontId::monospace(8.0),
            theme::METER_TICK,
        );
    }
    if let Some(pointer) = response.hover_pos() {
        let index = (((pointer.x - chart.left()) / slot) as usize).min(BAND_COUNT - 1);
        response.on_hover_text(format!(
            "{:.0} Hz: {:+.1} dB",
            BAND_CENTER_HZ[index], balance.deviation_db[index]
        ));
    }
}

fn legend(ui: &mut Ui, scope: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(scope)
                .small()
                .color(theme::STATUS_MUTED),
        );
        ui.label(
            egui::RichText::new("■ louder to the ear")
                .small()
                .color(theme::BALANCE_HOT),
        );
        ui.label(
            egui::RichText::new("■ quieter")
                .small()
                .color(theme::BALANCE_COLD),
        );
        ui.label(
            egui::RichText::new(format!("■ within ±{NEUTRAL_BAND_DB:.0} dB of the mean"))
                .small()
                .color(theme::BALANCE_NEUTRAL),
        );
    });
}

fn deviation_color(deviation: f32) -> egui::Color32 {
    if deviation.abs() < NEUTRAL_BAND_DB {
        theme::BALANCE_NEUTRAL
    } else if deviation > 0.0 {
        theme::BALANCE_HOT
    } else {
        theme::BALANCE_COLD
    }
}

fn region_summary(ui: &mut Ui, balance: &TonalBalance) {
    ui.horizontal_wrapped(|ui| {
        for (name, from, to) in REGIONS {
            let slice = &balance.deviation_db[*from..=*to];
            let mean = slice.iter().sum::<f32>() / slice.len() as f32;
            ui.label(
                egui::RichText::new(format!("{name} {mean:+.1}"))
                    .small()
                    .monospace()
                    .color(deviation_color(mean)),
            );
        }
    });
}

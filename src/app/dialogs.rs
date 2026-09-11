use egui::{Align2, Context, Ui};

use crate::analysis::anomalies::AnomalySettings;
use crate::audio::encode::{BitDepth, ExportFormat};
use crate::audio::fade::{FadeCurve, FadeDirection};
use crate::audio::normalize::PEAK_PRESETS_DBFS;
use crate::audio::resample::{ResampleQuality, STANDARD_SAMPLE_RATES};

const MIN_CUSTOM_DBFS: f32 = -60.0;
const MAX_CUSTOM_DBFS: f32 = 0.0;
const MIN_SAMPLE_RATE: u32 = 4000;
const MAX_SAMPLE_RATE: u32 = 384_000;
const DEFAULT_FADE_SECONDS: f64 = 1.0;

pub enum DialogResult<T> {
    Open,
    Cancel,
    Confirm(T),
}

pub trait Dialog {
    type Output;
    const TITLE: &'static str;
    fn body(&mut self, ui: &mut Ui) -> DialogResult<Self::Output>;
}

/// Shows the dialog if present, clearing the slot when it closes. Returns the confirmed value.
pub fn run<D: Dialog>(slot: &mut Option<D>, ctx: &Context) -> Option<D::Output> {
    let dialog = slot.as_mut()?;
    let mut open = true;
    let mut result = DialogResult::Open;
    egui::Window::new(D::TITLE)
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| result = dialog.body(ui));
    match result {
        DialogResult::Open if open => None,
        DialogResult::Open | DialogResult::Cancel => {
            *slot = None;
            None
        }
        DialogResult::Confirm(value) => {
            *slot = None;
            Some(value)
        }
    }
}

fn confirm_row<T>(ui: &mut Ui, label: &str, value: impl FnOnce() -> T) -> DialogResult<T> {
    let mut result = DialogResult::Open;
    ui.horizontal(|ui| {
        if ui.button(label).clicked() {
            result = DialogResult::Confirm(value());
        }
        if ui.button("Cancel").clicked() {
            result = DialogResult::Cancel;
        }
    });
    result
}

#[derive(Default)]
pub struct Dialogs {
    pub normalize: Option<NormalizeDialog>,
    pub fade: Option<FadeDialog>,
    pub resample: Option<ResampleDialog>,
    pub export: Option<ExportDialog>,
    pub anomaly_settings_open: bool,
    pub about_open: bool,
}

pub struct NormalizeDialog {
    pub target_dbfs: f32,
}

impl Default for NormalizeDialog {
    fn default() -> Self {
        Self {
            target_dbfs: PEAK_PRESETS_DBFS[1],
        }
    }
}

impl Dialog for NormalizeDialog {
    type Output = f32;
    const TITLE: &'static str = "Normalize peak";

    fn body(&mut self, ui: &mut Ui) -> DialogResult<f32> {
        ui.label("Target peak level (dBFS)");
        ui.horizontal_wrapped(|ui| {
            for preset in PEAK_PRESETS_DBFS {
                ui.selectable_value(&mut self.target_dbfs, *preset, format!("{preset:+.2}"));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Custom");
            ui.add(
                egui::DragValue::new(&mut self.target_dbfs)
                    .speed(0.1)
                    .range(MIN_CUSTOM_DBFS..=MAX_CUSTOM_DBFS)
                    .suffix(" dBFS"),
            );
        });
        ui.add_space(6.0);
        confirm_row(ui, "Normalize", || self.target_dbfs)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum FadeScope {
    Selection,
    Seconds(f64),
}

pub struct FadeDialog {
    pub curve: FadeCurve,
    pub direction: FadeDirection,
    pub scope: FadeScope,
    has_selection: bool,
}

impl FadeDialog {
    pub fn new(direction: FadeDirection, has_selection: bool) -> Self {
        let scope = if has_selection {
            FadeScope::Selection
        } else {
            FadeScope::Seconds(DEFAULT_FADE_SECONDS)
        };
        Self {
            curve: FadeCurve::EqualPower,
            direction,
            scope,
            has_selection,
        }
    }
}

impl Dialog for FadeDialog {
    type Output = (FadeCurve, FadeDirection, FadeScope);
    const TITLE: &'static str = "Fade";

    fn body(&mut self, ui: &mut Ui) -> DialogResult<Self::Output> {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.direction, FadeDirection::In, "Fade in");
            ui.selectable_value(&mut self.direction, FadeDirection::Out, "Fade out");
        });
        egui::ComboBox::from_label("Curve")
            .selected_text(self.curve.label())
            .show_ui(ui, |ui| {
                for curve in FadeCurve::ALL {
                    ui.selectable_value(&mut self.curve, curve, curve.label());
                }
            });
        ui.add_enabled_ui(self.has_selection, |ui| {
            ui.radio_value(&mut self.scope, FadeScope::Selection, "Over the selection");
        });
        let mut seconds = match self.scope {
            FadeScope::Seconds(s) => s,
            FadeScope::Selection => DEFAULT_FADE_SECONDS,
        };
        ui.horizontal(|ui| {
            let is_seconds = matches!(self.scope, FadeScope::Seconds(_));
            if ui.radio(is_seconds, "Over").clicked() {
                self.scope = FadeScope::Seconds(seconds);
            }
            if ui
                .add(
                    egui::DragValue::new(&mut seconds)
                        .speed(0.05)
                        .range(0.001..=3600.0)
                        .suffix(" s"),
                )
                .changed()
            {
                self.scope = FadeScope::Seconds(seconds);
            }
            ui.label(match self.direction {
                FadeDirection::In => "from the start",
                FadeDirection::Out => "before the end",
            });
        });
        ui.add_space(6.0);
        confirm_row(ui, "Apply fade", || {
            (self.curve, self.direction, self.scope)
        })
    }
}

pub struct ResampleDialog {
    pub target_rate: u32,
    pub quality: ResampleQuality,
    current_rate: u32,
}

impl ResampleDialog {
    pub fn new(current_rate: u32) -> Self {
        let target_rate = if current_rate == 48_000 {
            44_100
        } else {
            48_000
        };
        Self {
            target_rate,
            quality: ResampleQuality::High,
            current_rate,
        }
    }
}

impl Dialog for ResampleDialog {
    type Output = (u32, ResampleQuality);
    const TITLE: &'static str = "Resample";

    fn body(&mut self, ui: &mut Ui) -> DialogResult<Self::Output> {
        ui.label(format!("Current rate: {} Hz", self.current_rate));
        ui.horizontal_wrapped(|ui| {
            for rate in STANDARD_SAMPLE_RATES {
                ui.selectable_value(&mut self.target_rate, *rate, format!("{rate}"));
            }
        });
        ui.horizontal(|ui| {
            ui.label("Custom");
            ui.add(
                egui::DragValue::new(&mut self.target_rate)
                    .speed(10)
                    .range(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE)
                    .suffix(" Hz"),
            );
        });
        egui::ComboBox::from_label("Quality")
            .selected_text(self.quality.label())
            .show_ui(ui, |ui| {
                for quality in ResampleQuality::ALL {
                    ui.selectable_value(&mut self.quality, quality, quality.label());
                }
            });
        ui.add_space(6.0);
        confirm_row(ui, "Resample", || (self.target_rate, self.quality))
    }
}

pub struct ExportDialog {
    pub format: ExportFormat,
    pub depth: BitDepth,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            format: ExportFormat::Wav,
            depth: BitDepth::Int24,
        }
    }
}

impl Dialog for ExportDialog {
    type Output = (ExportFormat, BitDepth);
    const TITLE: &'static str = "Save as";

    fn body(&mut self, ui: &mut Ui) -> DialogResult<Self::Output> {
        egui::ComboBox::from_label("Format")
            .selected_text(self.format.label())
            .show_ui(ui, |ui| {
                for format in ExportFormat::ALL {
                    ui.selectable_value(&mut self.format, format, format.label());
                }
            });
        egui::ComboBox::from_label("Bit depth")
            .selected_text(self.depth.label())
            .show_ui(ui, |ui| {
                for depth in BitDepth::ALL
                    .into_iter()
                    .filter(|d| self.format.supports(*d))
                {
                    ui.selectable_value(&mut self.depth, depth, depth.label());
                }
            });
        if !self.format.supports(self.depth) {
            self.depth = BitDepth::Int24;
        }
        ui.add_space(6.0);
        confirm_row(ui, "Choose file…", || (self.format, self.depth))
    }
}

/// Edits the settings in place; returns true when something changed.
pub fn anomaly_settings_window(
    ctx: &Context,
    open: &mut bool,
    settings: &mut AnomalySettings,
) -> bool {
    let before = *settings;
    egui::Window::new("Highlight settings")
        .collapsible(false)
        .resizable(false)
        .open(open)
        .show(ctx, |ui| {
            egui::Grid::new("anomaly_settings")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Clip threshold");
                    ui.add(
                        egui::DragValue::new(&mut settings.clip_threshold)
                            .speed(0.001)
                            .range(0.5..=1.0)
                            .fixed_decimals(3),
                    );
                    ui.end_row();
                    ui.label("Clip min. consecutive samples");
                    ui.add(egui::DragValue::new(&mut settings.clip_min_run).range(1..=64));
                    ui.end_row();
                    ui.label("Zero run min. samples");
                    ui.add(egui::DragValue::new(&mut settings.zero_min_run).range(2..=100_000));
                    ui.end_row();
                    ui.label("Discontinuity jump");
                    ui.add(
                        egui::DragValue::new(&mut settings.discontinuity_jump)
                            .speed(0.01)
                            .range(0.05..=2.0)
                            .fixed_decimals(2),
                    );
                    ui.end_row();
                });
            if ui.button("Reset to defaults").clicked() {
                *settings = AnomalySettings::default();
            }
        });
    *settings != before
}

pub fn about_window(ctx: &Context, open: &mut bool) {
    egui::Window::new("About Switchblade").collapsible(false).resizable(false).open(open).show(ctx, |ui| {
        ui.label(format!("Switchblade {}", env!("CARGO_PKG_VERSION")));
        ui.label("A lightweight audio editor.");
        ui.label("Wheel or pinch: zoom around the cursor/selection · horizontal scroll or ⇧+wheel: pan · drag: select · double-click: select all");
    });
}

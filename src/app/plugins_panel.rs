use crossbeam_channel::Receiver;
use egui::Ui;

use super::theme;
use crate::engine::SharedState;
use crate::plugins::scan::scan_all;
use crate::plugins::stack::ProcessingStack;
use crate::plugins::{ParamInfo, PluginDescriptor, PluginFormat};

const PLUGIN_LIST_HEIGHT: f32 = 140.0;

pub enum PluginAction {
    ApplyStack,
}

#[derive(Default)]
pub struct PluginPanel {
    found: Vec<PluginDescriptor>,
    scan_rx: Option<Receiver<Vec<PluginDescriptor>>>,
    filter: String,
    error: Option<String>,
    expanded_slot: Option<usize>,
    pending_move: Option<(usize, usize)>,
    pending_remove: Option<usize>,
}

impl PluginPanel {
    pub fn poll(&mut self) {
        if let Some(found) = self.scan_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.found = found;
            self.scan_rx = None;
        }
    }

    pub fn show(&mut self, ui: &mut Ui, shared: &SharedState) -> Option<PluginAction> {
        self.browser(ui, shared);
        ui.add_space(8.0);
        let action = self.stack_ui(ui, shared);
        if let Some(error) = &self.error {
            ui.label(
                egui::RichText::new(error)
                    .color(theme::STATUS_ERROR)
                    .small(),
            );
        }
        action
    }

    fn start_scan(&mut self) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send(scan_all());
        });
        self.scan_rx = Some(rx);
    }

    fn browser(&mut self, ui: &mut Ui, shared: &SharedState) {
        ui.horizontal(|ui| {
            let scanning = self.scan_rx.is_some();
            if ui
                .add_enabled(
                    !scanning,
                    egui::Button::new(if scanning {
                        "Scanning…"
                    } else {
                        "Scan plugins"
                    }),
                )
                .clicked()
            {
                self.start_scan();
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("filter")
                    .desired_width(110.0),
            );
        });
        if self.found.is_empty() && self.scan_rx.is_none() {
            ui.label(
                egui::RichText::new(
                    "No plugins scanned yet. CLAP and VST2 load; VST3 is listed only.",
                )
                .small()
                .color(theme::STATUS_MUTED),
            );
            return;
        }
        let filter = self.filter.to_lowercase();
        let mut to_add: Option<PluginDescriptor> = None;
        egui::ScrollArea::vertical()
            .id_salt("plugin_browser")
            .max_height(PLUGIN_LIST_HEIGHT)
            .show(ui, |ui| {
                for plugin in self
                    .found
                    .iter()
                    .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter))
                {
                    ui.horizontal(|ui| {
                        if ui.small_button("+").on_hover_text("Add to stack").clicked() {
                            to_add = Some(plugin.clone());
                        }
                        ui.label(format_badge(plugin.format));
                        ui.label(&plugin.name);
                        if !plugin.vendor.is_empty() {
                            ui.label(
                                egui::RichText::new(&plugin.vendor)
                                    .small()
                                    .color(theme::STATUS_MUTED),
                            );
                        }
                    });
                }
            });
        if let Some(plugin) = to_add {
            self.error = shared
                .stack
                .lock()
                .add(&plugin)
                .err()
                .map(|e| e.to_string());
        }
    }

    fn stack_ui(&mut self, ui: &mut Ui, shared: &SharedState) -> Option<PluginAction> {
        let mut stack = shared.stack.lock();
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Stack ({})", stack.slots().len())).strong());
            if ui
                .add_enabled(
                    stack.has_active_plugins(),
                    egui::Button::new("Apply to audio"),
                )
                .on_hover_text("Render the stack into the file (undoable)")
                .clicked()
            {
                action = Some(PluginAction::ApplyStack);
            }
        });
        if stack.is_empty() {
            ui.label(
                egui::RichText::new("Empty — plugins here run live during playback.")
                    .small()
                    .color(theme::STATUS_MUTED),
            );
            return action;
        }
        let count = stack.slots().len();
        for index in 0..count {
            self.slot_row(ui, &mut stack, index, count);
        }
        if let Some((from, to)) = self.pending_move.take() {
            stack.move_slot(from, to);
        }
        if let Some(index) = self.pending_remove.take() {
            stack.remove(index);
            self.expanded_slot = None;
        }
        action
    }

    fn slot_row(&mut self, ui: &mut Ui, stack: &mut ProcessingStack, index: usize, count: usize) {
        let slot = &mut stack.slots_mut()[index];
        let name = slot.instance.descriptor().name.clone();
        ui.horizontal(|ui| {
            let mut enabled = !slot.bypassed;
            if ui
                .checkbox(&mut enabled, "")
                .on_hover_text("Enable")
                .changed()
            {
                slot.bypassed = !enabled;
            }
            let expanded = self.expanded_slot == Some(index);
            if ui.selectable_label(expanded, &name).clicked() {
                self.expanded_slot = if expanded { None } else { Some(index) };
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("✕").clicked() {
                    self.pending_remove = Some(index);
                }
                if ui
                    .add_enabled(index + 1 < count, egui::Button::new("▼").small())
                    .clicked()
                {
                    self.pending_move = Some((index, index + 1));
                }
                if ui
                    .add_enabled(index > 0, egui::Button::new("▲").small())
                    .clicked()
                {
                    self.pending_move = Some((index, index - 1));
                }
            });
        });
        if self.expanded_slot == Some(index) {
            param_editor(ui, slot);
        }
    }
}

fn param_editor(ui: &mut Ui, slot: &mut crate::plugins::stack::PluginSlot) {
    let params: Vec<ParamInfo> = slot.instance.params().to_vec();
    if params.is_empty() {
        ui.label(
            egui::RichText::new("No parameters exposed.")
                .small()
                .color(theme::STATUS_MUTED),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt(("params", slot.instance.descriptor().path.clone()))
        .max_height(220.0)
        .show(ui, |ui| {
            for param in &params {
                let mut value = slot.instance.param_value(param.id);
                let text = slot.instance.param_text(param.id, value);
                ui.horizontal(|ui| {
                    let slider = egui::Slider::new(&mut value, param.min..=param.max)
                        .text(&param.name)
                        .show_value(false);
                    let slider = if param.stepped {
                        slider.step_by(1.0)
                    } else {
                        slider
                    };
                    if ui.add(slider).changed() {
                        slot.instance.set_param(param.id, value);
                    }
                    if ui
                        .small_button("↺")
                        .on_hover_text("Reset to default")
                        .clicked()
                    {
                        slot.instance.set_param(param.id, param.default);
                    }
                    ui.label(egui::RichText::new(text).small().monospace());
                });
            }
        });
}

fn format_badge(format: PluginFormat) -> egui::RichText {
    egui::RichText::new(format.label())
        .small()
        .monospace()
        .color(theme::STATUS_MUTED)
}

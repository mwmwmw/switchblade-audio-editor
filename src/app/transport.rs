use crossbeam_channel::Receiver;
use egui::Ui;

use super::actions::Action;
use super::{format, theme, SwitchbladeApp};
use crate::engine::{devices, DeviceCatalog, DeviceSelection, EngineCommand};

#[derive(Default)]
pub struct DeviceState {
    pub catalog: Option<DeviceCatalog>,
    pub selection: DeviceSelection,
    pending: Option<Receiver<DeviceCatalog>>,
    started: bool,
}

impl DeviceState {
    /// Device enumeration is deferred to a background thread so startup stays instant.
    pub fn poll(&mut self) {
        if !self.started {
            self.started = true;
            let (tx, rx) = crossbeam_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send(devices::enumerate());
            });
            self.pending = Some(rx);
        }
        if let Some(catalog) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.catalog = Some(catalog);
            self.pending = None;
        }
    }
}

pub fn show(app: &mut SwitchbladeApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        transport_buttons(app, ui);
        ui.add_space(12.0);
        time_readout(app, ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            device_pickers(app, ui)
        });
    });
}

fn transport_buttons(app: &mut SwitchbladeApp, ui: &mut Ui) {
    let playing = app.engine.shared.is_playing();
    let recording = app.engine.shared.is_recording();
    let play_label = if playing { "⏸ Pause" } else { "▶ Play" };
    if ui.button(play_label).on_hover_text("Space").clicked() {
        app.perform(Action::TogglePlay);
    }
    if ui
        .add_enabled(playing || recording, egui::Button::new("⏹ Stop"))
        .clicked()
    {
        app.perform(Action::Stop);
    }
    let record = egui::Button::new(egui::RichText::new("⏺ Rec").color(if recording {
        theme::METER_RED
    } else {
        ui.visuals().text_color()
    }));
    if ui.add(record).on_hover_text("R").clicked() {
        app.perform(Action::ToggleRecord);
    }
    if ui
        .selectable_label(app.loop_playback, "🔁 Loop")
        .on_hover_text("L · loop the selection")
        .clicked()
    {
        app.perform(Action::ToggleLoop);
    }
}

fn time_readout(app: &SwitchbladeApp, ui: &mut Ui) {
    let rate = app.doc.clip.sample_rate;
    let position = if app.engine.shared.is_playing() {
        app.engine
            .shared
            .play_position
            .load(std::sync::atomic::Ordering::Relaxed)
    } else {
        app.doc.cursor
    };
    let text = format!(
        "{} / {}",
        format::time(position, rate),
        format::time(app.doc.clip.frames(), rate)
    );
    ui.label(egui::RichText::new(text).monospace().size(15.0));
    if let Some(selection) = &app.doc.selection {
        let text = format!(
            "sel {} – {}  ({})",
            format::time(selection.start, rate),
            format::time(selection.end, rate),
            format::time(selection.len(), rate)
        );
        ui.label(
            egui::RichText::new(text)
                .monospace()
                .color(theme::STATUS_MUTED),
        );
    }
}

fn device_pickers(app: &mut SwitchbladeApp, ui: &mut Ui) {
    let Some(catalog) = app.devices.catalog.clone() else {
        ui.label(egui::RichText::new("scanning audio devices…").color(theme::STATUS_MUTED));
        return;
    };
    let before = app.devices.selection.clone();
    let selection = &mut app.devices.selection;
    let host_id = selection
        .host
        .or_else(|| catalog.hosts.first().map(|h| h.id));
    let host = host_id.and_then(|id| catalog.host(id));
    if let Some(host) = host {
        device_combo(ui, "input_device", "In", &host.inputs, &mut selection.input);
        device_combo(
            ui,
            "output_device",
            "Out",
            &host.outputs,
            &mut selection.output,
        );
    }
    if catalog.hosts.len() > 1 {
        host_combo(ui, &catalog, selection);
    }
    if app.devices.selection != before {
        app.engine
            .send(EngineCommand::SetDevices(app.devices.selection.clone()));
    }
}

fn host_combo(ui: &mut Ui, catalog: &DeviceCatalog, selection: &mut DeviceSelection) {
    let current = selection
        .host
        .and_then(|id| catalog.host(id))
        .or(catalog.hosts.first())
        .map(|h| h.name.clone())
        .unwrap_or_default();
    egui::ComboBox::from_id_salt("audio_host")
        .selected_text(current)
        .show_ui(ui, |ui| {
            for host in &catalog.hosts {
                if ui
                    .selectable_value(&mut selection.host, Some(host.id), &host.name)
                    .changed()
                {
                    selection.input = None;
                    selection.output = None;
                }
            }
        });
    ui.label(egui::RichText::new("Driver").color(theme::STATUS_MUTED));
}

fn device_combo(
    ui: &mut Ui,
    id: &str,
    label: &str,
    names: &[String],
    selected: &mut Option<String>,
) {
    let current = selected
        .clone()
        .unwrap_or_else(|| "System default".to_string());
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(150.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, None, "System default");
            for name in names {
                ui.selectable_value(selected, Some(name.clone()), name);
            }
        });
    ui.label(egui::RichText::new(label).color(theme::STATUS_MUTED));
}

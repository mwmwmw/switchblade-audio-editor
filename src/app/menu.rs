use egui::Ui;

use super::actions::Action;
use super::shortcuts::shortcut_for;
use super::SwitchbladeApp;

pub fn show(app: &mut SwitchbladeApp, ui: &mut Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(app, ui, "Open…", Action::Open, true);
            item(app, ui, "Save", Action::Save, !app.doc.clip.is_empty());
            item(
                app,
                ui,
                "Save as…",
                Action::SaveAs,
                !app.doc.clip.is_empty(),
            );
            ui.separator();
            item(app, ui, "Quit", Action::Quit, true);
        });
        ui.menu_button("Edit", |ui| {
            let undo = app
                .doc
                .undo_label()
                .map(|l| format!("Undo {l}"))
                .unwrap_or_else(|| "Undo".into());
            let redo = app
                .doc
                .redo_label()
                .map(|l| format!("Redo {l}"))
                .unwrap_or_else(|| "Redo".into());
            item(app, ui, &undo, Action::Undo, app.doc.can_undo());
            item(app, ui, &redo, Action::Redo, app.doc.can_redo());
            ui.separator();
            let has_selection = app.doc.has_selection();
            item(app, ui, "Cut", Action::Cut, has_selection);
            item(app, ui, "Copy", Action::Copy, has_selection);
            item(app, ui, "Paste", Action::Paste, app.clipboard.is_some());
            item(app, ui, "Delete", Action::Delete, has_selection);
            item(app, ui, "Trim to selection", Action::Trim, has_selection);
            item(app, ui, "Silence selection", Action::Silence, has_selection);
            ui.separator();
            item(app, ui, "Select all", Action::SelectAll, true);
        });
        ui.menu_button("Process", |ui| {
            let has_audio = !app.doc.clip.is_empty();
            item(app, ui, "Normalize…", Action::Normalize, has_audio);
            item(app, ui, "Fade in…", Action::FadeIn, has_audio);
            item(app, ui, "Fade out…", Action::FadeOut, has_audio);
            item(app, ui, "Resample…", Action::Resample, has_audio);
            ui.separator();
            item(app, ui, "Remove DC offset", Action::RemoveDc, has_audio);
            item(
                app,
                ui,
                "Repair discontinuities",
                Action::RepairDiscontinuities,
                has_audio,
            );
            item(
                app,
                ui,
                "Crossfade loop",
                Action::CrossfadeLoop,
                app.doc.has_selection(),
            );
            ui.separator();
            let stack_ready = app.engine.shared.stack.lock().has_active_plugins();
            item(
                app,
                ui,
                "Apply plugin stack",
                Action::ApplyStack,
                has_audio && stack_ready,
            );
        });
        ui.menu_button("View", |ui| {
            item(app, ui, "Zoom to fit", Action::ZoomFit, true);
            item(
                app,
                ui,
                "Zoom to selection",
                Action::ZoomSelection,
                app.doc.has_selection(),
            );
            ui.separator();
            let snap_label = if app.snap_to_beats {
                "✓ Snap to beats"
            } else {
                "Snap to beats"
            };
            item(app, ui, snap_label, Action::ToggleBeatSnap, true);
            ui.separator();
            item(
                app,
                ui,
                "Highlight settings…",
                Action::HighlightSettings,
                true,
            );
            item(app, ui, "Toggle side panel", Action::ToggleSidePanel, true);
            ui.separator();
            item(app, ui, "About", Action::About, true);
        });
    });
}

fn item(app: &mut SwitchbladeApp, ui: &mut Ui, label: &str, action: Action, enabled: bool) {
    let mut button = egui::Button::new(label);
    if let Some(shortcut) = shortcut_for(action) {
        button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
    }
    if ui.add_enabled(enabled, button).clicked() {
        app.perform(action);
        ui.close();
    }
}

use egui::{Context, Key, KeyboardShortcut, Modifiers};

use super::actions::Action;
use super::SwitchbladeApp;

pub const SHORTCUTS: &[(Modifiers, Key, Action)] = &[
    (Modifiers::COMMAND, Key::O, Action::Open),
    (Modifiers::COMMAND, Key::S, Action::Save),
    (
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
        Key::S,
        Action::SaveAs,
    ),
    (
        Modifiers::COMMAND.plus(Modifiers::SHIFT),
        Key::Z,
        Action::Redo,
    ),
    (Modifiers::COMMAND, Key::Z, Action::Undo),
    (Modifiers::COMMAND, Key::X, Action::Cut),
    (Modifiers::COMMAND, Key::C, Action::Copy),
    (Modifiers::COMMAND, Key::V, Action::Paste),
    (Modifiers::COMMAND, Key::A, Action::SelectAll),
    (Modifiers::COMMAND, Key::T, Action::Trim),
    (Modifiers::COMMAND, Key::N, Action::Normalize),
    (Modifiers::COMMAND, Key::E, Action::ZoomSelection),
    (Modifiers::COMMAND, Key::Num0, Action::ZoomFit),
    (Modifiers::COMMAND, Key::B, Action::ToggleSidePanel),
    (Modifiers::COMMAND, Key::Q, Action::Quit),
    (Modifiers::NONE, Key::Delete, Action::Delete),
    (Modifiers::NONE, Key::Backspace, Action::Delete),
    (Modifiers::NONE, Key::Space, Action::TogglePlay),
    (Modifiers::NONE, Key::Escape, Action::Stop),
    (Modifiers::NONE, Key::R, Action::ToggleRecord),
    (Modifiers::NONE, Key::L, Action::ToggleLoop),
    (Modifiers::NONE, Key::Home, Action::GoToStart),
    (Modifiers::NONE, Key::End, Action::GoToEnd),
];

pub fn shortcut_for(action: Action) -> Option<KeyboardShortcut> {
    SHORTCUTS
        .iter()
        .find(|(_, _, a)| *a == action)
        .map(|(m, k, _)| KeyboardShortcut::new(*m, *k))
}

pub fn handle(app: &mut SwitchbladeApp, ctx: &Context) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    let fired: Vec<Action> = ctx.input_mut(|input| {
        SHORTCUTS
            .iter()
            .filter(|(modifiers, key, _)| {
                input.consume_shortcut(&KeyboardShortcut::new(*modifiers, *key))
            })
            .map(|(_, _, action)| *action)
            .collect()
    });
    for action in fired {
        app.perform(action);
    }
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod app;
mod audio;
mod cache;
mod document;
mod engine;
mod plugins;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

const APP_NAME: &str = "Switchblade";
const INITIAL_WINDOW_SIZE: [f32; 2] = [1240.0, 720.0];
const MIN_WINDOW_SIZE: [f32; 2] = [760.0, 440.0];

/// A GUI-subsystem binary starts with no console, so warnings would go nowhere when the
/// app is launched from a terminal. Reattaching to the parent's console restores them;
/// it fails harmlessly when there is no console to attach to (launched from Explorer).
#[cfg(windows)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn main() -> eframe::Result {
    attach_parent_console();
    // Plugin loads, stream errors and cache misses are all logged at warn; show them by default.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let initial_file = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_NAME)
            .with_inner_size(INITIAL_WINDOW_SIZE)
            .with_min_inner_size(MIN_WINDOW_SIZE),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::SwitchbladeApp::new(cc, initial_file)))),
    )
}

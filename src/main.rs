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

fn main() -> eframe::Result {
    env_logger::init();
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

//! Developer aid: `SWITCHBLADE_SCREENSHOT=/path/shot.ppm` saves a frame of the window and quits.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{ColorImage, Context, Event, UserData, ViewportCommand};

const ENV_VAR: &str = "SWITCHBLADE_SCREENSHOT";
/// Frames to let the UI settle (device scan, analysis) before capturing.
const CAPTURE_AFTER_FRAMES: u32 = 45;

pub struct DebugCapture {
    target: PathBuf,
    frames: u32,
    requested: bool,
}

impl DebugCapture {
    pub fn from_env() -> Option<Self> {
        std::env::var_os(ENV_VAR).map(|path| Self {
            target: PathBuf::from(path),
            frames: 0,
            requested: false,
        })
    }

    /// Returns true once the capture has been written and the app should close.
    pub fn tick(&mut self, ctx: &Context) -> bool {
        self.frames += 1;
        ctx.request_repaint();
        if self.frames >= CAPTURE_AFTER_FRAMES && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
        }
        let image = ctx.input(|i| i.events.iter().find_map(screenshot_event));
        match image {
            Some(image) => {
                if let Err(error) = write_ppm(&self.target, &image) {
                    log::error!("screenshot failed: {error}");
                }
                true
            }
            None => false,
        }
    }
}

fn screenshot_event(event: &Event) -> Option<Arc<ColorImage>> {
    match event {
        Event::Screenshot { image, .. } => Some(Arc::clone(image)),
        _ => None,
    }
}

fn write_ppm(path: &PathBuf, image: &ColorImage) -> std::io::Result<()> {
    let [width, height] = image.size;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(file, "P6\n{width} {height}\n255\n")?;
    for pixel in &image.pixels {
        file.write_all(&pixel.to_array()[..3])?;
    }
    file.flush()
}

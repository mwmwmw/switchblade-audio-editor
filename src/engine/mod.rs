pub mod devices;
mod playback;
mod record;
mod shared;
mod worker;

use std::ops::Range;
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};

use crate::audio::AudioClip;
pub use devices::{DeviceCatalog, DeviceSelection};
pub use shared::{MeterSnapshot, MeterSource, SharedState};

pub enum EngineCommand {
    Play {
        clip: Arc<AudioClip>,
        start_frame: usize,
        loop_range: Option<Range<usize>>,
    },
    Stop,
    Seek(usize),
    StartRecording,
    StopRecording,
    SetDevices(DeviceSelection),
    Shutdown,
}

pub enum EngineEvent {
    Recorded(AudioClip),
    PlaybackFinished,
    StreamOpened { sample_rate: u32, channels: usize },
    Error(String),
}

/// Handle owned by the UI; the audio streams live on a dedicated worker thread.
pub struct Engine {
    commands: Sender<EngineCommand>,
    events: Receiver<EngineEvent>,
    pub shared: Arc<SharedState>,
    worker: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn start() -> Self {
        let shared = Arc::new(SharedState::default());
        let (commands, command_rx) = crossbeam_channel::unbounded();
        let (event_tx, events) = crossbeam_channel::unbounded();
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("switchblade-audio".into())
            .spawn(move || worker::run(command_rx, event_tx, worker_shared))
            .expect("spawning audio worker");
        Self {
            commands,
            events,
            shared,
            worker: Some(worker),
        }
    }

    pub fn send(&self, command: EngineCommand) {
        let _ = self.commands.send(command);
    }

    pub fn poll_events(&self) -> Vec<EngineEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.commands.send(EngineCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

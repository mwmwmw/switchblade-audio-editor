use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::{never, Receiver, Sender};

use super::devices::{resolve_host, resolve_input, resolve_output, DeviceSelection};
use super::playback::{self, PlaybackRequest};
use super::record::{self, Recorder};
use super::shared::SharedState;
use super::{EngineCommand, EngineEvent};

pub fn run(
    commands: Receiver<EngineCommand>,
    events: Sender<EngineEvent>,
    shared: Arc<SharedState>,
) {
    let mut worker = Worker {
        shared,
        events,
        selection: DeviceSelection::default(),
        output: None,
        input: None,
    };
    loop {
        let chunks = worker
            .input
            .as_ref()
            .map_or_else(never, |input| input.chunks.clone());
        crossbeam_channel::select! {
            recv(commands) -> command => match command {
                Ok(EngineCommand::Shutdown) | Err(_) => break,
                Ok(command) => worker.handle(command),
            },
            recv(chunks) -> chunk => if let (Ok(chunk), Some(input)) = (chunk, worker.input.as_mut()) {
                input.recorder.push(chunk);
            },
        }
    }
    worker.stop_playback();
    worker.input.take();
}

struct Worker {
    shared: Arc<SharedState>,
    events: Sender<EngineEvent>,
    selection: DeviceSelection,
    output: Option<cpal::Stream>,
    input: Option<ActiveInput>,
}

struct ActiveInput {
    _stream: cpal::Stream,
    recorder: Recorder,
    chunks: Receiver<Vec<f32>>,
}

impl Worker {
    fn handle(&mut self, command: EngineCommand) {
        match command {
            EngineCommand::Play {
                clip,
                start_frame,
                loop_range,
            } => self.start_playback(PlaybackRequest {
                clip,
                start_frame,
                loop_range,
            }),
            EngineCommand::Stop => self.stop_playback(),
            EngineCommand::Seek(frame) => self.shared.request_seek(frame),
            EngineCommand::StartRecording => self.start_recording(),
            EngineCommand::StopRecording => self.stop_recording(),
            EngineCommand::SetDevices(selection) => self.selection = selection,
            EngineCommand::Shutdown => {}
        }
    }

    fn report(&self, message: String) {
        let _ = self.events.send(EngineEvent::Error(message));
    }

    fn start_playback(&mut self, request: PlaybackRequest) {
        self.stop_playback();
        let host = resolve_host(&self.selection);
        let Some(device) = resolve_output(&host, &self.selection) else {
            return self.report("no output device available".into());
        };
        match playback::open(
            &device,
            request,
            Arc::clone(&self.shared),
            self.events.clone(),
        ) {
            Ok(opened) => {
                let _ = self.events.send(EngineEvent::StreamOpened {
                    sample_rate: opened.sample_rate,
                    channels: opened.channels,
                });
                self.output = Some(opened.stream);
            }
            Err(error) => self.report(error.to_string()),
        }
    }

    fn stop_playback(&mut self) {
        self.output.take();
        self.shared.playing.store(false, Ordering::Relaxed);
        self.shared.stack.lock().deactivate();
        if !self.shared.is_recording() {
            self.shared.clear_meters();
        }
    }

    fn start_recording(&mut self) {
        if self.input.is_some() {
            return;
        }
        let host = resolve_host(&self.selection);
        let Some(device) = resolve_input(&host, &self.selection) else {
            return self.report("no input device available".into());
        };
        match record::open(&device, Arc::clone(&self.shared), self.events.clone()) {
            Ok(opened) => {
                self.input = Some(ActiveInput {
                    _stream: opened.stream,
                    recorder: opened.recorder,
                    chunks: opened.chunks,
                })
            }
            Err(error) => self.report(error.to_string()),
        }
    }

    fn stop_recording(&mut self) {
        let Some(input) = self.input.take() else {
            return;
        };
        let ActiveInput {
            _stream,
            mut recorder,
            chunks,
        } = input;
        drop(_stream);
        for chunk in chunks.try_iter() {
            recorder.push(chunk);
        }
        self.shared.recording.store(false, Ordering::Relaxed);
        self.shared.clear_meters();
        let _ = self.events.send(EngineEvent::Recorded(recorder.finish()));
    }
}

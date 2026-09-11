use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::Sender;

use super::shared::{MeterChain, MeterSource, SharedState};
use super::EngineEvent;
use crate::audio::AudioClip;

const CHUNK_QUEUE_CAPACITY: usize = 512;

pub struct OpenedInput {
    pub stream: cpal::Stream,
    pub recorder: Recorder,
    pub chunks: crossbeam_channel::Receiver<Vec<f32>>,
}

/// Accumulates interleaved input chunks off the audio thread.
pub struct Recorder {
    sample_rate: u32,
    channels: usize,
    interleaved: Vec<f32>,
}

impl Recorder {
    pub fn push(&mut self, chunk: Vec<f32>) {
        self.interleaved.extend_from_slice(&chunk);
    }

    pub fn finish(self) -> AudioClip {
        AudioClip::from_interleaved(self.sample_rate, self.channels, &self.interleaved)
    }
}

pub fn open(
    device: &cpal::Device,
    shared: Arc<SharedState>,
    events: Sender<EngineEvent>,
) -> Result<OpenedInput> {
    let default = device
        .default_input_config()
        .map_err(|e| anyhow!("input config: {e}"))?;
    let sample_rate = default.sample_rate();
    let channels = default.channels() as usize;
    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let (chunk_tx, chunks) = crossbeam_channel::bounded(CHUNK_QUEUE_CAPACITY);
    let mut state = InputState {
        channels,
        planes: vec![Vec::new(); channels],
        meters: MeterChain::new(MeterSource::Input, channels, sample_rate),
        shared: Arc::clone(&shared),
        chunk_tx,
    };
    let stream = device
        .build_input_stream(
            config,
            move |data: &[f32], _| state.capture(data),
            move |error| {
                let _ = events.send(EngineEvent::Error(format!("input stream: {error}")));
            },
            None,
        )
        .map_err(|e| anyhow!("building input stream: {e}"))?;
    stream
        .play()
        .map_err(|e| anyhow!("starting input stream: {e}"))?;
    shared.recording.store(true, Ordering::Relaxed);
    Ok(OpenedInput {
        stream,
        recorder: Recorder {
            sample_rate,
            channels,
            interleaved: Vec::new(),
        },
        chunks,
    })
}

struct InputState {
    channels: usize,
    planes: Vec<Vec<f32>>,
    meters: MeterChain,
    shared: Arc<SharedState>,
    chunk_tx: Sender<Vec<f32>>,
}

impl InputState {
    fn capture(&mut self, data: &[f32]) {
        let _ = self.chunk_tx.try_send(data.to_vec());
        self.deinterleave(data);
        let planes: Vec<&[f32]> = self.planes.iter().map(Vec::as_slice).collect();
        let snapshot = self
            .meters
            .measure(&planes, self.shared.vu_reference_dbfs());
        self.shared.publish_meters(snapshot);
    }

    fn deinterleave(&mut self, data: &[f32]) {
        let frames = data.len() / self.channels.max(1);
        for (channel, plane) in self.planes.iter_mut().enumerate() {
            plane.clear();
            plane.extend((0..frames).map(|frame| data[frame * self.channels + channel]));
        }
    }
}

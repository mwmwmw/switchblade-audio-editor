use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
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
    let state = InputState {
        channels,
        planes: vec![Vec::new(); channels],
        meters: MeterChain::new(MeterSource::Input, channels, sample_rate),
        shared: Arc::clone(&shared),
        chunk_tx,
    };
    let stream = build_stream(device, &config, default.sample_format(), state, events)?;
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

/// The device dictates the sample type; captured frames are converted to f32 on arrival.
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    state: InputState,
    events: Sender<EngineEvent>,
) -> Result<cpal::Stream> {
    match format {
        cpal::SampleFormat::I8 => build_typed::<i8>(device, config, state, events),
        cpal::SampleFormat::I16 => build_typed::<i16>(device, config, state, events),
        cpal::SampleFormat::I32 => build_typed::<i32>(device, config, state, events),
        cpal::SampleFormat::I64 => build_typed::<i64>(device, config, state, events),
        cpal::SampleFormat::U8 => build_typed::<u8>(device, config, state, events),
        cpal::SampleFormat::U16 => build_typed::<u16>(device, config, state, events),
        cpal::SampleFormat::U32 => build_typed::<u32>(device, config, state, events),
        cpal::SampleFormat::U64 => build_typed::<u64>(device, config, state, events),
        cpal::SampleFormat::F32 => build_typed::<f32>(device, config, state, events),
        cpal::SampleFormat::F64 => build_typed::<f64>(device, config, state, events),
        other => Err(anyhow!("unsupported input sample format: {other}")),
    }
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: InputState,
    events: Sender<EngineEvent>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            config.clone(),
            move |data: &[T], _| state.capture(data),
            move |error| {
                let _ = events.send(EngineEvent::Error(format!("input stream: {error}")));
            },
            None,
        )
        .map_err(|e| anyhow!("building input stream: {e}"))
}

struct InputState {
    channels: usize,
    planes: Vec<Vec<f32>>,
    meters: MeterChain,
    shared: Arc<SharedState>,
    chunk_tx: Sender<Vec<f32>>,
}

impl InputState {
    fn capture<T: Copy>(&mut self, data: &[T])
    where
        f32: FromSample<T>,
    {
        let samples: Vec<f32> = data.iter().map(|&s| f32::from_sample(s)).collect();
        self.deinterleave(&samples);
        let planes: Vec<&[f32]> = self.planes.iter().map(Vec::as_slice).collect();
        let snapshot = self
            .meters
            .measure(&planes, self.shared.vu_reference_dbfs());
        self.shared.publish_meters(snapshot);
        let _ = self.chunk_tx.try_send(samples);
    }

    fn deinterleave(&mut self, data: &[f32]) {
        let frames = data.len() / self.channels.max(1);
        for (channel, plane) in self.planes.iter_mut().enumerate() {
            plane.clear();
            plane.extend((0..frames).map(|frame| data[frame * self.channels + channel]));
        }
    }
}

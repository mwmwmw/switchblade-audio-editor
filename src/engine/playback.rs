use std::ops::Range;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use crossbeam_channel::Sender;

use super::shared::{MeterChain, MeterSource, SharedState, NO_SEEK};
use super::EngineEvent;
use crate::audio::resample::{resample, ResampleQuality};
use crate::audio::AudioClip;
use crate::plugins::stack::DEFAULT_BLOCK_FRAMES;

pub struct PlaybackRequest {
    pub clip: Arc<AudioClip>,
    pub start_frame: usize,
    pub loop_range: Option<Range<usize>>,
}

pub struct OpenedStream {
    pub stream: cpal::Stream,
    pub sample_rate: u32,
    pub channels: usize,
}

pub fn open(
    device: &cpal::Device,
    request: PlaybackRequest,
    shared: Arc<SharedState>,
    events: Sender<EngineEvent>,
) -> Result<OpenedStream> {
    let default = device
        .default_output_config()
        .map_err(|e| anyhow!("output config: {e}"))?;
    let sample_rate = choose_sample_rate(device, request.clip.sample_rate, default.sample_rate());
    let channels = default.channels() as usize;
    let config = cpal::StreamConfig {
        channels: channels as u16,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let state = PlaybackState::new(
        request,
        sample_rate,
        channels,
        Arc::clone(&shared),
        events.clone(),
    )?;
    shared
        .stack
        .lock()
        .configure(sample_rate as f64, DEFAULT_BLOCK_FRAMES)
        .unwrap_or_else(|error| log::warn!("plugin stack failed to activate: {error}"));
    let stream = build_stream(device, &config, default.sample_format(), state, events)?;
    stream
        .play()
        .map_err(|e| anyhow!("starting output stream: {e}"))?;
    shared.playing.store(true, Ordering::Relaxed);
    Ok(OpenedStream {
        stream,
        sample_rate,
        channels,
    })
}

/// The device dictates the sample type; rendering stays in f32 and converts on the way out.
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    state: PlaybackState,
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
        other => Err(anyhow!("unsupported output sample format: {other}")),
    }
}

fn build_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: PlaybackState,
    events: Sender<EngineEvent>,
) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _| state.render(data),
            move |error| {
                let _ = events.send(EngineEvent::Error(format!("output stream: {error}")));
            },
            None,
        )
        .map_err(|e| anyhow!("building output stream: {e}"))
}

fn choose_sample_rate(device: &cpal::Device, wanted: u32, fallback: u32) -> u32 {
    let supported = device
        .supported_output_configs()
        .map(|configs| configs.into_iter().any(|range| range.contains_rate(wanted)))
        .unwrap_or(false);
    if supported {
        wanted
    } else {
        fallback
    }
}

struct PlaybackState {
    clip: Arc<AudioClip>,
    /// Document frames per playback frame; differs from 1 when the device forced a resample.
    ratio: f64,
    position: usize,
    end: usize,
    channels: usize,
    scratch: Vec<Vec<f32>>,
    meters: MeterChain,
    shared: Arc<SharedState>,
    events: Sender<EngineEvent>,
    finished: bool,
}

impl PlaybackState {
    fn new(
        request: PlaybackRequest,
        sample_rate: u32,
        channels: usize,
        shared: Arc<SharedState>,
        events: Sender<EngineEvent>,
    ) -> Result<Self> {
        let source_rate = request.clip.sample_rate;
        let clip = if source_rate == sample_rate {
            request.clip
        } else {
            Arc::new(resample(&request.clip, sample_rate, ResampleQuality::Fast)?)
        };
        let ratio = source_rate as f64 / sample_rate as f64;
        let to_playback = |frame: usize| (frame as f64 / ratio).round() as usize;
        // A loop starts at its head; from then on the bounds are read live from shared state.
        let start = match &request.loop_range {
            Some(range) => range.start,
            None => request.start_frame,
        };
        let end = clip.frames();
        Ok(Self {
            meters: MeterChain::new(MeterSource::Playback, channels, sample_rate),
            scratch: vec![Vec::new(); channels],
            position: to_playback(start),
            clip,
            ratio,
            end,
            channels,
            shared,
            events,
            finished: false,
        })
    }

    fn render<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
        let frames = data.len() / self.channels.max(1);
        self.apply_seek();
        self.prepare_scratch(frames);
        self.fill_from_clip(frames);
        self.run_plugin_stack(frames);
        self.publish_meters(frames);
        interleave(&self.scratch, data, self.channels, frames);
        self.publish_position();
    }

    fn apply_seek(&mut self) {
        let seek = self.shared.seek_request.swap(NO_SEEK, Ordering::Relaxed);
        if seek != NO_SEEK {
            self.position = ((seek as f64 / self.ratio).round() as usize).min(self.clip.frames());
            self.finished = false;
        }
    }

    fn prepare_scratch(&mut self, frames: usize) {
        for plane in &mut self.scratch {
            if plane.len() < frames {
                plane.resize(frames, 0.0);
            }
        }
    }

    /// The loop the UI currently wants, in playback frames and clamped to the clip.
    ///
    /// Read once per block rather than cached at stream start, so dragging the loop edges
    /// or toggling the loop off takes effect within a block instead of on the next play.
    fn loop_bounds(&self) -> Option<Range<usize>> {
        let range = self.shared.loop_range()?;
        let to_playback = |frame: usize| (frame as f64 / self.ratio).round() as usize;
        let start = to_playback(range.start).min(self.end);
        let end = to_playback(range.end).min(self.end);
        (end > start).then_some(start..end)
    }

    fn fill_from_clip(&mut self, frames: usize) {
        let looping = self.loop_bounds();
        for frame in 0..frames {
            let end = looping.as_ref().map_or(self.end, |range| range.end);
            if self.position >= end {
                match &looping {
                    // Covers a loop that shrank or moved behind the playhead as well as the
                    // ordinary wrap: either way the next frame comes from the loop head.
                    Some(range) => self.position = range.start,
                    None => {
                        self.write_silence_frame(frame);
                        self.mark_finished();
                        continue;
                    }
                }
            }
            for (channel, plane) in self.scratch.iter_mut().enumerate() {
                plane[frame] = self
                    .clip
                    .channels
                    .get(channel)
                    .or(self.clip.channels.first())
                    .map_or(0.0, |c| c[self.position]);
            }
            self.position += 1;
        }
    }

    fn write_silence_frame(&mut self, frame: usize) {
        for plane in &mut self.scratch {
            plane[frame] = 0.0;
        }
    }

    fn mark_finished(&mut self) {
        if !self.finished {
            self.finished = true;
            self.shared.playing.store(false, Ordering::Relaxed);
            let _ = self.events.try_send(EngineEvent::PlaybackFinished);
        }
    }

    fn run_plugin_stack(&mut self, frames: usize) {
        if let Some(mut stack) = self.shared.stack.try_lock() {
            let mut planes: Vec<&mut [f32]> =
                self.scratch.iter_mut().map(|p| &mut p[..frames]).collect();
            stack.process(&mut planes, frames);
        }
    }

    fn publish_meters(&mut self, frames: usize) {
        let planes: Vec<&[f32]> = self.scratch.iter().map(|p| &p[..frames]).collect();
        let snapshot = self
            .meters
            .measure(&planes, self.shared.vu_reference_dbfs());
        self.shared.publish_meters(snapshot);
    }

    fn publish_position(&self) {
        let document_frame = (self.position as f64 * self.ratio).round() as usize;
        self.shared
            .play_position
            .store(document_frame, Ordering::Relaxed);
    }
}

fn interleave<T: Sample + FromSample<f32>>(
    planes: &[Vec<f32>],
    data: &mut [T],
    channels: usize,
    frames: usize,
) {
    for frame in 0..frames {
        for channel in 0..channels {
            data[frame * channels + channel] = T::from_sample(planes[channel][frame]);
        }
    }
}

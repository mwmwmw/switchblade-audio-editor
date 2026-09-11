use anyhow::Result;

use super::{PluginDescriptor, PluginInstance};
use crate::audio::AudioClip;

/// The stack always runs stereo; mono material is duplicated and other channels pass through.
pub const STACK_CHANNELS: usize = 2;
pub const DEFAULT_BLOCK_FRAMES: usize = 1024;

pub struct PluginSlot {
    pub instance: Box<dyn PluginInstance>,
    pub bypassed: bool,
}

pub struct ProcessingStack {
    slots: Vec<PluginSlot>,
    sample_rate: f64,
    block_frames: usize,
    active: bool,
    scratch_a: Vec<Vec<f32>>,
    scratch_b: Vec<Vec<f32>>,
}

impl Default for ProcessingStack {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            sample_rate: 0.0,
            block_frames: DEFAULT_BLOCK_FRAMES,
            active: false,
            scratch_a: vec![vec![0.0; DEFAULT_BLOCK_FRAMES]; STACK_CHANNELS],
            scratch_b: vec![vec![0.0; DEFAULT_BLOCK_FRAMES]; STACK_CHANNELS],
        }
    }
}

impl ProcessingStack {
    pub fn slots(&self) -> &[PluginSlot] {
        &self.slots
    }

    pub fn slots_mut(&mut self) -> &mut [PluginSlot] {
        &mut self.slots
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn has_active_plugins(&self) -> bool {
        self.slots.iter().any(|slot| !slot.bypassed)
    }

    pub fn add(&mut self, descriptor: &PluginDescriptor) -> Result<()> {
        let mut instance = super::load(descriptor)?;
        if self.active {
            instance.activate(self.sample_rate, self.block_frames)?;
        }
        self.slots.push(PluginSlot {
            instance,
            bypassed: false,
        });
        Ok(())
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.slots.len() {
            let mut slot = self.slots.remove(index);
            slot.instance.deactivate();
        }
    }

    pub fn move_slot(&mut self, from: usize, to: usize) {
        if from < self.slots.len() && to < self.slots.len() {
            let slot = self.slots.remove(from);
            self.slots.insert(to, slot);
        }
    }

    /// (Re)activates every plugin for the given stream format.
    pub fn configure(&mut self, sample_rate: f64, block_frames: usize) -> Result<()> {
        self.deactivate();
        self.sample_rate = sample_rate;
        self.block_frames = block_frames.max(1);
        for plane in self.scratch_a.iter_mut().chain(self.scratch_b.iter_mut()) {
            plane.resize(self.block_frames, 0.0);
        }
        for slot in &mut self.slots {
            slot.instance.activate(sample_rate, self.block_frames)?;
        }
        self.active = true;
        Ok(())
    }

    pub fn deactivate(&mut self) {
        if self.active {
            for slot in &mut self.slots {
                slot.instance.deactivate();
            }
        }
        self.active = false;
    }

    /// Processes `frames` of planar audio in place. Blocks longer than the configured size are split.
    pub fn process(&mut self, planes: &mut [&mut [f32]], frames: usize) {
        if !self.active || !self.has_active_plugins() || planes.is_empty() {
            return;
        }
        let mut offset = 0;
        while offset < frames {
            let count = (frames - offset).min(self.block_frames);
            self.process_block(planes, offset, count);
            offset += count;
        }
    }

    fn process_block(&mut self, planes: &mut [&mut [f32]], offset: usize, count: usize) {
        copy_in(&mut self.scratch_a, planes, offset, count);
        for slot in self.slots.iter_mut().filter(|slot| !slot.bypassed) {
            slot.instance
                .process(&self.scratch_a, &mut self.scratch_b, count);
            std::mem::swap(&mut self.scratch_a, &mut self.scratch_b);
        }
        copy_out(&self.scratch_a, planes, offset, count);
    }

    /// Renders the whole clip through the stack; the realtime configuration is restored afterwards.
    pub fn render_offline(&mut self, clip: &AudioClip) -> Result<AudioClip> {
        let realtime = (self.sample_rate, self.block_frames, self.active);
        self.configure(clip.sample_rate as f64, DEFAULT_BLOCK_FRAMES)?;
        let mut rendered = clip.clone();
        let frames = rendered.frames();
        let mut planes: Vec<&mut [f32]> = rendered
            .channels
            .iter_mut()
            .map(Vec::as_mut_slice)
            .collect();
        self.process(&mut planes, frames);
        self.restore(realtime)?;
        Ok(rendered)
    }

    fn restore(&mut self, (sample_rate, block_frames, active): (f64, usize, bool)) -> Result<()> {
        if active {
            self.configure(sample_rate, block_frames)
        } else {
            self.deactivate();
            Ok(())
        }
    }
}

fn copy_in(scratch: &mut [Vec<f32>], planes: &[&mut [f32]], offset: usize, count: usize) {
    for (channel, plane) in scratch.iter_mut().enumerate() {
        let source = planes
            .get(channel)
            .or(planes.first())
            .map(|p| &p[offset..offset + count]);
        match source {
            Some(source) => plane[..count].copy_from_slice(source),
            None => plane[..count].fill(0.0),
        }
    }
}

fn copy_out(scratch: &[Vec<f32>], planes: &mut [&mut [f32]], offset: usize, count: usize) {
    for (channel, plane) in planes.iter_mut().enumerate().take(STACK_CHANNELS) {
        plane[offset..offset + count].copy_from_slice(&scratch[channel][..count]);
    }
}

use std::ops::Range;

/// Planar floating-point audio: one `Vec<f32>` per channel, all the same length.
#[derive(Clone, Debug, Default)]
pub struct AudioClip {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioClip {
    pub fn new(sample_rate: u32, channel_count: usize) -> Self {
        Self {
            sample_rate,
            channels: vec![Vec::new(); channel_count],
        }
    }

    pub fn silence(sample_rate: u32, channel_count: usize, frames: usize) -> Self {
        Self {
            sample_rate,
            channels: vec![vec![0.0; frames]; channel_count],
        }
    }

    pub fn from_interleaved(sample_rate: u32, channel_count: usize, interleaved: &[f32]) -> Self {
        let frames = interleaved.len() / channel_count.max(1);
        let mut clip = Self::silence(sample_rate, channel_count, frames);
        for (frame, samples) in interleaved.chunks_exact(channel_count).enumerate() {
            for (channel, sample) in samples.iter().enumerate() {
                clip.channels[channel][frame] = *sample;
            }
        }
        clip
    }

    pub fn to_interleaved(&self) -> Vec<f32> {
        let channel_count = self.channel_count();
        let mut out = vec![0.0; self.frames() * channel_count];
        for (channel, samples) in self.channels.iter().enumerate() {
            for (frame, sample) in samples.iter().enumerate() {
                out[frame * channel_count + channel] = *sample;
            }
        }
        out
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.frames() == 0
    }

    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 / self.sample_rate as f64
    }

    pub fn full_range(&self) -> Range<usize> {
        0..self.frames()
    }

    pub fn clamp_range(&self, range: &Range<usize>) -> Range<usize> {
        let end = range.end.min(self.frames());
        range.start.min(end)..end
    }

    pub fn peak(&self, range: &Range<usize>) -> f32 {
        let range = self.clamp_range(range);
        self.channels
            .iter()
            .flat_map(|channel| channel[range.clone()].iter())
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    }

    pub fn slice(&self, range: &Range<usize>) -> AudioClip {
        let range = self.clamp_range(range);
        AudioClip {
            sample_rate: self.sample_rate,
            channels: self
                .channels
                .iter()
                .map(|channel| channel[range.clone()].to_vec())
                .collect(),
        }
    }

    pub fn remove(&mut self, range: &Range<usize>) {
        let range = self.clamp_range(range);
        for channel in &mut self.channels {
            channel.drain(range.clone());
        }
    }

    pub fn insert(&mut self, at: usize, other: &AudioClip) {
        let at = at.min(self.frames());
        for (index, channel) in self.channels.iter_mut().enumerate() {
            let source = other.channel_or_first(index);
            channel.splice(at..at, source.iter().copied());
        }
    }

    /// Falls back to the first channel so mono material can be pasted into stereo.
    fn channel_or_first(&self, index: usize) -> &[f32] {
        self.channels
            .get(index)
            .or(self.channels.first())
            .map_or(&[], Vec::as_slice)
    }

    pub fn silence_range(&mut self, range: &Range<usize>) {
        let range = self.clamp_range(range);
        for channel in &mut self.channels {
            channel[range.clone()].fill(0.0);
        }
    }

    pub fn apply_gain(&mut self, range: &Range<usize>, gain: f32) {
        let range = self.clamp_range(range);
        for channel in &mut self.channels {
            for sample in &mut channel[range.clone()] {
                *sample *= gain;
            }
        }
    }
}

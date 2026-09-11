use crate::audio::AudioClip;

pub const DEFAULT_PEAK_BUCKET: usize = 256;

/// Min/max of fixed-size sample buckets, so zoomed-out waveform drawing stays O(pixels).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PeakMipmap {
    pub bucket: usize,
    pub mins: Vec<Vec<f32>>,
    pub maxs: Vec<Vec<f32>>,
}

impl PeakMipmap {
    pub fn build(clip: &AudioClip) -> Self {
        Self {
            bucket: DEFAULT_PEAK_BUCKET,
            mins: clip
                .channels
                .iter()
                .map(|c| bucket_reduce(c, DEFAULT_PEAK_BUCKET, f32::min))
                .collect(),
            maxs: clip
                .channels
                .iter()
                .map(|c| bucket_reduce(c, DEFAULT_PEAK_BUCKET, f32::max))
                .collect(),
        }
    }

    pub fn matches(&self, clip: &AudioClip) -> bool {
        self.bucket > 0
            && self.mins.len() == clip.channel_count()
            && self
                .mins
                .iter()
                .zip(&clip.channels)
                .all(|(m, c)| m.len() == c.len().div_ceil(self.bucket))
    }

    /// Min and max over `start..end` of `samples`, using buckets for the fully covered interior.
    pub fn min_max(&self, samples: &[f32], channel: usize, start: usize, end: usize) -> (f32, f32) {
        let first_bucket = start.div_ceil(self.bucket);
        let last_bucket = end / self.bucket;
        let (Some(mins), Some(maxs)) = (self.mins.get(channel), self.maxs.get(channel)) else {
            return scan_min_max(&samples[start..end]);
        };
        if first_bucket >= last_bucket || last_bucket > mins.len() {
            return scan_min_max(&samples[start..end]);
        }
        let (mut min, mut max) = (f32::MAX, f32::MIN);
        for bucket in first_bucket..last_bucket {
            min = min.min(mins[bucket]);
            max = max.max(maxs[bucket]);
        }
        for edge in [
            &samples[start..first_bucket * self.bucket],
            &samples[last_bucket * self.bucket..end],
        ] {
            let (edge_min, edge_max) = scan_min_max(edge);
            min = min.min(edge_min);
            max = max.max(edge_max);
        }
        (min, max)
    }
}

fn bucket_reduce(samples: &[f32], bucket: usize, reduce: fn(f32, f32) -> f32) -> Vec<f32> {
    samples
        .chunks(bucket)
        .map(|chunk| chunk.iter().copied().fold(chunk[0], reduce))
        .collect()
}

pub fn scan_min_max(samples: &[f32]) -> (f32, f32) {
    samples.iter().fold((f32::MAX, f32::MIN), |(min, max), s| {
        (min.min(*s), max.max(*s))
    })
}

use anyhow::{anyhow, Result};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

use super::AudioClip;

pub const STANDARD_SAMPLE_RATES: &[u32] = &[
    8000, 11025, 16000, 22050, 32000, 44100, 48000, 88200, 96000, 176400, 192000,
];

const CHUNK_FRAMES: usize = 1024;
const HIGH_QUALITY_SINC_LENGTH: usize = 256;
const HIGH_QUALITY_OVERSAMPLING: usize = 256;
const RATIO_HEADROOM: f64 = 1.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResampleQuality {
    High,
    Fast,
}

impl ResampleQuality {
    pub const ALL: [ResampleQuality; 2] = [ResampleQuality::High, ResampleQuality::Fast];

    pub fn label(self) -> &'static str {
        match self {
            ResampleQuality::High => "High (windowed sinc, 256 taps)",
            ResampleQuality::Fast => "Fast (FFT)",
        }
    }
}

pub fn resample(clip: &AudioClip, target_rate: u32, quality: ResampleQuality) -> Result<AudioClip> {
    if clip.sample_rate == target_rate || clip.is_empty() {
        return Ok(AudioClip {
            sample_rate: target_rate,
            channels: clip.channels.clone(),
        });
    }
    let channels = clip.channel_count();
    let mut resampler = build_resampler(clip.sample_rate, target_rate, channels, quality)?;
    let input = SequentialSliceOfVecs::new(&clip.channels, channels, clip.frames())
        .map_err(|error| anyhow!("resampler input: {error}"))?;
    let output = resampler
        .process_all(&input, clip.frames(), None)
        .map_err(|error| anyhow!("resampling failed: {error}"))?;
    Ok(AudioClip::from_interleaved(
        target_rate,
        channels,
        &output.take_data(),
    ))
}

fn build_resampler(
    from_rate: u32,
    to_rate: u32,
    channels: usize,
    quality: ResampleQuality,
) -> Result<Box<dyn Resampler<f32>>> {
    let resampler: Box<dyn Resampler<f32>> = match quality {
        ResampleQuality::High => {
            let ratio = to_rate as f64 / from_rate as f64;
            Box::new(
                Async::<f32>::new_sinc(
                    ratio,
                    RATIO_HEADROOM,
                    &high_quality_parameters(),
                    CHUNK_FRAMES,
                    channels,
                    FixedAsync::Input,
                )
                .map_err(|error| anyhow!("sinc resampler: {error}"))?,
            )
        }
        ResampleQuality::Fast => Box::new(
            Fft::<f32>::new(
                from_rate as usize,
                to_rate as usize,
                CHUNK_FRAMES,
                channels,
                FixedSync::Input,
            )
            .map_err(|error| anyhow!("fft resampler: {error}"))?,
        ),
    };
    Ok(resampler)
}

fn high_quality_parameters() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: HIGH_QUALITY_SINC_LENGTH,
        f_cutoff: None,
        oversampling_factor: HIGH_QUALITY_OVERSAMPLING,
        interpolation: SincInterpolationType::Cubic,
        window: WindowFunction::BlackmanHarris2,
    }
}

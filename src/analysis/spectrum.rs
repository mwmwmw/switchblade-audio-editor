use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::audio::db::power_to_db;
use crate::audio::AudioClip;

/// ISO one-third-octave centre frequencies, matching the ISO 226 table.
pub const BAND_CENTER_HZ: [f32; 29] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
    500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
    8000.0, 10000.0, 12500.0,
];
pub const BAND_COUNT: usize = BAND_CENTER_HZ.len();
/// Index of the 1 kHz band; handy for tests and tooling.
#[cfg(test)]
pub const REFERENCE_BAND_INDEX: usize = 17;

const FFT_SIZE: usize = 8192;
const THIRD_OCTAVE_HALF_WIDTH: f32 = 1.0 / 6.0;

/// Long-term average level per one-third-octave band, in dB (relative units).
#[derive(Clone, Copy, Debug)]
pub struct BandSpectrum {
    pub levels_db: [f32; BAND_COUNT],
}

impl Default for BandSpectrum {
    fn default() -> Self {
        Self {
            levels_db: [crate::audio::db::SILENCE_DB; BAND_COUNT],
        }
    }
}

pub fn average_band_spectrum(clip: &AudioClip) -> BandSpectrum {
    if clip.is_empty() {
        return BandSpectrum::default();
    }
    let power_bins = average_power_spectrum(clip);
    let bin_hz = clip.sample_rate as f32 / FFT_SIZE as f32;
    let levels_db =
        BAND_CENTER_HZ.map(|center| power_to_db(band_power(&power_bins, bin_hz, center)));
    BandSpectrum { levels_db }
}

fn average_power_spectrum(clip: &AudioClip) -> Vec<f32> {
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
    let window = hann_window();
    let mut accumulated = vec![0.0_f32; FFT_SIZE / 2];
    let mut buffer = vec![Complex::new(0.0, 0.0); FFT_SIZE];
    let mut block_count = 0_usize;
    for channel in &clip.channels {
        for block in channel.chunks(FFT_SIZE) {
            fill_windowed(&mut buffer, block, &window);
            fft.process(&mut buffer);
            for (sum, bin) in accumulated.iter_mut().zip(&buffer) {
                *sum += bin.norm_sqr();
            }
            block_count += 1;
        }
    }
    let scale = 1.0 / (block_count.max(1) as f32 * FFT_SIZE as f32);
    accumulated.iter().map(|p| p * scale).collect()
}

fn hann_window() -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos())
        .collect()
}

fn fill_windowed(buffer: &mut [Complex<f32>], block: &[f32], window: &[f32]) {
    for (index, slot) in buffer.iter_mut().enumerate() {
        let sample = block.get(index).copied().unwrap_or(0.0);
        *slot = Complex::new(sample * window[index], 0.0);
    }
}

fn band_power(power_bins: &[f32], bin_hz: f32, center_hz: f32) -> f32 {
    let low = center_hz * 2.0_f32.powf(-THIRD_OCTAVE_HALF_WIDTH);
    let high = center_hz * 2.0_f32.powf(THIRD_OCTAVE_HALF_WIDTH);
    let first = (low / bin_hz).ceil() as usize;
    let last = ((high / bin_hz).floor() as usize).min(power_bins.len().saturating_sub(1));
    if first > last {
        return power_bins
            .get(((center_hz / bin_hz).round() as usize).min(power_bins.len() - 1))
            .copied()
            .unwrap_or(0.0);
    }
    power_bins[first..=last].iter().sum::<f32>() / (last - first + 1) as f32
}

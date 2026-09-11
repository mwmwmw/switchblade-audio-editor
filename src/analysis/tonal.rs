//! Tonal balance against ISO 226:2003 equal-loudness contours or a reference track.

use super::spectrum::{BandSpectrum, BAND_COUNT};

const ISO226_ALPHA_F: [f32; BAND_COUNT] = [
    0.532, 0.506, 0.480, 0.455, 0.432, 0.409, 0.387, 0.367, 0.349, 0.330, 0.315, 0.301, 0.288,
    0.276, 0.267, 0.259, 0.253, 0.250, 0.246, 0.244, 0.243, 0.243, 0.243, 0.242, 0.242, 0.245,
    0.254, 0.271, 0.301,
];
const ISO226_L_U: [f32; BAND_COUNT] = [
    -31.6, -27.2, -23.0, -19.1, -15.9, -13.0, -10.3, -8.1, -6.2, -4.5, -3.1, -2.0, -1.1, -0.4, 0.0,
    0.3, 0.5, 0.0, -2.7, -4.1, -1.0, 1.7, 2.5, 1.2, -2.1, -7.1, -11.2, -10.7, -3.1,
];
const ISO226_T_F: [f32; BAND_COUNT] = [
    78.5, 68.7, 59.5, 51.1, 44.0, 37.5, 31.5, 26.5, 22.1, 17.9, 14.4, 11.4, 8.6, 6.2, 4.4, 3.0,
    2.2, 2.4, 3.5, 1.7, -1.3, -4.2, -6.0, -5.4, -1.5, 6.0, 12.6, 13.9, 12.3,
];

pub const PHON_PRESETS: &[f32] = &[40.0, 60.0, 70.0, 80.0, 90.0];
pub const DEFAULT_PHON: f32 = 80.0;

/// Sound pressure level (dB SPL) per band that is perceived as equally loud at `phon`.
pub fn equal_loudness_contour(phon: f32) -> [f32; BAND_COUNT] {
    let mut contour = [0.0_f32; BAND_COUNT];
    for (index, level) in contour.iter_mut().enumerate() {
        *level = iso226_spl(
            phon,
            ISO226_ALPHA_F[index],
            ISO226_L_U[index],
            ISO226_T_F[index],
        );
    }
    contour
}

fn iso226_spl(phon: f32, alpha_f: f32, l_u: f32, t_f: f32) -> f32 {
    let a_f = 4.47e-3 * (10.0_f32.powf(0.025 * phon) - 1.15)
        + (0.4 * 10.0_f32.powf((t_f + l_u) / 10.0 - 9.0)).powf(alpha_f);
    (10.0 / alpha_f) * a_f.log10() - l_u + 94.0
}

#[derive(Clone, Copy, Debug)]
pub struct TonalBalance {
    /// Positive values mean the band is louder than the target; in dB.
    pub deviation_db: [f32; BAND_COUNT],
}

impl TonalBalance {
    /// How loud each band sounds relative to the average band, after the ear's frequency response.
    /// The result is anchored on the mean so a dip in any single band does not shift the others.
    pub fn against_equal_loudness(spectrum: &BandSpectrum, phon: f32) -> Self {
        let contour = equal_loudness_contour(phon);
        let mut perceived = [0.0_f32; BAND_COUNT];
        for index in 0..BAND_COUNT {
            perceived[index] = spectrum.levels_db[index] - contour[index];
        }
        Self {
            deviation_db: centred_levels(&perceived),
        }
    }

    /// Pink noise has equal energy per third-octave band, and typical program material sits close
    /// to it, so this is the physical spectrum compared with a flat band spectrum.
    pub fn against_pink(spectrum: &BandSpectrum) -> Self {
        Self {
            deviation_db: centred_levels(&spectrum.levels_db),
        }
    }

    pub fn against_reference(spectrum: &BandSpectrum, reference: &BandSpectrum) -> Self {
        let own = centred(spectrum);
        let other = centred(reference);
        let mut deviation_db = [0.0_f32; BAND_COUNT];
        for index in 0..BAND_COUNT {
            deviation_db[index] = own[index] - other[index];
        }
        Self { deviation_db }
    }
}

fn centred(spectrum: &BandSpectrum) -> [f32; BAND_COUNT] {
    centred_levels(&spectrum.levels_db)
}

fn centred_levels(levels: &[f32; BAND_COUNT]) -> [f32; BAND_COUNT] {
    let mean = levels.iter().sum::<f32>() / BAND_COUNT as f32;
    levels.map(|level| level - mean)
}

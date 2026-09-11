//! Dither for fixed-point export.
//!
//! Rounding a float to 16 or 24 bits correlates the rounding error with the signal, which
//! is audible on quiet material as gritty distortion rather than as hiss. Adding a little
//! noise before rounding decorrelates the error: the distortion becomes a steady, far less
//! objectionable noise floor. TPDF (triangular probability density) noise of two LSBs peak
//! to peak is the standard choice — it fully decorrelates both the error and its variance,
//! which rectangular noise does not.

/// Deterministic, allocation-free noise source; exports should be reproducible.
///
/// xorshift64*, which is plenty for dither and avoids pulling in a rand dependency.
pub struct Dither {
    state: u64,
    /// Amplitude of one quantisation step, in full-scale units.
    step: f32,
}

const SEED: u64 = 0x2545_F491_4F6C_DD1D;

impl Dither {
    /// `bits` is the target word length; 16-bit gets a step of 1/32768, and so on.
    pub fn new(bits: u16) -> Self {
        Self {
            state: SEED,
            step: 1.0 / (1_i64 << (bits - 1)) as f32,
        }
    }

    fn next_uniform(&mut self) -> f32 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let bits = self.state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Top 24 bits to [0, 1): f32 has 24 bits of mantissa, so nothing is wasted.
        (bits >> 40) as f32 / (1_u32 << 24) as f32
    }

    /// One TPDF sample, ±1 LSB peak, mean zero. The difference of two uniforms is triangular.
    pub fn next(&mut self) -> f32 {
        (self.next_uniform() - self.next_uniform()) * self.step
    }
}

use num_complex::Complex32;

/// Simple FM demodulator using phase differentiation.
///
/// This implements the standard quadrature demodulation technique:
///
/// ```text
/// y[n] = angle(x[n] * conj(x[n-1]))
/// ```
///
/// Where:
/// - x[n] is the current complex sample
/// - x[n-1] is the previous sample
///
/// This extracts instantaneous phase change, which corresponds to frequency deviation.
///
/// Notes:
/// - Output is in radians/sample
/// - Scaling to Hz or audio level is handled downstream
pub struct FmDemodulator {
    /// Previous IQ sample (used for phase difference)
    prev: Complex32,

    /// Indicates whether `prev` is valid
    have_prev: bool,
}

impl Default for FmDemodulator {
    fn default() -> Self {
        Self::new()
    }
}

impl FmDemodulator {
    /// Create a new FM demodulator.
    pub fn new() -> Self {
        Self {
            prev: Complex32::new(0.0, 0.0),
            have_prev: false,
        }
    }

    /// Reset internal state.
    ///
    /// This should be called when:
    /// - switching radios
    /// - large frequency jumps occur
    /// - stream discontinuities happen
    pub fn reset(&mut self) {
        self.prev = Complex32::new(0.0, 0.0);
        self.have_prev = false;
    }

    /// Demodulate FM audio from complex baseband input.
    ///
    /// Output:
    /// - one audio sample per input sample
    /// - first sample is 0.0 (no previous sample available)
    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(input.len());

        for &current in input {
            if !self.have_prev {
                // First sample: no previous sample to compare against
                self.prev = current;
                self.have_prev = true;
                output.push(0.0);
                continue;
            }

            // Phase difference via complex multiply with conjugate
            let delta = current * self.prev.conj();

            // Extract instantaneous phase (atan2 is robust)
            let audio = delta.im.atan2(delta.re);

            output.push(audio);

            // Update previous sample
            self.prev = current;
        }

        output
    }
}

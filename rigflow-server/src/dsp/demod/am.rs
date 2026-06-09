use num_complex::Complex32;

/// Simple AM demodulator using envelope detection.
///
/// This computes:
///
/// ```text
/// y[n] = |x[n]|
/// ```
///
/// Where:
/// - x[n] is the current complex IQ sample
///
/// Notes:
/// - Output is non-negative before downstream DC blocking
/// - Carrier/DC removal is expected to happen downstream
/// - Scaling/AGC/filtering is handled by the pipeline
pub struct AmDemodulator;

impl Default for AmDemodulator {
    fn default() -> Self {
        Self::new()
    }
}

impl AmDemodulator {
    /// Create a new AM demodulator.
    pub fn new() -> Self {
        Self
    }

    /// Reset internal state.
    ///
    /// Present for interface consistency with other demodulators.
    pub fn reset(&mut self) {}

    /// Demodulate AM audio from complex baseband input.
    ///
    /// Output:
    /// - one audio sample per input sample
    /// - output is the raw envelope magnitude
    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(input.len());

        for &sample in input {
            output.push(sample.norm());
        }

        output
    }
}

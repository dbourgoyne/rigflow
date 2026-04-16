use num_complex::Complex32;
use std::f32::consts::TAU;

/// Simple CW demodulator using a fixed beat-frequency oscillator (BFO).
///
/// This assumes the incoming IQ has already been channelized around the target
/// signal. The demodulator mixes the signal with a complex oscillator to create
/// an audible tone, then outputs the real part.
///
/// Notes:
/// - Output is raw mono `f32` audio
/// - DC blocking / AGC / audio filtering are expected downstream
/// - If the CW signal is exactly centered, the output tone will be near `bfo_hz`
pub struct CwDemodulator {
    sample_rate_hz: f32,
    bfo_hz: f32,
    phase: f32,
    phase_step: f32,
}

impl Default for CwDemodulator {
    fn default() -> Self {
        Self::new(12_000.0, 700.0)
    }
}

impl CwDemodulator {
    /// Create a new CW demodulator.
    ///
    /// `sample_rate_hz` is the IQ sample rate entering the CW demod stage.
    /// `bfo_hz` is the desired CW sidetone frequency, typically 600-800 Hz.
    pub fn new(sample_rate_hz: f32, bfo_hz: f32) -> Self {
        assert!(sample_rate_hz > 0.0, "sample_rate_hz must be > 0");
        assert!(bfo_hz >= 0.0, "bfo_hz must be >= 0");
        assert!(
            bfo_hz < sample_rate_hz * 0.5,
            "bfo_hz must be below Nyquist"
        );

        let phase_step = TAU * bfo_hz / sample_rate_hz;

        Self {
            sample_rate_hz,
            bfo_hz,
            phase: 0.0,
            phase_step,
        }
    }

    /// Reset internal phase state.
    pub fn reset(&mut self) {
        self.phase = 0.0;
    }

    /// Update the sample rate and recompute oscillator step.
    pub fn set_sample_rate(&mut self, sample_rate_hz: f32) {
        assert!(sample_rate_hz > 0.0, "sample_rate_hz must be > 0");
        assert!(
            self.bfo_hz < sample_rate_hz * 0.5,
            "bfo_hz must be below Nyquist"
        );

        self.sample_rate_hz = sample_rate_hz;
        self.phase_step = TAU * self.bfo_hz / self.sample_rate_hz;
    }

    /// Update the desired CW sidetone frequency.
    pub fn set_bfo_hz(&mut self, bfo_hz: f32) {
        assert!(bfo_hz >= 0.0, "bfo_hz must be >= 0");
        assert!(
            bfo_hz < self.sample_rate_hz * 0.5,
            "bfo_hz must be below Nyquist"
        );

        self.bfo_hz = bfo_hz;
        self.phase_step = TAU * self.bfo_hz / self.sample_rate_hz;
    }

    /// Demodulate CW audio from complex baseband input.
    ///
    /// Output:
    /// - one audio sample per input sample
    /// - raw beat-note audio centered around the chosen BFO frequency
    pub fn process(&mut self, input: &[Complex32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(input.len());

        for &sample in input {
            // Complex oscillator: e^(-j*phase)
            let osc = Complex32::new(self.phase.cos(), -self.phase.sin());

            // Mix to audio.
            let mixed = sample * osc;

            // Use the real part as audio. Downstream filtering cleans it up.
            output.push(mixed.re);

            self.phase += self.phase_step;
            if self.phase >= TAU {
                self.phase -= TAU;
            }
        }

        output
    }
}

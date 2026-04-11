use std::f32::consts::PI;

use num_complex::Complex32;

/// Frequency-shifting virtual tuner.
///
/// Multiplies incoming IQ samples by a complex exponential to shift
/// the desired target frequency down to baseband (0 Hz).
///
/// Effectively performs:
///     f_out = f_in - (target - center)
pub struct VirtualTuner {
    center_freq_hz: f32,
    target_freq_hz: f32,
    sample_rate_hz: f32,

    /// Current oscillator state (rotating phasor)
    osc: Complex32,

    /// Per-sample phase increment (unit-magnitude complex step)
    step: Complex32,
}

impl VirtualTuner {
    pub fn new(center_freq_hz: f32, target_freq_hz: f32, sample_rate_hz: f32) -> Self {
        let mut tuner = Self {
            center_freq_hz,
            target_freq_hz,
            sample_rate_hz,
            osc: Complex32::new(1.0, 0.0),
            step: Complex32::new(1.0, 0.0),
        };

        tuner.update_step();
        tuner
    }

    pub fn center_frequency(&self) -> f32 {
        self.center_freq_hz
    }

    pub fn target_frequency(&self) -> f32 {
        self.target_freq_hz
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate_hz
    }

    pub fn set_center_frequency(&mut self, center_freq_hz: f32) {
        self.center_freq_hz = center_freq_hz;
        self.update_step();
    }

    pub fn set_target_frequency(&mut self, target_freq_hz: f32) {
        self.target_freq_hz = target_freq_hz;
        self.update_step();
    }

    pub fn set_sample_rate(&mut self, sample_rate_hz: f32) {
        self.sample_rate_hz = sample_rate_hz;
        self.update_step();
    }

    /// Reset oscillator phase to 0 (1 + j0)
    pub fn reset_phase(&mut self) {
        self.osc = Complex32::new(1.0, 0.0);
    }

    /// Process into a newly allocated output buffer.
    ///
    /// Convenience wrapper around `process_into`.
    pub fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut output = Vec::with_capacity(input.len());
        self.process_into(input, &mut output);
        output
    }

    /// Process in-place (zero allocation).
    ///
    /// This is the preferred path for low-latency DSP pipelines.
    pub fn process_in_place(&mut self, samples: &mut [Complex32]) {
        let mut osc = self.osc;
        let step = self.step;

        for sample in samples {
            *sample *= osc;
            osc *= step;
        }

        self.osc = normalize_if_needed(osc);
    }

    /// Process into caller-provided buffer (allocation-free path).
    pub fn process_into(&mut self, input: &[Complex32], output: &mut Vec<Complex32>) {
        output.clear();
        output.reserve(input.len().saturating_sub(output.capacity()));

        let mut osc = self.osc;
        let step = self.step;

        for &sample in input {
            output.push(sample * osc);
            osc *= step;
        }

        self.osc = normalize_if_needed(osc);
    }

    /// Recompute oscillator step when frequency parameters change.
    fn update_step(&mut self) {
        let shift_hz = self.center_freq_hz - self.target_freq_hz;
        let phase_inc = 2.0 * PI * shift_hz / self.sample_rate_hz;

        self.step = Complex32::new(phase_inc.cos(), phase_inc.sin());
    }
}

/// Renormalize occasionally to control floating-point drift.
///
/// In exact math, |osc| stays 1 forever.
/// In floating-point, repeated multiplication slowly drifts.
fn normalize_if_needed(z: Complex32) -> Complex32 {
    let norm_sqr = z.norm_sqr();

    if (norm_sqr - 1.0).abs() > 1e-3 {
        let norm = norm_sqr.sqrt();

        if norm > 0.0 {
            z / norm
        } else {
            // Extremely unlikely fallback
            Complex32::new(1.0, 0.0)
        }
    } else {
        z
    }
}

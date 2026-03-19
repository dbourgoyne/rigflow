use num_complex::Complex32;
use std::f32::consts::PI;

pub struct VirtualTuner {
    center_freq_hz: f32,
    target_freq_hz: f32,
    sample_rate_hz: f32,
    osc: Complex32,
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

    pub fn reset_phase(&mut self) {
        self.osc = Complex32::new(1.0, 0.0);
    }

    pub fn process(&mut self, input: &[Complex32]) -> Vec<Complex32> {
        let mut output = Vec::with_capacity(input.len());
        let mut osc = self.osc;
        let step = self.step;

        for &sample in input {
            output.push(sample * osc);
            osc *= step;
        }

        self.osc = normalize_if_needed(osc);
        output
    }

    pub fn process_in_place(&mut self, samples: &mut [Complex32]) {
        let mut osc = self.osc;
        let step = self.step;

        for sample in samples.iter_mut() {
            *sample *= osc;
            osc *= step;
        }

        self.osc = normalize_if_needed(osc);
    }

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
            Complex32::new(1.0, 0.0)
        }
    } else {
        z
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualTuner;
    use num_complex::Complex32;
    use std::f32::consts::PI;

    fn approx_eq(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() < eps
    }

    /*
    Test: target -> DC
    Simulates: real SDR case: SDR center = 10 Hz, signal at 12 Hz
    Verifies: that the tuner correctly shifts signal to baseband 0 Hz
    */
    #[test]
    fn shifts_target_frequency_to_dc() {
        let sample_rate_hz = 48_000.0;

        // Simulate SDR centered at 10 kHz
        let center_freq_hz = 10_000.0;

        // We want to tune to a signal at 12 kHz
        let target_freq_hz = 12_000.0;

        let num_samples = 2048;

        // The tone appears at +2 kHz relative to center
        let relative_tone_hz = target_freq_hz - center_freq_hz;

        // Generate IQ signal containing that tone
        let input: Vec<Complex32> = (0..num_samples)
            .map(|n| {
                let phase = 2.0 * PI * relative_tone_hz * (n as f32) / sample_rate_hz;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        // Create tuner that shifts target → DC
        let mut tuner = VirtualTuner::new(
            center_freq_hz,
            target_freq_hz,
            sample_rate_hz,
        );

        let output = tuner.process(&input);

        // Ignore initial transient
        let steady = &output[32..];

        // Compute average
        let mean_re = steady.iter().map(|s| s.re).sum::<f32>() / steady.len() as f32;
        let mean_im = steady.iter().map(|s| s.im).sum::<f32>() / steady.len() as f32;

        // Expect DC (constant signal ~1 + j0)
        assert!(
            approx_eq(mean_re, 1.0, 1e-3),
            "real part not ~1.0: {mean_re}"
        );

        assert!(
            approx_eq(mean_im, 0.0, 1e-3),
            "imag part not ~0.0: {mean_im}"
        );

        // Check variation is small (signal is flat)
        let max_dev = steady
            .iter()
            .map(|s| ((s.re - mean_re).powi(2) + (s.im - mean_im).powi(2)).sqrt())
            .fold(0.0_f32, f32::max);

        assert!(
            max_dev < 1e-2,
            "output not stable at DC, max deviation {max_dev}"
        );
    }

    /*
    test; general frequency shift
    verifies: f_out = f_in - (f_target - f_center), uses phase difference methog to estimate output frequency
    */
    #[test]
    fn shifts_frequency_correctly_nonzero_result() {
        let sample_rate_hz = 48_000.0;
        let center_freq_hz = 10_000.0;
        let target_freq_hz = 12_000.0;

        let num_samples = 2048;

        // Input tone at +3 kHz relative to center
        let input_tone_hz = 3_000.0;

        let input: Vec<Complex32> = (0..num_samples)
            .map(|n| {
                let phase = 2.0 * PI * input_tone_hz * (n as f32) / sample_rate_hz;
                Complex32::new(phase.cos(), phase.sin())
            })
            .collect();

        let mut tuner = VirtualTuner::new(
            center_freq_hz,
            target_freq_hz,
            sample_rate_hz,
        );

        let output = tuner.process(&input);

        // Expected output frequency:
        // f_out = f_in - (target - center)
        let shift = target_freq_hz - center_freq_hz;
        let expected_freq = input_tone_hz - shift;

        // Estimate frequency from phase difference
        let mut phase_diffs = Vec::new();

        for i in 1..output.len() {
            let prev = output[i - 1];
            let curr = output[i];

            let phase_prev = prev.arg();
            let phase_curr = curr.arg();

            let mut diff = phase_curr - phase_prev;

            // unwrap phase
            if diff > PI {
                diff -= 2.0 * PI;
            } else if diff < -PI {
                diff += 2.0 * PI;
            }

            phase_diffs.push(diff);
        }

        let mean_diff = phase_diffs.iter().sum::<f32>() / phase_diffs.len() as f32;

        let estimated_freq = mean_diff * sample_rate_hz / (2.0 * PI);

        assert!(
            approx_eq(estimated_freq, expected_freq, 5.0),
            "expected freq {expected_freq}, got {estimated_freq}"
        );
    }
}
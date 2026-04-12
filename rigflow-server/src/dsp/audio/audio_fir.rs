use std::f32::consts::PI;

/// Streaming low-pass FIR filter for real-valued audio samples.
///
/// This implements a standard FIR filter with:
/// - circular delay line
/// - windowed-sinc low-pass design
///
/// Characteristics:
/// - linear phase (due to symmetric taps)
/// - stable
/// - predictable latency (group delay = (num_taps - 1) / 2)
pub struct AudioFir {
    /// FIR coefficients (impulse response)
    taps: Vec<f32>,

    /// Circular delay buffer
    delay: Vec<f32>,

    /// Current write position in delay line
    pos: usize,
}

impl AudioFir {
    /// Create a low-pass FIR using a windowed-sinc design.
    ///
    /// Parameters:
    /// - `sample_rate_hz`: audio sample rate
    /// - `cutoff_hz`: low-pass cutoff frequency
    /// - `num_taps`: filter length (usually odd for symmetry)
    pub fn new(sample_rate_hz: f32, cutoff_hz: f32, num_taps: usize) -> Self {
        assert!(sample_rate_hz > 0.0, "sample_rate_hz must be > 0");
        assert!(cutoff_hz > 0.0, "cutoff_hz must be > 0");
        assert!(
            cutoff_hz < sample_rate_hz * 0.5,
            "cutoff_hz must be below Nyquist"
        );
        assert!(num_taps >= 3, "num_taps must be at least 3");

        let taps = design_lowpass(sample_rate_hz, cutoff_hz, num_taps);

        Self {
            delay: vec![0.0; num_taps],
            taps,
            pos: 0,
        }
    }

    /// Reset internal state.
    ///
    /// Clears delay line and resets write position.
    pub fn reset(&mut self) {
        self.delay.fill(0.0);
        self.pos = 0;
    }

    /// Process a single sample through the FIR.
    ///
    /// Steps:
    /// 1. Write new sample into circular buffer
    /// 2. Convolve with taps (reverse order through delay line)
    /// 3. Advance write position
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        self.delay[self.pos] = sample;

        let len = self.taps.len();
        let mut idx = self.pos;
        let mut acc = 0.0_f32;

        // Convolution: newest sample aligns with first tap
        for &tap in &self.taps {
            acc += self.delay[idx] * tap;

            if idx == 0 {
                idx = len - 1;
            } else {
                idx -= 1;
            }
        }

        // Advance circular buffer position
        self.pos += 1;
        if self.pos == len {
            self.pos = 0;
        }

        acc
    }

    /// Process a slice and return a newly allocated output buffer.
    ///
    /// Convenience wrapper around `process_sample`.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&x| self.process_sample(x)).collect()
    }

    /// Process samples in-place (preferred for real-time pipelines).
    ///
    /// Avoids allocation and is the recommended path for low latency.
    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.process_sample(*sample);
        }
    }
}

/// Design a low-pass FIR using windowed-sinc method.
///
/// Steps:
/// 1. Ideal sinc low-pass
/// 2. Apply Hamming window
/// 3. Normalize to unity DC gain
fn design_lowpass(sample_rate_hz: f32, cutoff_hz: f32, num_taps: usize) -> Vec<f32> {
    let fc = cutoff_hz / sample_rate_hz;
    let m = (num_taps - 1) as f32;
    let mid = m / 2.0;

    let mut taps = Vec::with_capacity(num_taps);

    for n in 0..num_taps {
        let x = n as f32 - mid;

        // Ideal sinc low-pass
        let sinc = if x.abs() < 1e-12 {
            2.0 * fc
        } else {
            let arg = 2.0 * PI * fc * x;
            arg.sin() / (PI * x)
        };

        // Hamming window
        let window = 0.54 - 0.46 * (2.0 * PI * n as f32 / m).cos();

        taps.push(sinc * window);
    }

    // Normalize to unity DC gain
    let sum: f32 = taps.iter().sum();
    for tap in &mut taps {
        *tap /= sum;
    }

    taps
}

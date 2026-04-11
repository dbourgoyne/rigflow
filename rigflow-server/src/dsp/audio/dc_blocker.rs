/// Simple first-order DC blocker (high-pass filter).
///
/// Implements the difference equation:
///
///     y[n] = x[n] - x[n-1] + r * y[n-1]
///
/// Where:
/// - `r` controls the cutoff frequency (closer to 1.0 = lower cutoff)
///
/// Characteristics:
/// - removes DC offset and very low-frequency drift
/// - minimal computational cost
/// - small phase distortion at low frequencies (acceptable for audio)
///
/// Typical usage:
/// - after demodulation (especially FM)
/// - before AGC
pub struct DcBlocker {
    /// Feedback coefficient (0 < r < 1)
    r: f32,

    /// Previous input sample (x[n-1])
    prev_x: f32,

    /// Previous output sample (y[n-1])
    prev_y: f32,
}

impl DcBlocker {
    /// Create a new DC blocker.
    ///
    /// Recommended values:
    /// - 0.995 → moderate cutoff (~a few Hz at audio rates)
    /// - 0.999 → very low cutoff (more aggressive DC removal)
    pub fn new(r: f32) -> Self {
        assert!(r > 0.0 && r < 1.0, "r must be between 0 and 1");

        Self {
            r,
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    /// Reset internal filter state.
    ///
    /// Should be called when:
    /// - switching radios
    /// - stream discontinuities occur
    /// - avoiding transient artifacts after large jumps
    pub fn reset(&mut self) {
        self.prev_x = 0.0;
        self.prev_y = 0.0;
    }

    /// Process a single sample.
    ///
    /// Applies a simple high-pass filter to remove DC offset.
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        // Difference equation:
        // y[n] = x[n] - x[n-1] + r * y[n-1]
        let output = sample - self.prev_x + self.r * self.prev_y;

        self.prev_x = sample;
        self.prev_y = output;

        output
    }

    /// Process a slice and return a newly allocated output buffer.
    ///
    /// Convenience wrapper around `process_sample`.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&x| self.process_sample(x)).collect()
    }

    /// Process samples in-place (preferred for low-latency pipelines).
    ///
    /// Avoids allocation and is the recommended path for real-time DSP.
    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.process_sample(*sample);
        }
    }
}

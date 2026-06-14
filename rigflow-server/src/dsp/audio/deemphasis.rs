use log::debug;

/// First-order deemphasis filter for FM audio.
///
/// This implements a simple RC low-pass filter:
///
/// ```text
/// y[n] = y[n-1] + alpha * (x[n] - y[n-1])
/// ```
///
/// Where:
/// - `tau` is the time constant (e.g. 75 µs in North America)
/// - `alpha = dt / (tau + dt)`
///
/// Purpose:
/// - FM transmission applies **pre-emphasis** (boosts high frequencies)
/// - This filter reverses that effect (deemphasis)
///
/// Characteristics:
/// - first-order IIR filter
/// - smooths high-frequency components
/// - very low computational cost
///
/// Typical values:
/// - 75 µs → North America FM broadcast
/// - 50 µs → Europe FM broadcast
pub struct DeemphasisFilter {
    /// Filter coefficient derived from tau and sample rate
    alpha: f32,

    /// Previous output sample (y[n-1])
    y_prev: f32,
}

impl DeemphasisFilter {
    /// Create a deemphasis filter.
    ///
    /// Parameters:
    /// - `sample_rate_hz`: audio sample rate
    /// - `tau_seconds`: time constant (e.g. 75e-6 for 75 µs)
    pub fn new(sample_rate_hz: f32, tau_seconds: f32) -> Self {
        debug!(
            "DeemphasisFilter: new: sample_rate_hz = {}, tau_seconds = {}",
            sample_rate_hz, tau_seconds
        );
        assert!(sample_rate_hz > 0.0, "sample_rate_hz must be > 0");
        assert!(tau_seconds > 0.0, "tau_seconds must be > 0");

        let dt = 1.0 / sample_rate_hz;

        // Standard RC filter discretization
        let alpha = dt / (tau_seconds + dt);

        Self { alpha, y_prev: 0.0 }
    }

    /// Reset internal filter state.
    ///
    /// Should be called when:
    /// - switching radios
    /// - stream discontinuities occur
    pub fn reset(&mut self) {
        self.y_prev = 0.0;
    }

    /// Process samples in-place.
    ///
    /// This is the preferred path for real-time DSP:
    /// - no allocation
    /// - minimal overhead
    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples {
            // y[n] = y[n-1] + alpha * (x[n] - y[n-1])
            self.y_prev = self.y_prev + self.alpha * (*sample - self.y_prev);
            *sample = self.y_prev;
        }
    }
}

/// Simple peak-tracking automatic gain control (AGC).
///
/// This AGC operates on scalar audio samples and:
/// - tracks signal envelope using attack/decay smoothing
/// - applies gain to drive the signal toward a target level
/// - limits maximum gain to avoid excessive amplification
///
/// This is a **feed-forward AGC** using envelope estimation.
///
/// Behavior:
/// - fast attack: quickly reacts to increases in signal level
/// - slow decay: slowly recovers gain when signal drops
///
/// Typical use:
/// - SSB audio leveling
/// - general amplitude normalization in SDR pipelines
pub struct Agc {
    /// Desired output level (rough target amplitude)
    target_level: f32,

    /// Envelope smoothing factor when signal is increasing
    attack: f32,

    /// Envelope smoothing factor when signal is decreasing
    decay: f32,

    /// Maximum allowed gain
    max_gain: f32,

    /// Internal envelope state (tracks signal magnitude)
    envelope: f32,
}

impl Agc {
    /// Create a new AGC instance.
    ///
    /// Parameters:
    /// - `target_level`: desired output amplitude (e.g. 0.3–0.7)
    /// - `attack`: smoothing factor for rising signals (closer to 1.0 = slower)
    /// - `decay`: smoothing factor for falling signals (closer to 1.0 = slower)
    /// - `max_gain`: upper limit on amplification
    pub fn new(target_level: f32, attack: f32, decay: f32, max_gain: f32) -> Self {
        assert!(target_level > 0.0, "target_level must be > 0");
        assert!(
            attack > 0.0 && attack < 1.0,
            "attack must be between 0 and 1"
        );
        assert!(
            decay > 0.0 && decay < 1.0,
            "decay must be between 0 and 1"
        );
        assert!(max_gain > 0.0, "max_gain must be > 0");

        Self {
            target_level,
            attack,
            decay,
            max_gain,
            envelope: 0.0,
        }
    }

    /// Reset internal envelope state.
    ///
    /// Should be called when:
    /// - switching radios
    /// - stream discontinuities occur
    /// - large gain jumps are undesirable
    pub fn reset(&mut self) {
        self.envelope = 0.0;
    }

    /// Process a single sample through the AGC.
    ///
    /// Steps:
    /// 1. Measure instantaneous magnitude
    /// 2. Update envelope (attack/decay smoothing)
    /// 3. Compute gain relative to target level
    /// 4. Apply gain (with max clamp)
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        let level = sample.abs();

        // Envelope tracking with asymmetric response:
        // - attack when level rises
        // - decay when level falls
        if level > self.envelope {
            self.envelope = self.attack * self.envelope + (1.0 - self.attack) * level;
        } else {
            self.envelope = self.decay * self.envelope + (1.0 - self.decay) * level;
        }

        // Compute gain toward target level
        let gain = if self.envelope > 1e-9 {
            (self.target_level / self.envelope).min(self.max_gain)
        } else {
            // Avoid division instability when envelope is near zero
            self.max_gain
        };

        sample * gain
    }

    /// Process a slice and return a newly allocated output buffer.
    ///
    /// Convenience wrapper around `process_sample`.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        input.iter().map(|&x| self.process_sample(x)).collect()
    }

    /// Process samples in-place (preferred for low-latency pipelines).
    ///
    /// This avoids allocation and is the recommended path in real-time DSP.
    pub fn process_in_place(&mut self, samples: &mut [f32]) {
        for sample in samples {
            *sample = self.process_sample(*sample);
        }
    }
}

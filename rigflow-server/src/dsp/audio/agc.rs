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

    /// When false, samples pass through unchanged (gain == 1.0).
    enabled: bool,

    /// Most recent applied gain (diagnostics only).
    current_gain: f32,
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
        assert!(decay > 0.0 && decay < 1.0, "decay must be between 0 and 1");
        assert!(max_gain > 0.0, "max_gain must be > 0");

        Self {
            target_level,
            attack,
            decay,
            max_gain,
            envelope: 0.0,
            enabled: true,
            current_gain: 1.0,
        }
    }

    /// Reset internal envelope state.
    ///
    /// Should be called when:
    /// - switching radios
    /// - stream discontinuities occur
    /// - large gain jumps are undesirable
    ///
    /// Leaves the operator settings (enabled / attack / decay) intact.
    pub fn reset(&mut self) {
        self.envelope = 0.0;
        self.current_gain = 1.0;
    }

    /// Enable or disable the AGC.  When disabled, audio passes through
    /// unchanged (gain == 1.0).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Map operator AGC strength in [0, 1] to attack/decay responsiveness.
    ///
    /// Higher strength → faster (more aggressive) gain adaptation.  The mapping
    /// is centred so `strength = 0.5` reproduces the previous always-on tuning
    /// (attack 0.90, decay 0.999), giving no behaviour change at the default.
    /// `target_level` and `max_gain` are left unchanged.
    pub fn set_strength(&mut self, strength: f32) {
        let s = strength.clamp(0.0, 1.0);
        // attack: 0.97 (slow) .. 0.83 (fast); decay kept near 1.0 to avoid
        // pumping: 0.9995 (very slow) .. 0.9985.
        self.attack = 0.97 - 0.14 * s;
        self.decay = 0.9995 - 0.001 * s;
    }

    /// Most recently applied gain (diagnostics).
    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }

    /// Current tracked envelope (diagnostics).
    pub fn envelope(&self) -> f32 {
        self.envelope
    }

    /// Process a single sample through the AGC.
    ///
    /// Steps:
    /// 1. Measure instantaneous magnitude
    /// 2. Update envelope (attack/decay smoothing)
    /// 3. Compute gain relative to target level
    /// 4. Apply gain (with max clamp)
    pub fn process_sample(&mut self, sample: f32) -> f32 {
        // Disabled → pass through unchanged.
        if !self.enabled {
            self.current_gain = 1.0;
            return sample;
        }

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

        self.current_gain = gain;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make() -> Agc {
        Agc::new(0.3, 0.9, 0.999, 20.0)
    }

    #[test]
    fn disabled_is_passthrough() {
        let mut agc = make();
        agc.set_enabled(false);
        let input: Vec<f32> = (0..512)
            .map(|n| 0.02 * (2.0 * std::f32::consts::PI * 700.0 * n as f32 / 48000.0).sin())
            .collect();
        let out = agc.process(&input);
        assert_eq!(out, input, "disabled AGC must pass audio through unchanged");
        assert_eq!(agc.current_gain(), 1.0);
    }

    #[test]
    fn enabled_output_finite_and_bounded() {
        let mut agc = make();
        agc.set_enabled(true);
        agc.set_strength(0.5);
        // Quiet signal: AGC should boost but never exceed max_gain.
        let input = vec![0.001f32; 4096];
        let out = agc.process(&input);
        assert!(out.iter().all(|s| s.is_finite()), "no NaN/Inf");
        assert!(
            agc.current_gain() <= 20.0 + 1e-3,
            "gain bounded by max_gain"
        );
        assert!(
            out.iter().all(|s| s.abs() <= 0.001 * 20.0 + 1e-6),
            "output bounded"
        );
    }

    #[test]
    fn silence_stays_finite() {
        let mut agc = make();
        let out = agc.process(&vec![0.0; 1024]);
        assert!(out.iter().all(|s| s.is_finite() && *s == 0.0));
    }

    #[test]
    fn strength_maps_attack_decay_centered_on_half() {
        let mut agc = make();
        agc.set_strength(0.5);
        assert!((agc.attack - 0.90).abs() < 1e-6);
        assert!((agc.decay - 0.999).abs() < 1e-6);
        // Higher strength → faster (smaller) attack factor.
        agc.set_strength(1.0);
        let fast_attack = agc.attack;
        agc.set_strength(0.0);
        assert!(
            agc.attack > fast_attack,
            "lower strength is slower (attack closer to 1)"
        );
    }

    #[test]
    fn reset_clears_envelope_and_gain() {
        let mut agc = make();
        let _ = agc.process(&vec![0.5; 1024]);
        agc.reset();
        assert_eq!(agc.envelope(), 0.0);
        assert_eq!(agc.current_gain(), 1.0);
    }
}

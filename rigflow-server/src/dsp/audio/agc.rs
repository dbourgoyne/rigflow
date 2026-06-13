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

    /// Operator "strength" in [0, 1] — the *amount* of AGC applied.  The computed
    /// gain's deviation from unity is scaled by this, so `0` = no leveling
    /// (passthrough, identical to disabled) and `1` = full AGC.
    strength: f32,

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
            // Full strength until the operator sets one, so an AGC that is built
            // but never told a strength behaves as a normal full AGC.
            strength: 1.0,
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

    /// Set the operator AGC strength in [0, 1] — the *amount* of leveling applied.
    ///
    /// The computed gain is interpolated toward unity by this value (see
    /// `process_sample`), so `0` = no leveling (identical to disabled) and `1` =
    /// full AGC.  The envelope attack/decay time constants are left at their
    /// construction values; strength controls amount, not response speed.
    pub fn set_strength(&mut self, strength: f32) {
        self.strength = strength.clamp(0.0, 1.0);
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

        // Scale the gain's deviation from unity by strength: strength 0 leaves the
        // sample untouched (== disabled), strength 1 applies the full computed gain.
        let effective_gain = 1.0 + self.strength * (gain - 1.0);
        self.current_gain = effective_gain;
        sample * effective_gain
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
    fn strength_zero_is_passthrough() {
        // Enabled but strength 0 must be identical to disabled (gain == 1.0).
        let mut agc = make();
        agc.set_enabled(true);
        agc.set_strength(0.0);
        let input: Vec<f32> = (0..512)
            .map(|n| 0.02 * (2.0 * std::f32::consts::PI * 700.0 * n as f32 / 48000.0).sin())
            .collect();
        let out = agc.process(&input);
        assert_eq!(out, input, "strength 0 must pass audio through unchanged");
        assert_eq!(agc.current_gain(), 1.0);
    }

    #[test]
    fn strength_scales_gain_between_unity_and_full() {
        // Steady quiet input drives the raw gain to its max; the applied gain must
        // interpolate from 1.0 (strength 0) to the full gain (strength 1), with
        // 0.5 landing halfway.
        let gain_at = |s: f32| {
            let mut agc = make();
            agc.set_enabled(true);
            agc.set_strength(s);
            let _ = agc.process(&vec![0.001f32; 4096]);
            agc.current_gain()
        };
        let none = gain_at(0.0);
        let half = gain_at(0.5);
        let full = gain_at(1.0);
        assert_eq!(none, 1.0, "strength 0 → unity gain");
        assert!(full > 1.5, "strength 1 → meaningful boost (got {full})");
        let expected_half = 1.0 + 0.5 * (full - 1.0);
        assert!(
            (half - expected_half).abs() < 1e-3,
            "strength 0.5 gain {half} should be halfway between 1.0 and {full}"
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

//! Soft-knee peak limiter for the SSB transmit audio chain (ALC Phase 1).
//!
//! This is a simple feed-forward peak limiter — **not** a closed-loop ALC and
//! **not** a compressor.  It sits between the band-limit/DC stage and the SSB
//! modulator and gently pulls the gain down when the audio envelope exceeds a
//! threshold, protecting the modulator (and the transmitted spectrum) from
//! clipping and the splatter it causes.
//!
//! Design (per the spec: peak detector → attack/release envelope → gain):
//! 1. **Peak detector + envelope:** a one-pole follower tracks `|x|` with a
//!    fast attack (envelope rises quickly when the signal gets louder) and a
//!    slow release (falls slowly), so the gain changes smoothly with no
//!    pumping or abrupt steps.
//! 2. **Soft-knee static curve:** the gain reduction is computed from the
//!    envelope in dB with a soft knee around the threshold — limiting eases in
//!    rather than switching on hard, which keeps it transparent during normal
//!    speech and avoids the distortion of a hard `clamp()`.
//! 3. **Gain:** the (smoothly-varying) envelope drives a per-sample gain, so
//!    the output never hard-clips at the threshold.
//!
//! `process_in_place` returns the peak gain reduction (dB, ≥0) applied over the
//! block, for the TX gain-reduction meter.

/// Knee width in dB around the threshold for the soft-knee curve.
const KNEE_DB: f32 = 6.0;

/// Soft-knee peak limiter operating on mono audio at a fixed sample rate.
pub struct TxLimiter {
    /// Threshold in dBFS (e.g. 90% → ≈ -0.92 dB).
    threshold_db: f32,
    /// One-pole envelope coefficients (closer to 1.0 = slower).
    attack_coeff: f32,
    release_coeff: f32,
    /// Envelope follower state (linear amplitude).
    env: f32,
}

impl TxLimiter {
    /// Create a limiter.
    ///
    /// - `sample_rate_hz` — audio rate (48 kHz for the TX path).
    /// - `threshold_linear` — limit threshold, 0..1 (fraction of full scale).
    /// - `attack_ms` / `release_ms` — envelope time constants.
    pub fn new(
        sample_rate_hz: f32,
        threshold_linear: f32,
        attack_ms: f32,
        release_ms: f32,
    ) -> Self {
        let threshold_linear = threshold_linear.clamp(0.05, 0.999);
        let tau_coeff = |ms: f32| (-1.0 / (ms.max(0.1) * 0.001 * sample_rate_hz)).exp();
        Self {
            threshold_db: 20.0 * threshold_linear.log10(),
            attack_coeff: tau_coeff(attack_ms),
            release_coeff: tau_coeff(release_ms),
            env: 0.0,
        }
    }

    /// Static soft-knee gain reduction (dB, ≥0) for an envelope level in dB.
    /// Infinite ratio above the knee (a true limiter); quadratic in the knee.
    fn reduction_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        if over <= -KNEE_DB / 2.0 {
            0.0
        } else if over >= KNEE_DB / 2.0 {
            over
        } else {
            let x = over + KNEE_DB / 2.0;
            (x * x) / (2.0 * KNEE_DB)
        }
    }

    /// Limit `samples` in place; returns the peak gain reduction (dB) applied.
    pub fn process_in_place(&mut self, samples: &mut [f32]) -> f32 {
        let mut peak_gr_db = 0.0f32;
        for s in samples.iter_mut() {
            let inst = s.abs();
            // Envelope follower: fast attack (rising), slow release (falling).
            let coeff = if inst > self.env {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.env = coeff * self.env + (1.0 - coeff) * inst;

            // Static soft-knee curve on the smoothed envelope.
            let env_db = 20.0 * self.env.max(1e-9).log10();
            let gr_db = self.reduction_db(env_db);
            if gr_db > peak_gr_db {
                peak_gr_db = gr_db;
            }
            if gr_db > 0.0 {
                *s *= 10.0f32.powf(-gr_db / 20.0);
            }
        }
        peak_gr_db
    }
}

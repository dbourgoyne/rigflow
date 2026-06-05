//! Feed-forward speech compressor for the SSB transmit audio chain.
//!
//! Sits **before** the [`super::tx_limiter::TxLimiter`] (compressor first,
//! limiter second): it raises average voice level / talk power by reducing
//! dynamic range, while the limiter remains the final peak-protection stage.
//! This is a plain downward compressor — **not** a closed-loop ALC, not
//! adaptive, not multi-band, no automatic gain riding.
//!
//! Stages (per the spec): envelope detector → gain computer → attack/release
//! smoothing → gain application:
//! 1. **Envelope follower** — one-pole tracker of `|x|` with attack/release
//!    times, so the gain moves smoothly (no pumping).
//! 2. **Soft-knee gain computer** — above the threshold the level is reduced
//!    toward the chosen ratio; a 6 dB knee eases limiting in.
//! 3. **Make-up gain** — a fixed boost (derived from threshold and ratio) that
//!    restores compressed peaks back toward full scale, so quiet speech is
//!    lifted and the average transmitted level rises.  Residual peaks are
//!    caught by the downstream limiter.
//!
//! `process_in_place` returns the peak gain reduction (dB, ≥0, excluding
//! make-up) over the block, for the compression gain-reduction meter.

/// Threshold for compression onset, in dBFS.
const THRESHOLD_DB: f32 = -15.0;
/// Soft-knee width in dB around the threshold.
const KNEE_DB: f32 = 6.0;

/// Map a UI compression level (0–10) to a compression ratio.
/// 0 → 1:1 (off); anchors match the spec (1→1.5, 3→2, 5→3, 10→6).
pub fn ratio_for_level(level: u8) -> f32 {
    match level {
        0 => 1.0,
        1 => 1.5,
        2 => 1.8,
        3 => 2.0,
        4 => 2.5,
        5 => 3.0,
        6 => 3.5,
        7 => 4.0,
        8 => 4.5,
        9 => 5.0,
        _ => 6.0, // 10 and above
    }
}

/// Feed-forward soft-knee speech compressor on mono audio.
pub struct SpeechCompressor {
    threshold_db: f32,
    ratio: f32,
    makeup_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    env: f32,
}

impl SpeechCompressor {
    /// Create a compressor.
    ///
    /// - `sample_rate_hz` — audio rate (48 kHz for the TX path).
    /// - `ratio` — compression ratio (≥1; 1.0 = no compression).
    /// - `attack_ms` / `release_ms` — envelope time constants.
    pub fn new(sample_rate_hz: f32, ratio: f32, attack_ms: f32, release_ms: f32) -> Self {
        let ratio = ratio.max(1.0);
        let tau_coeff = |ms: f32| (-1.0 / (ms.max(0.1) * 0.001 * sample_rate_hz)).exp();
        // Make-up restores the gain lost at ~0 dBFS, i.e. |T|·(1 − 1/ratio),
        // lifting quiet speech and raising the average level.
        let makeup_db = THRESHOLD_DB.abs() * (1.0 - 1.0 / ratio);
        Self {
            threshold_db: THRESHOLD_DB,
            ratio,
            makeup_db,
            attack_coeff: tau_coeff(attack_ms),
            release_coeff: tau_coeff(release_ms),
            env: 0.0,
        }
    }

    /// Soft-knee gain reduction (dB, ≥0) for an envelope level in dB.
    fn reduction_db(&self, level_db: f32) -> f32 {
        let over = level_db - self.threshold_db;
        let slope = 1.0 - 1.0 / self.ratio;
        if over <= -KNEE_DB / 2.0 {
            0.0
        } else if over >= KNEE_DB / 2.0 {
            over * slope
        } else {
            let x = over + KNEE_DB / 2.0;
            slope * (x * x) / (2.0 * KNEE_DB)
        }
    }

    /// Compress `samples` in place; returns the peak gain reduction (dB) applied
    /// (excluding make-up gain), for the meter.
    pub fn process_in_place(&mut self, samples: &mut [f32]) -> f32 {
        let mut peak_gr_db = 0.0f32;
        for s in samples.iter_mut() {
            let inst = s.abs();
            let coeff = if inst > self.env {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.env = coeff * self.env + (1.0 - coeff) * inst;

            let env_db = 20.0 * self.env.max(1e-9).log10();
            let gr_db = self.reduction_db(env_db);
            if gr_db > peak_gr_db {
                peak_gr_db = gr_db;
            }
            // Net gain = make-up − compression reduction.
            let gain_db = self.makeup_db - gr_db;
            *s *= 10.0f32.powf(gain_db / 20.0);
        }
        peak_gr_db
    }
}

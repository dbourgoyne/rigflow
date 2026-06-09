//! NR2-style spectral noise reduction for receive audio.
//!
//! A first-pass, Quisk-NR2-inspired (spectral, not LMS) noise reducer applied
//! to demodulated audio.  It is a weighted overlap-add (WOLA) STFT processor:
//!
//! 1. Buffer incoming audio and process fixed `FRAME`-sample frames with 50 %
//!    overlap, using a √Hann analysis+synthesis window (perfect reconstruction
//!    at unity gain, COLA at 50 % hop).
//! 2. Per FFT bin, track a slow noise-floor estimate (minima tracking with a
//!    slow upward leak).
//! 3. Compute a decision-directed Wiener gain (Ephraim-Malah-lite) per bin and
//!    apply it.  A gain floor limits maximum suppression to avoid musical
//!    noise / over-gating.
//! 4. Overlap-add back to time domain.
//!
//! When disabled the worker never calls this, so audio is passed through
//! untouched (no latency, no allocation).  While enabled it adds ~one frame of
//! latency (FRAME/48 kHz ≈ 5 ms).  State is reset on demod-mode / sample-rate
//! changes via [`Nr2::reset`].

use std::sync::Arc;

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// STFT frame size (samples).  256 @ 48 kHz ≈ 5.3 ms; bins = 129.
const FRAME: usize = 256;
/// Hop = 50 % overlap.
const HOP: usize = FRAME / 2;
/// Number of unique (real-FFT) bins.
const BINS: usize = FRAME / 2 + 1;

/// Decision-directed smoothing factor for the a-priori SNR.
const DD_ALPHA: f32 = 0.98;
/// Deepest suppression at full strength: gain floor at strength = 1.0
/// (≈ -26 dB).
const NR2_MIN_GAIN_FLOOR: f32 = 0.05;
/// Default strength used until the operator sets one.
const NR2_DEFAULT_STRENGTH: f32 = 0.5;
/// Upward leak for the noise-floor minima tracker (per processed frame).
const NOISE_LEAK_UP: f32 = 0.02;
/// Small constant to keep divisions finite.
const EPS: f32 = 1e-12;

/// Map a strength in [0, 1] to the per-bin gain floor.
///
/// `strength = 0.0` → floor `1.0` (gain pinned to unity → no reduction at all,
/// audio passes through via the unity-gain WOLA reconstruction).
/// `strength = 1.0` → floor `NR2_MIN_GAIN_FLOOR` (maximum suppression).
fn gain_floor_for_strength(strength: f32) -> f32 {
    let s = strength.clamp(0.0, 1.0);
    1.0 - s * (1.0 - NR2_MIN_GAIN_FLOOR)
}

pub struct Nr2 {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    /// √Hann window (used for both analysis and synthesis).
    window: Vec<f32>,
    /// Pending input samples not yet consumed by a frame.
    in_buf: Vec<f32>,
    /// Overlap-add accumulator, length `FRAME`.
    olap: Vec<f32>,
    /// Reusable complex scratch, length `FRAME`.
    scratch: Vec<Complex32>,
    /// Smoothed noise power per bin.
    noise: Vec<f32>,
    /// Previous clean (post-gain) power per bin (decision-directed history).
    prev_clean: Vec<f32>,
    /// Whether the noise estimate has been seeded.
    seeded: bool,
    /// Whether any state has been accumulated (so the worker can skip reset).
    active: bool,
    /// Per-bin gain floor (max suppression), derived from operator strength.
    gain_floor: f32,
}

impl Nr2 {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FRAME);
        let ifft = planner.plan_fft_inverse(FRAME);

        // √Hann: window[n]^2 summed over 50 %-overlapped frames == 1.0, so
        // analysis*synthesis reconstruction is unity at gain == 1.
        let window: Vec<f32> = (0..FRAME)
            .map(|n| {
                let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / FRAME as f32).cos();
                hann.sqrt()
            })
            .collect();

        Self {
            fft,
            ifft,
            window,
            in_buf: Vec::with_capacity(FRAME * 2),
            olap: vec![0.0; FRAME],
            scratch: vec![Complex32::new(0.0, 0.0); FRAME],
            noise: vec![0.0; BINS],
            prev_clean: vec![0.0; BINS],
            seeded: false,
            active: false,
            gain_floor: gain_floor_for_strength(NR2_DEFAULT_STRENGTH),
        }
    }

    /// True if the processor holds accumulated state (buffers / estimates).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Set the reduction strength in [0, 1] (0 = none, 1 = maximum).
    pub fn set_strength(&mut self, strength: f32) {
        self.gain_floor = gain_floor_for_strength(strength);
    }

    /// Clear all state.  Call when the demod mode or sample rate changes, or
    /// when NR2 is disabled, so a later re-enable starts fresh.
    pub fn reset(&mut self) {
        self.in_buf.clear();
        self.olap.iter_mut().for_each(|x| *x = 0.0);
        self.noise.iter_mut().for_each(|x| *x = 0.0);
        self.prev_clean.iter_mut().for_each(|x| *x = 0.0);
        self.seeded = false;
        self.active = false;
    }

    /// Process one block of audio, returning the denoised output.  In steady
    /// state the output length matches the input length (with a fixed ~one-
    /// frame latency); during warm-up fewer samples are returned.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.active = true;
        self.in_buf.extend_from_slice(input);

        let mut out = Vec::with_capacity(input.len());

        while self.in_buf.len() >= FRAME {
            // ── Analysis: window the leading FRAME samples ──────────────────
            for i in 0..FRAME {
                self.scratch[i] = Complex32::new(self.in_buf[i] * self.window[i], 0.0);
            }
            self.fft.process(&mut self.scratch);

            // ── Per-bin spectral gain (bins 0..=FRAME/2; mirror the rest) ───
            for k in 0..BINS {
                let mag2 = self.scratch[k].norm_sqr();

                // Noise-floor estimate: minima tracking with slow upward leak.
                // Seed and new-minimum both snap straight to mag2.
                if !self.seeded || mag2 < self.noise[k] {
                    self.noise[k] = mag2;
                } else {
                    self.noise[k] = (1.0 - NOISE_LEAK_UP) * self.noise[k] + NOISE_LEAK_UP * mag2;
                }
                let noise = self.noise[k].max(EPS);

                // Decision-directed a-priori SNR (Ephraim-Malah-lite).
                let post_snr = mag2 / noise;
                let prior_snr = DD_ALPHA * (self.prev_clean[k] / noise)
                    + (1.0 - DD_ALPHA) * (post_snr - 1.0).max(0.0);
                // Wiener gain from the a-priori SNR.
                let gain = (prior_snr / (1.0 + prior_snr)).clamp(self.gain_floor, 1.0);

                self.prev_clean[k] = gain * gain * mag2;

                self.scratch[k] *= gain;
                // Maintain conjugate symmetry for a real output.
                if k > 0 && k < FRAME - k {
                    let mirror = FRAME - k;
                    self.scratch[mirror] *= gain;
                }
            }
            self.seeded = true;

            // ── Synthesis: IFFT, window, overlap-add ────────────────────────
            self.ifft.process(&mut self.scratch);
            let norm = 1.0 / FRAME as f32;
            for i in 0..FRAME {
                self.olap[i] += self.scratch[i].re * norm * self.window[i];
            }

            // Emit the first HOP finished samples, then shift the accumulator.
            out.extend_from_slice(&self.olap[..HOP]);
            self.olap.copy_within(HOP.., 0);
            for x in self.olap[FRAME - HOP..].iter_mut() {
                *x = 0.0;
            }

            // Advance input by one hop (frames overlap by FRAME - HOP).
            self.in_buf.drain(0..HOP);
        }

        out
    }
}

impl Default for Nr2 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_output_is_finite_no_nan() {
        let mut nr = Nr2::new();
        // Noisy-ish input: tone + pseudo-random noise.
        let mut acc = 0u32;
        let input: Vec<f32> = (0..2048)
            .map(|n| {
                acc = acc.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (acc >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
                0.3 * (2.0 * std::f32::consts::PI * 1000.0 * n as f32 / 48000.0).sin() + 0.1 * noise
            })
            .collect();

        let out = nr.process(&input);
        assert!(!out.is_empty(), "should emit samples after warm-up");
        assert!(
            out.iter().all(|s| s.is_finite()),
            "all samples must be finite"
        );
    }

    #[test]
    fn silence_in_stays_finite_and_bounded() {
        let mut nr = Nr2::new();
        let out = nr.process(&vec![0.0; 1024]);
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(out.iter().all(|s| s.abs() < 1.0));
    }

    #[test]
    fn strength_maps_to_gain_floor() {
        assert!((gain_floor_for_strength(0.0) - 1.0).abs() < 1e-6); // none → unity
        assert!((gain_floor_for_strength(1.0) - NR2_MIN_GAIN_FLOOR).abs() < 1e-6); // max
                                                                                   // Monotonic: more strength → lower floor (deeper suppression).
        assert!(gain_floor_for_strength(0.25) > gain_floor_for_strength(0.75));
        // Out-of-range clamps.
        assert!((gain_floor_for_strength(-1.0) - 1.0).abs() < 1e-6);
        assert!((gain_floor_for_strength(2.0) - NR2_MIN_GAIN_FLOOR).abs() < 1e-6);
    }

    #[test]
    fn strength_zero_is_passthrough() {
        // gain floor = 1.0 pins the Wiener gain to unity → WOLA reconstructs the
        // input exactly (after one frame of latency).
        let mut nr = Nr2::new();
        nr.set_strength(0.0);
        let input: Vec<f32> = (0..FRAME * 6)
            .map(|n| 0.4 * (2.0 * std::f32::consts::PI * 800.0 * n as f32 / 48000.0).sin())
            .collect();
        let out = nr.process(&input);
        // The √Hann 50 %-overlap WOLA reconstructs with zero net delay once the
        // overlap region is fully populated (after the first HOP ramp samples).
        // Compare the steady-state tail of the output to the input directly.
        let mut max_err = 0.0f32;
        for i in (FRAME * 2)..out.len() {
            max_err = max_err.max((out[i] - input[i]).abs());
        }
        assert!(
            max_err < 1e-3,
            "strength 0 should pass through; max_err={max_err}"
        );
    }

    #[test]
    fn reset_clears_state() {
        let mut nr = Nr2::new();
        let _ = nr.process(&vec![0.5; 1024]);
        assert!(nr.is_active());
        nr.reset();
        assert!(!nr.is_active());
        assert!(nr.in_buf.is_empty());
        assert!(nr.noise.iter().all(|&x| x == 0.0));
        assert!(nr.olap.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn steady_state_length_tracks_input() {
        let mut nr = Nr2::new();
        // Prime past warm-up.
        let _ = nr.process(&vec![0.1; FRAME * 4]);
        let out = nr.process(&vec![0.1; HOP * 3]);
        // After warm-up, output length should match the input in hop units.
        assert_eq!(out.len(), HOP * 3);
    }
}

//! Spectral auto-notch with persistence gating (Quisk-style).
//!
//! A weighted overlap-add (STFT) processor on demodulated audio. Each frame it
//! updates a smoothed magnitude spectrum, finds the two strongest bins, and tracks
//! how *persistent* each one is (a steady carrier stays in the same bin frame after
//! frame; speech harmonics move). Only a bin that has been steady for several frames
//! is notched — so steady carriers/heterodynes are nulled while broadband, moving
//! speech passes through untouched. When nothing is persistent the gain is unity
//! everywhere and the √Hann WOLA reconstructs the input exactly. Modeled on Quisk's
//! `dAutoNotch` and the NR2 WOLA skeleton.

use std::sync::Arc;

use num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// STFT frame (1024 @ 48 kHz → ~47 Hz/bin, ~21 ms latency).
const FRAME: usize = 1024;
const HOP: usize = FRAME / 2;
const BINS: usize = FRAME / 2 + 1;
/// Demodulated-audio sample rate (output of `process_audio`).
const RATE: f32 = 48_000.0;

/// Per-frame spectrum smoothing (Quisk uses 0.5).
const AVG: f32 = 0.5;
/// Residual gain inside the notch (deep but not a hard zero, to limit ringing).
const NOTCH_GAIN: f32 = 0.02;
/// Half-width of the notch, in Hz.
const HALF_WIDTH_HZ: f32 = 100.0;
/// Don't notch below this audio frequency (protect low audio / DC).
const MIN_FREQ_HZ: f32 = 150.0;
/// Exclusion half-width (Hz) around a protected frequency (e.g. the CW pitch).
const PROTECT_DELTA_HZ: f32 = 300.0;

#[inline]
fn hz_to_bins(hz: f32) -> usize {
    (hz * FRAME as f32 / RATE) as usize
}

pub struct AutoNotch {
    fft: Arc<dyn Fft<f32>>,
    ifft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    in_buf: Vec<f32>,
    olap: Vec<f32>,
    scratch: Vec<Complex32>,
    avg_spectrum: Vec<f32>,
    // Persistence trackers for the two strongest steady bins.
    old1: usize,
    count1: i32,
    old2: usize,
    count2: i32,
    active: bool,
}

impl AutoNotch {
    pub fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FRAME);
        let ifft = planner.plan_fft_inverse(FRAME);

        let window: Vec<f32> = (0..FRAME)
            .map(|n| {
                let hann = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * n as f32 / FRAME as f32).cos();
                hann.sqrt()
            })
            .collect();

        let min_bin = hz_to_bins(MIN_FREQ_HZ).max(1);
        Self {
            fft,
            ifft,
            window,
            in_buf: Vec::with_capacity(FRAME * 2),
            olap: vec![0.0; FRAME],
            scratch: vec![Complex32::new(0.0, 0.0); FRAME],
            avg_spectrum: vec![0.0; BINS],
            old1: min_bin,
            count1: -4,
            old2: min_bin,
            count2: -4,
            active: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.in_buf.clear();
        self.olap.iter_mut().for_each(|x| *x = 0.0);
        self.avg_spectrum.iter_mut().for_each(|x| *x = 0.0);
        let min_bin = hz_to_bins(MIN_FREQ_HZ).max(1);
        self.old1 = min_bin;
        self.old2 = min_bin;
        self.count1 = -4;
        self.count2 = -4;
        self.active = false;
    }

    /// Notch steady tones. `protect_hz > 0` excludes a band around that frequency
    /// (e.g. the CW pitch) so the wanted tone isn't notched.
    pub fn process(&mut self, input: &[f32], protect_hz: f32) -> Vec<f32> {
        self.active = true;
        self.in_buf.extend_from_slice(input);

        let min_bin = hz_to_bins(MIN_FREQ_HZ).max(1);
        let half_w = hz_to_bins(HALF_WIDTH_HZ).max(1) as isize;
        let protect_bin: isize = if protect_hz > 0.0 {
            hz_to_bins(protect_hz) as isize
        } else {
            -1
        };
        let protect_delta = hz_to_bins(PROTECT_DELTA_HZ) as isize;
        let is_protected = |k: usize| -> bool {
            protect_bin >= 0 && (k as isize - protect_bin).abs() <= protect_delta
        };

        let mut out = Vec::with_capacity(input.len());

        while self.in_buf.len() >= FRAME {
            // ── Analysis ────────────────────────────────────────────────────
            for i in 0..FRAME {
                self.scratch[i] = Complex32::new(self.in_buf[i] * self.window[i], 0.0);
            }
            self.fft.process(&mut self.scratch);

            // ── Update smoothed spectrum; find strongest bin i1 ─────────────
            let mut d1 = 0.0;
            let mut i1 = min_bin;
            for k in min_bin..BINS {
                let m = self.scratch[k].norm();
                self.avg_spectrum[k] = AVG * self.avg_spectrum[k] + (1.0 - AVG) * m;
                if !is_protected(k) && self.avg_spectrum[k] > d1 {
                    d1 = self.avg_spectrum[k];
                    i1 = k;
                }
            }
            // Persistence of i1.
            if (i1 as isize - self.old1 as isize).abs() < 3 {
                self.count1 += 1;
            } else {
                self.count1 -= 1;
            }
            self.count1 = self.count1.clamp(-1, 4);
            if self.count1 < 0 {
                self.old1 = i1;
            }

            // Second strongest bin i2, away from i1.
            let mut d2 = 0.0;
            let mut i2 = min_bin;
            for k in min_bin..BINS {
                if !is_protected(k)
                    && (k as isize - i1 as isize).abs() > 2 * half_w
                    && self.avg_spectrum[k] > d2
                {
                    d2 = self.avg_spectrum[k];
                    i2 = k;
                }
            }
            if (i2 as isize - self.old2 as isize).abs() < 3 {
                self.count2 += 1;
            } else {
                self.count2 -= 1;
            }
            self.count2 = self.count2.clamp(-2, 4);
            if self.count2 < 0 {
                self.old2 = i2;
            }

            // ── Apply notch(es) where a bin is persistent (else unity) ──────
            for k in 0..BINS {
                let mut gain = 1.0;
                if self.count1 > 0 && (k as isize - self.old1 as isize).abs() <= half_w {
                    gain = NOTCH_GAIN;
                }
                if self.count2 > 0 && (k as isize - self.old2 as isize).abs() <= half_w {
                    gain = NOTCH_GAIN;
                }
                if gain != 1.0 {
                    self.scratch[k] *= gain;
                    if k > 0 && k < FRAME - k {
                        let mirror = FRAME - k;
                        self.scratch[mirror] *= gain;
                    }
                }
            }

            // ── Synthesis: IFFT, window, overlap-add ────────────────────────
            self.ifft.process(&mut self.scratch);
            let norm = 1.0 / FRAME as f32;
            for i in 0..FRAME {
                self.olap[i] += self.scratch[i].re * norm * self.window[i];
            }

            out.extend_from_slice(&self.olap[..HOP]);
            self.olap.copy_within(HOP.., 0);
            for x in self.olap[FRAME - HOP..].iter_mut() {
                *x = 0.0;
            }

            self.in_buf.drain(0..HOP);
        }

        out
    }
}

impl Default for AutoNotch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_finite() {
        let mut n = AutoNotch::new();
        let input: Vec<f32> = (0..FRAME * 8)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 1500.0 * i as f32 / RATE).sin())
            .collect();
        let out = n.process(&input, 0.0);
        assert!(!out.is_empty());
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn notches_a_steady_tone_after_persistence() {
        let mut n = AutoNotch::new();
        // A steady 1500 Hz carrier.
        let tone = |i: usize| 0.5 * (2.0 * std::f32::consts::PI * 1500.0 * i as f32 / RATE).sin();
        // Prime well past the persistence threshold.
        let mut idx = 0usize;
        for _ in 0..40 {
            let block: Vec<f32> = (0..HOP)
                .map(|_| {
                    let v = tone(idx);
                    idx += 1;
                    v
                })
                .collect();
            let _ = n.process(&block, 0.0);
        }
        // Measure residual energy of a fresh steady-tone block.
        let block: Vec<f32> = (0..FRAME * 2)
            .map(|_| {
                let v = tone(idx);
                idx += 1;
                v
            })
            .collect();
        let out = n.process(&block, 0.0);
        let in_energy: f32 = block.iter().map(|x| x * x).sum::<f32>() / block.len() as f32;
        let out_energy: f32 = out.iter().map(|x| x * x).sum::<f32>() / out.len().max(1) as f32;
        assert!(
            out_energy < in_energy * 0.25,
            "steady tone should be substantially notched: in={in_energy} out={out_energy}"
        );
    }
}

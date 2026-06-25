//! IQ-domain impulse noise blanker (Quisk-style).
//!
//! Operates on the **wideband complex IQ before demodulation**, where impulses are
//! sharp and localized (after the channel filter they smear into a ring that an
//! audio-domain blanker can't cleanly remove). When a sample's magnitude exceeds the
//! running mean by a factor, it blanks a **window (~500 µs each side)** around the
//! pulse with a smooth ramp, via a delay line — so the blanked region has no hard
//! discontinuity (no click). Ported from Quisk's `NoiseBlanker`.
//!
//! Introduces a fixed delay of `save_size` IQ samples while enabled; state persists
//! across blocks and is reset on sample-rate change or disable.

use num_complex::Complex32;

/// Half-size of the blanking window, in seconds.
const HWINDOW_SECS: f32 = 500.0e-6;

pub struct NoiseBlanker {
    sample_rate: f32,
    /// Sensitivity in [0.0, 1.0] (higher = more aggressive = lower trigger).
    threshold: f32,

    // Delay line + running magnitude sum.
    c_saved: Vec<Complex32>,
    d_saved: Vec<f32>,
    save_sum: f32,
    save_size: usize,
    hwindow_size: usize,
    index: usize,
    win_index: usize,
    state: u8, // 0 = normal, 1 = in-pulse
    active: bool,
}

impl NoiseBlanker {
    pub fn new() -> Self {
        Self {
            sample_rate: 0.0,
            threshold: 0.5,
            c_saved: Vec::new(),
            d_saved: Vec::new(),
            save_sum: 0.0,
            save_size: 0,
            hwindow_size: 0,
            index: 0,
            win_index: 0,
            state: 0,
            active: false,
        }
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold.clamp(0.0, 1.0);
    }

    pub fn reset(&mut self) {
        self.sample_rate = 0.0; // forces re-init on next process
        self.save_sum = 0.0;
        self.index = 0;
        self.win_index = 0;
        self.state = 0;
        self.c_saved.clear();
        self.d_saved.clear();
        self.active = false;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    fn init(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.hwindow_size = ((sample_rate * HWINDOW_SECS) + 0.5) as usize;
        self.hwindow_size = self.hwindow_size.max(1);
        self.save_size = self.hwindow_size * 3;
        self.c_saved = vec![Complex32::new(0.0, 0.0); self.save_size];
        self.d_saved = vec![0.0; self.save_size];
        self.save_sum = 0.0;
        self.index = 0;
        self.win_index = 0;
        self.state = 0;
    }

    /// Blank impulses in the complex IQ block, in place (output delayed by
    /// `save_size`). `sample_rate` is the IQ input rate.
    pub fn process_iq_in_place(&mut self, samples: &mut [Complex32], sample_rate: f32) {
        self.active = true;
        if (self.sample_rate - sample_rate).abs() > 0.5 || self.c_saved.is_empty() {
            self.init(sample_rate);
        }
        // threshold 0 → 6× the mean (gentle), 1 → 2.5× (firm) — Quisk's level range.
        let limit = 6.0 - 3.5 * self.threshold;
        let hwin = self.hwindow_size;
        let save_size = self.save_size;

        for s in samples.iter_mut() {
            let samp = *s;
            // Output the oldest (delayed) sample; save the newest.
            *s = self.c_saved[self.index];
            self.c_saved[self.index] = samp;

            let mag = samp.norm();
            self.save_sum -= self.d_saved[self.index];
            self.d_saved[self.index] = mag;
            self.save_sum += mag;

            let mean = self.save_sum / save_size as f32;
            let is_pulse = mag > mean * limit;

            match self.state {
                0 => {
                    if is_pulse {
                        self.state = 1;
                        // Ramp down the window of samples leading up to the pulse.
                        let mut k = self.index as isize;
                        for j in 0..hwin {
                            self.c_saved[k as usize] *= j as f32 / hwin as f32;
                            k -= 1;
                            if k < 0 {
                                k = save_size as isize - 1;
                            }
                        }
                    } else if self.win_index != 0 {
                        // Pulses stopped — ramp the window back up to 1.0.
                        self.c_saved[self.index] *= self.win_index as f32 / hwin as f32;
                        self.win_index += 1;
                        if self.win_index >= hwin {
                            self.win_index = 0;
                        }
                    }
                }
                _ => {
                    // In-pulse: zero samples until the pulses stop.
                    self.c_saved[self.index] = Complex32::new(0.0, 0.0);
                    if !is_pulse {
                        self.state = 0;
                        self.win_index = 1;
                    }
                }
            }

            self.index += 1;
            if self.index >= save_size {
                self.index = 0;
            }
        }
    }
}

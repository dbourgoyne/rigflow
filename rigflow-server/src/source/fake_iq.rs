use num_complex::Complex32;

use crate::source::IqSource;

/// Simple synthetic IQ source that generates a single complex tone.
///
/// Used for testing and debugging the DSP pipeline without hardware.
pub struct FakeIqSource {
    sample_rate_hz: f32,
    tone_hz: f32,
    phase: f32,
}

impl FakeIqSource {
    pub fn new(sample_rate_hz: f32, tone_hz: f32) -> Self {
        Self {
            sample_rate_hz,
            tone_hz,
            phase: 0.0,
        }
    }
}

impl IqSource for FakeIqSource {
    fn sample_rate(&self) -> f32 {
        self.sample_rate_hz
    }

    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String> {
        let phase_inc =
            2.0 * std::f32::consts::PI * self.tone_hz / self.sample_rate_hz;

        let mut out = Vec::with_capacity(max_samples);

        for _ in 0..max_samples {
            out.push(Complex32::new(self.phase.cos(), self.phase.sin()));

            self.phase += phase_inc;

            // Wrap phase into [-π, π] to avoid unbounded growth.
            if self.phase > std::f32::consts::PI {
                self.phase -= 2.0 * std::f32::consts::PI;
            }
        }

        Ok(out)
    }

    fn set_center_frequency(&mut self, _center_freq_hz: f32) -> Result<(), String> {
        // No-op: fake source does not tune.
        Ok(())
    }

    fn is_realtime(&self) -> bool {
        // This source generates data as fast as requested (not wall-clock bound).
        false
    }
}

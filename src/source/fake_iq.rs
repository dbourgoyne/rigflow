use num_complex::Complex32;
use std::f32::consts::PI;

use crate::source::IqSource;

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
        let phase_inc = 2.0 * PI * self.tone_hz / self.sample_rate_hz;
        let mut out = Vec::with_capacity(max_samples);

        for _ in 0..max_samples {
            out.push(Complex32::new(self.phase.cos(), self.phase.sin()));
            self.phase += phase_inc;

            if self.phase > PI {
                self.phase -= 2.0 * PI;
            }
        }

        Ok(out)
    }
}

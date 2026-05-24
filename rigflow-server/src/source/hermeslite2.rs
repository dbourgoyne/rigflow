use num_complex::Complex32;

use crate::source::IqSource;

/// Stub Hermes Lite 2 IQ source.
///
/// Returns silence until real Protocol 1 UDP communication is wired up in
/// step 3. The struct is intentionally minimal so it compiles cleanly.
pub struct HermesLite2Source {
    sample_rate_hz: f32,
    center_freq_hz: f32,
}

impl HermesLite2Source {
    pub fn new(sample_rate_hz: f32, center_freq_hz: f32) -> Self {
        Self {
            sample_rate_hz,
            center_freq_hz,
        }
    }
}

impl IqSource for HermesLite2Source {
    fn sample_rate(&self) -> f32 {
        self.sample_rate_hz
    }

    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String> {
        Ok(vec![Complex32::new(0.0, 0.0); max_samples])
    }

    fn set_center_frequency(&mut self, center_freq_hz: f32) -> Result<(), String> {
        self.center_freq_hz = center_freq_hz;
        Ok(())
    }

    fn is_realtime(&self) -> bool {
        true
    }
}

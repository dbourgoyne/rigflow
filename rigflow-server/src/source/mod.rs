use num_complex::Complex32;

pub mod fake;
pub mod factory;
pub mod rtlsdr;
pub mod wav;
pub mod wav_metadata;

pub trait IqSource {
    fn sample_rate(&self) -> f32;
    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String>;

    fn set_center_frequency(&mut self, _center_freq_hz: f32) -> Result<(), String> {
        Ok(())
    }

    fn is_realtime(&self) -> bool {
        false
    }
}

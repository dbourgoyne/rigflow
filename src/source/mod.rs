use num_complex::Complex32;

pub mod fake_iq;
pub mod factory;

pub trait IqSource: Send {
    fn sample_rate(&self) -> f32;
    fn read_block(&mut self, max_samples: usize) -> Result<Vec<Complex32>, String>;
}

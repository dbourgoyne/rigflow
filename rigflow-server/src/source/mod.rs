use num_complex::Complex32;

use rigflow_core::radio::source_control::{
    SourceCapabilities,
    SourceControlState,
    GainMode,
    DirectSamplingMode,
};

pub mod fake;
pub mod factory;
pub mod hermeslite2;
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

    // -----------------------------
    // NEW: capabilities + control
    // -----------------------------

    fn source_capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::none()
    }

    fn source_control_state(&self) -> SourceControlState {
        SourceControlState::default()
    }

    fn set_sample_rate(&mut self, _sample_rate_hz: u32) -> Result<(), String> {
        Ok(())
    }

    fn set_gain_mode(&mut self, _mode: GainMode) -> Result<(), String> {
        Ok(())
    }

    fn set_gain_db(&mut self, _gain_db: f32) -> Result<(), String> {
        Ok(())
    }

    fn set_ppm_correction(&mut self, _ppm: i32) -> Result<(), String> {
        Ok(())
    }

    fn set_direct_sampling(&mut self, _mode: DirectSamplingMode) -> Result<(), String> {
        Ok(())
    }

    /// Send a periodic keepalive to hardware that would otherwise time out.
    /// Default is a no-op; override for sources that require it (e.g. HL2).
    fn keepalive(&mut self) {}
}

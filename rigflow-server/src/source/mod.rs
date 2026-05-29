use num_complex::Complex32;

use rigflow_core::radio::source_control::{
    SourceCapabilities,
    SourceControlState,
    GainMode,
    DirectSamplingMode,
};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::tx_tune::{TxTuneResult, TxTuneStatus};

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

    /// Return the latest read-only telemetry from this source.
    ///
    /// Default returns an empty `SourceStatus` (all fields `None`).
    /// Override for sources that expose hardware telemetry (e.g. HL2).
    fn source_status(&self) -> SourceStatus {
        SourceStatus::default()
    }

    /// Perform a TX tune test (short carrier pulse for SWR measurement).
    ///
    /// Called from the capture thread, which owns the IQ source exclusively.
    ///
    /// # Safety contract for overrides
    ///
    /// - PTT MUST be released on all exit paths, including error paths.
    /// - Duration MUST be clamped to a safe maximum (≤ 500 ms).
    /// - Drive/amplitude MUST be clamped to a safe minimum.
    ///
    /// The default rejects the request with `"not_supported"`.
    fn tx_tune_test(
        &mut self,
        _center_freq_hz: u64,
        _duration_ms: u32,
        _drive: f32,
    ) -> TxTuneResult {
        TxTuneResult {
            status: TxTuneStatus::Fault,
            message: Some("not_supported".to_string()),
            ..TxTuneResult::default()
        }
    }
}

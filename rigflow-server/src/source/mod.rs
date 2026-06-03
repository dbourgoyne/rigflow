use num_complex::Complex32;

use rigflow_core::radio::source_control::{
    DirectSamplingMode, GainMode, SourceCapabilities, SourceControlState,
};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::tx_tune::{TxTuneResult, TxTuneStatus};

pub mod factory;
pub mod fake;
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

    /// Program the N2ADR HF filter board to the given 7-bit filter value.
    ///
    /// Default is a no-op (sources without an N2ADR board, e.g. RTL-SDR).
    /// HL2 overrides this to send the address-0 C&C with `C2 = value << 1`.
    fn set_n2adr_filter(&mut self, _value: u8) -> Result<(), String> {
        Ok(())
    }

    /// Send a periodic keepalive to hardware that would otherwise time out.
    /// Default is a no-op; override for sources that require it (e.g. HL2).
    fn keepalive(&mut self) {}

    /// Enable/disable FDX (TX Monitor Spectrum).
    ///
    /// When enabled, a source should retain the RX IQ it decodes during a
    /// transmit (`tx_tune_test`) so the worker can forward it into the RX DSP
    /// pipeline and keep the spectrum/waterfall live.  Default is a no-op
    /// (sources that cannot receive while transmitting); HL2 overrides it.
    fn set_fdx_enabled(&mut self, _enabled: bool) {}

    /// Drain and return any RX IQ captured during the most recent transmit
    /// while FDX was enabled.  Default returns empty (nothing captured).
    fn take_fdx_iq(&mut self) -> Vec<Complex32> {
        Vec::new()
    }

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
    ///
    /// `tx_drive_percent` (0–100) sets the HL2 drive register; `spot_level_percent`
    /// (0–100) sets the digital carrier IQ amplitude (`amplitude_fs = pct/100`).
    /// RF power ≈ drive × amplitude, matching Quisk's two-control Spot model.
    ///
    /// The default rejects the request with `"not_supported"`.
    fn tx_tune_test(
        &mut self,
        _target_freq_hz: u64,
        _duration_ms: u32,
        _tx_drive_percent: f32,
        _spot_level_percent: f32,
    ) -> TxTuneResult {
        TxTuneResult {
            status: TxTuneStatus::Fault,
            message: Some("not_supported".to_string()),
            ..TxTuneResult::default()
        }
    }
}

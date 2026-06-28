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
    // Multi-receiver (dual-watch VFO B)
    // -----------------------------

    /// Number of independent hardware receivers this source can stream at once.
    /// Default 1 (single RX); the HL2 returns 2 when its second DDC is usable.
    /// Dual-watch (a second VFO you can hear) requires `>= 2`.
    fn max_receivers(&self) -> u8 {
        1
    }

    /// Enable/disable the secondary hardware receiver (RX1).  Default no-op
    /// (single-RX sources).  HL2 overrides it to set the OpenHPSDR receiver-count
    /// field; while enabled, `read_block` deinterleaves RX1 samples that
    /// `read_secondary_block` then drains.
    fn set_secondary_receiver_enabled(&mut self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    /// Program the secondary receiver's NCO (its independent center frequency).
    /// Default no-op; HL2 overrides it (RX1 NCO register).  Independent of the
    /// primary `set_center_frequency`, so VFO B can sit on a different band.
    fn set_secondary_center_frequency(&mut self, _center_freq_hz: f32) -> Result<(), String> {
        Ok(())
    }

    /// Drain RX1 samples deinterleaved from the same packets as the most recent
    /// `read_block` (time-aligned, equal length).  Default empty (single-RX).
    fn read_secondary_block(&mut self) -> Vec<Complex32> {
        Vec::new()
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

    /// Install a shared "transmitting" flag the source sets while PTT is asserted
    /// (from the start of `tx_seq_begin` until `tx_seq_end` has released PTT).  The
    /// HR50 amplifier poller watches it to stay off the serial for the *entire*
    /// real keyed window — covering every TX path and any skew from the per-path
    /// software keying flags.  Default no-op (non-HL2 sources / no amplifier).
    fn set_tx_active_flag(&mut self, _flag: std::sync::Arc<std::sync::atomic::AtomicBool>) {}

    /// Drain and return any RX IQ captured during the most recent transmit
    /// while FDX was enabled.  Default returns empty (nothing captured).
    fn take_fdx_iq(&mut self) -> Vec<Complex32> {
        Vec::new()
    }

    /// Set the TX PTT sequencing lead/tail delays (ms).  Transmit paths assert
    /// PTT, wait `lead_ms`, emit RF, stop RF, wait `tail_ms`, then release PTT —
    /// so relay-based external amplifiers are never hot-switched.  Default no-op;
    /// HL2 overrides it.
    fn set_tx_sequencing(&mut self, _lead_ms: u32, _tail_ms: u32) {}

    /// Key CW (CW TX Phase 1): assert PTT, ramp the carrier up with a smooth
    /// envelope, sustain a complex CW tone at `pitch_hz` (USB above carrier, LSB
    /// below) while `key_held` is set, then ramp down and release PTT once it
    /// clears.  Amplitude from `spot_level_percent`, drive from
    /// `tx_drive_percent`.  RX IQ captured during keying is handed to `on_rx_iq`
    /// (FDX).  Default: not supported.
    #[allow(clippy::too_many_arguments)]
    fn tx_cw_key(
        &mut self,
        _target_freq_hz: u64,
        _pitch_hz: f32,
        _usb: bool,
        _tx_drive_percent: f32,
        _spot_level_percent: f32,
        _hang_ms: u32,
        _key_held: &std::sync::atomic::AtomicBool,
        _abort: &std::sync::atomic::AtomicBool,
        _on_rx_iq: &mut dyn FnMut(Vec<Complex32>),
    ) -> Result<(), String> {
        Err("not_supported".to_string())
    }

    /// Transmit SSB from a live microphone audio stream (SSB Mic TX Phase 3).
    /// Modulates pulled mono-f32 audio to a single-sideband complex baseband
    /// (USB above carrier, `usb=false` below) and keys the transmitter until
    /// `active` clears.  `pull_audio(n, out)` appends up to `n` mic samples
    /// (returns how many; fewer = underrun → pad silence).  `abort` (worker
    /// stop) forces release.  RX IQ goes to `on_rx_iq` (FDX).  When
    /// `limiter_enabled`, a soft peak limiter at `limiter_threshold` (0..1) runs
    /// before the modulator.  Default: not supported.
    #[allow(clippy::too_many_arguments)]
    fn tx_ssb_mic(
        &mut self,
        _target_freq_hz: u64,
        _usb: bool,
        _tx_drive_percent: f32,
        _limiter_enabled: bool,
        _limiter_threshold: f32,
        _compressor_enabled: bool,
        _compressor_level: u8,
        _active: &std::sync::atomic::AtomicBool,
        _abort: &std::sync::atomic::AtomicBool,
        _pull_audio: &mut dyn FnMut(usize, &mut Vec<f32>) -> usize,
        _on_rx_iq: &mut dyn FnMut(Vec<Complex32>),
    ) -> Result<(), String> {
        Err("not_supported".to_string())
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

    /// Transmit an open-ended SSB **test tone** (FDX Phase 2): a pure sine fed
    /// through the transmit path as a complex baseband signal, so the HL2 DUC
    /// places it above (`usb = true`) or below (`usb = false`) the carrier.
    ///
    /// Runs until `stop` is set.  Amplitude is `spot_level_percent / 100` (FS);
    /// drive comes from `tx_drive_percent`.  While running, RX IQ decoded during
    /// transmit is handed to `on_rx_iq` (used for FDX spectrum/waterfall) — this
    /// is the only output; audio is never touched.
    ///
    /// # Safety contract for overrides
    /// - PTT MUST be released on every exit path (normal stop, fault).
    /// - The drive register MUST be safed-off on exit.
    ///
    /// Default: not supported.
    fn tx_test_tone(
        &mut self,
        _target_freq_hz: u64,
        _tone_hz: f32,
        _usb: bool,
        _tx_drive_percent: f32,
        _spot_level_percent: f32,
        _stop: &std::sync::atomic::AtomicBool,
        _on_rx_iq: &mut dyn FnMut(Vec<Complex32>),
    ) -> Result<(), String> {
        Err("not_supported".to_string())
    }
}

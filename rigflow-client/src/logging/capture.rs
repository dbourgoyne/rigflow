//! Build a `rigflow_log::CapturedRadioState` from the live `UiState`.
//!
//! The TX frequency mirrors the server's authoritative
//! `effective_tx_freq_hz` (`rigflow-server` worker): split → the TX VFO's
//! target, plus XIT. The math is done in `f64` because `UiState` frequencies are
//! `f32` (exact only below ~16.7 MHz); the addition at least isn't further
//! degraded. For WSJT-X QSOs the exact ADIF frequency is preferred over this.

use rigflow_core::dsp::modes::DemodMode;
use rigflow_core::radio::vfo::VfoSelect;
use rigflow_log::{CapturedRadioState, Receiver};

use crate::ui::state::UiState;

/// Effective transmit frequency (Hz): `(split ? tx_vfo target : A target) + XIT`.
pub fn effective_tx_freq_hz(s: &UiState) -> u64 {
    let base = if s.split_enabled {
        match s.tx_vfo {
            VfoSelect::A => s.target_freq_hz,
            VfoSelect::B => s.vfo_b_target_freq_hz,
        }
    } else {
        s.target_freq_hz
    } as f64;
    let off = if s.xit_enabled {
        s.xit_offset_hz as f64
    } else {
        0.0
    };
    (base + off).round().max(0.0) as u64
}

/// Effective transmit mode: VFO B's mode on a split-to-B, else VFO A's.
pub fn effective_tx_mode(s: &UiState) -> DemodMode {
    if s.split_enabled && s.tx_vfo == VfoSelect::B {
        s.vfo_b_demod_mode
    } else {
        s.demod_mode
    }
}

/// Map a demod mode to its ADIF `MODE`. USB/LSB → SSB; CWU/CWL → CW; a data
/// mode (`DgtU`) defaults to FT8 (the entry form leaves it editable).
pub fn demod_to_adif_mode(m: DemodMode) -> &'static str {
    match m {
        DemodMode::Usb | DemodMode::Lsb => "SSB",
        DemodMode::Cwu | DemodMode::Cwl => "CW",
        DemodMode::Am => "AM",
        DemodMode::Nfm | DemodMode::Wfm => "FM",
        DemodMode::DgtU => "FT8",
    }
}

/// Default signal report for a mode (RST): 599 for CW, a nominal FT8 report for
/// data, 59 otherwise. A default only — editable in the entry form.
pub fn default_rst(m: DemodMode) -> &'static str {
    match m {
        DemodMode::Cwu | DemodMode::Cwl => "599",
        DemodMode::DgtU => "-05",
        _ => "59",
    }
}

/// Snapshot the live radio state for split-frequency derivation.
///
/// Receivers: VFO A is always present; VFO B is added under split or dual-watch.
/// `is_tx` is set from the effective-TX rule (never VFO-A-blindly) so the
/// `FREQ_RX` search excludes the transmit receiver.
pub fn capture_radio_state(s: &UiState) -> CapturedRadioState {
    let tx_freq_hz = effective_tx_freq_hz(s);
    let tx_mode = demod_to_adif_mode(effective_tx_mode(s)).to_string();

    let mut receivers = Vec::with_capacity(2);
    let a_is_tx = !s.split_enabled || s.tx_vfo == VfoSelect::A;
    receivers.push(Receiver {
        freq_hz: s.target_freq_hz.max(0.0) as u64,
        is_tx: a_is_tx,
    });
    if s.split_enabled || s.dual_watch_enabled {
        let b_is_tx = s.split_enabled && s.tx_vfo == VfoSelect::B;
        receivers.push(Receiver {
            freq_hz: s.vfo_b_target_freq_hz.max(0.0) as u64,
            is_tx: b_is_tx,
        });
    }

    CapturedRadioState {
        tx_freq_hz,
        tx_mode,
        split_active: s.split_enabled,
        receivers,
    }
}

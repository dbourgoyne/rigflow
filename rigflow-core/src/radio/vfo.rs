use serde::{Deserialize, Serialize};

use crate::dsp::modes::{DeemphasisMode, DemodMode, Sideband};

/// A complete, self-contained snapshot of one VFO's receiver settings.
///
/// This encapsulates everything that makes a VFO "what it is" — frequency,
/// demodulation, filtering, and the DSP utilities — into a single `Clone`-able,
/// serializable value, so a whole VFO can be copied wholesale (`vfo_b =
/// vfo_a.clone()`) rather than field-by-field.  Read-only telemetry (S-meter)
/// is intentionally excluded: it is measured, not a setting.  Volume is included
/// because it is part of the receiver's audio state (it is applied client-side).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VfoState {
    /// Tuned frequency (the signal you hear).
    pub target_freq_hz: u64,
    /// LO / centre frequency (the receiver's hardware NCO; the panadapter window
    /// centre).  `target` sits at an offset within the window centred here.
    pub center_freq_hz: u64,
    pub demod_mode: DemodMode,
    pub sideband: Sideband,
    pub filter_bandwidth_hz: f32,
    pub ssb_pitch_hz: f32,
    pub cw_pitch_hz: f32,
    pub deemphasis_mode: DeemphasisMode,
    pub squelch_enabled: bool,
    pub squelch_threshold_db: f32,
    pub nr2_enabled: bool,
    pub nr2_strength: f32,
    pub nb_enabled: bool,
    pub nb_threshold: f32,
    pub notch_auto_enabled: bool,
    pub agc_enabled: bool,
    pub agc_strength: f32,
    /// RIT (receive increment tuning): a small RX-only offset.
    pub rit_enabled: bool,
    pub rit_offset_hz: i32,
    /// Receive-audio volume in percent (0–100); applied client-side.
    pub volume_percent: u8,
}

impl Default for VfoState {
    fn default() -> Self {
        Self {
            target_freq_hz: 0,
            center_freq_hz: 0,
            demod_mode: DemodMode::Usb,
            sideband: Sideband::Usb,
            filter_bandwidth_hz: 2700.0,
            ssb_pitch_hz: 0.0,
            cw_pitch_hz: 600.0,
            deemphasis_mode: DeemphasisMode::Off,
            squelch_enabled: false,
            squelch_threshold_db: -90.0,
            nr2_enabled: false,
            nr2_strength: 0.5,
            nb_enabled: false,
            nb_threshold: 0.5,
            notch_auto_enabled: false,
            agc_enabled: true,
            agc_strength: 0.5,
            rit_enabled: false,
            rit_offset_hz: 0,
            volume_percent: 50,
        }
    }
}

/// Which VFO a control or transmit refers to, for dual-VFO / split operation.
///
/// `A` is the primary VFO (the single-VFO mirror that all pre-dual-watch code
/// uses); `B` is the secondary VFO (independent frequency + mode, fed by the
/// source's second hardware receiver when dual-watch is active).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VfoSelect {
    #[default]
    A,
    B,
}

impl VfoSelect {
    /// The other VFO (for A↔B swap / "the receiving VFO is the non-TX one").
    pub fn other(self) -> Self {
        match self {
            VfoSelect::A => VfoSelect::B,
            VfoSelect::B => VfoSelect::A,
        }
    }
}

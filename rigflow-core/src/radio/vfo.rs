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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfo_select_default_is_a() {
        assert_eq!(VfoSelect::default(), VfoSelect::A);
    }

    #[test]
    fn vfo_select_other_swaps_a_and_b() {
        assert_eq!(VfoSelect::A.other(), VfoSelect::B);
        assert_eq!(VfoSelect::B.other(), VfoSelect::A);
    }

    #[test]
    fn vfo_select_other_is_an_involution() {
        // Applying `other()` twice must return to the start — this is what makes
        // it safe to call unconditionally (e.g. on every TX/RX toggle) without
        // tracking parity separately.
        for select in [VfoSelect::A, VfoSelect::B] {
            assert_eq!(select.other().other(), select);
        }
    }

    #[test]
    fn vfo_select_serde_wire_format_is_stable() {
        // Crosses the WebSocket boundary alongside VfoState — pinned the same way
        // as DemodMode/Sideband in dsp/modes.rs.
        assert_eq!(serde_json::to_string(&VfoSelect::A).unwrap(), "\"a\"");
        assert_eq!(serde_json::to_string(&VfoSelect::B).unwrap(), "\"b\"");
    }

    #[test]
    fn default_vfo_state_matches_documented_values() {
        let state = VfoState::default();
        assert_eq!(state.target_freq_hz, 0);
        assert_eq!(state.center_freq_hz, 0);
        assert_eq!(state.demod_mode, DemodMode::Usb);
        assert_eq!(state.sideband, Sideband::Usb);
        assert_eq!(state.filter_bandwidth_hz, 2700.0);
        assert_eq!(state.ssb_pitch_hz, 0.0);
        assert_eq!(state.cw_pitch_hz, 600.0);
        assert_eq!(state.deemphasis_mode, DeemphasisMode::Off);
        assert!(!state.squelch_enabled);
        assert_eq!(state.squelch_threshold_db, -90.0);
        assert!(!state.nr2_enabled);
        assert_eq!(state.nr2_strength, 0.5);
        assert!(!state.nb_enabled);
        assert_eq!(state.nb_threshold, 0.5);
        assert!(!state.notch_auto_enabled);
        // AGC defaults on; everything else defaults off — a fresh VFO should be
        // usable immediately without the user having to find and enable AGC first.
        assert!(state.agc_enabled);
        assert_eq!(state.agc_strength, 0.5);
        assert!(!state.rit_enabled);
        assert_eq!(state.rit_offset_hz, 0);
        assert_eq!(state.volume_percent, 50);
    }

    #[test]
    fn vfo_state_serde_round_trip_preserves_all_fields() {
        // A round trip goes out and back through the same struct, so it agrees
        // with itself regardless of what the wire keys are actually called — a
        // rename on both sides simultaneously would still pass this. What it
        // does catch: a field silently dropped or made lossy by a `#[serde]`
        // attribute (e.g. `#[serde(skip)]`), where the value that comes back
        // no longer matches what went in. See `vfo_state_wire_keys_are_stable`
        // below for the complementary check that pins the actual key names.
        let mut state = VfoState {
            target_freq_hz: 14_074_000,
            center_freq_hz: 14_070_000,
            rit_enabled: true,
            rit_offset_hz: -150,
            ..VfoState::default()
        };
        state.demod_mode = DemodMode::DgtU;
        state.sideband = Sideband::Lsb;

        let json = serde_json::to_string(&state).unwrap();
        let round_tripped: VfoState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, round_tripped);
    }

    #[test]
    fn vfo_state_wire_keys_are_stable() {
        // Deserialize from a fixed literal: these key names cross the WebSocket
        // boundary, so renaming one is a breaking protocol change. Unlike the
        // round-trip test above, this fails if a key is renamed on only one
        // side — and if a new field is ever added without `#[serde(default)]`,
        // since a literal with no `serde(default)` fallback for it won't parse.
        let json = r#"{"target_freq_hz":14074000,"center_freq_hz":14070000,
            "demod_mode":"usb","sideband":"Usb","filter_bandwidth_hz":2700.0,
            "ssb_pitch_hz":0.0,"cw_pitch_hz":600.0,"deemphasis_mode":"off",
            "squelch_enabled":false,"squelch_threshold_db":-90.0,
            "nr2_enabled":false,"nr2_strength":0.5,"nb_enabled":false,
            "nb_threshold":0.5,"notch_auto_enabled":false,"agc_enabled":true,
            "agc_strength":0.5,"rit_enabled":false,"rit_offset_hz":0,
            "volume_percent":50}"#;
        let parsed: VfoState = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            VfoState {
                target_freq_hz: 14_074_000,
                center_freq_hz: 14_070_000,
                ..VfoState::default()
            }
        );
    }
}

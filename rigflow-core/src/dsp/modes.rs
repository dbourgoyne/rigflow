use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported demodulation modes.
///
/// These are shared between:
/// - client UI
/// - server DSP pipeline
/// - protocol layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemodMode {
    /// Wideband FM (broadcast FM)
    Wfm,

    /// Narrowband FM (two-way radio, etc.)
    Nfm,

    /// Upper Sideband
    Usb,

    /// Lower Sideband
    Lsb,

    /// AM
    Am,

    /// CW, upper sideband (tone above the dial: RF = dial + pitch).
    /// `alias = "cw"` maps legacy persisted/bookmark `"cw"` to CWU.
    #[serde(alias = "cw")]
    Cwu,

    /// CW, lower sideband (tone below the dial: RF = dial − pitch).
    Cwl,

    /// Data / digital USB (FT8, JS8, fldigi, …).  Identical to `Usb` at the
    /// RF/DSP level; distinct so it carries its own filter preset + label,
    /// reports the data mode over CAT (`PKTUSB`), and auto-routes RX audio.
    DgtU,
}

impl fmt::Display for DemodMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DemodMode::Wfm => "wfm",
            DemodMode::Nfm => "nfm",
            DemodMode::Usb => "usb",
            DemodMode::Lsb => "lsb",
            DemodMode::Am => "am",
            DemodMode::Cwu => "cwu",
            DemodMode::Cwl => "cwl",
            DemodMode::DgtU => "dgt_u",
        };
        write!(f, "{}", s)
    }
}

use std::str::FromStr;

impl FromStr for DemodMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wfm" => Ok(DemodMode::Wfm),
            "nfm" => Ok(DemodMode::Nfm),
            "usb" => Ok(DemodMode::Usb),
            "lsb" => Ok(DemodMode::Lsb),
            "am" => Ok(DemodMode::Am),
            "cwu" => Ok(DemodMode::Cwu),
            "cwl" => Ok(DemodMode::Cwl),
            "dgt_u" => Ok(DemodMode::DgtU),
            // Legacy single CW maps to CWU.
            "cw" => Ok(DemodMode::Cwu),
            _ => Err(format!("invalid demod mode: {}", s)),
        }
    }
}

/// Sideband selection for SSB demodulation.
///
/// This is separate from `DemodMode` because:
/// - SSB processing may need sideband independently
/// - pipeline stages may operate on sideband directly
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sideband {
    Usb,
    Lsb,
}

impl fmt::Display for Sideband {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Sideband::Usb => "usb",
            Sideband::Lsb => "lsb",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for Sideband {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "usb" => Ok(Sideband::Usb),
            "lsb" => Ok(Sideband::Lsb),
            _ => Err(format!("invalid sideband: {}", s)),
        }
    }
}

pub struct PitchUiConfig {
    pub min_hz: f32,
    pub max_hz: f32,
    pub default_hz: f32,
    pub label: &'static str,
    pub debounce_delta_hz: f32,
    pub debounce_interval_ms: u64,
}

pub fn pitch_limits(mode: DemodMode) -> Option<PitchUiConfig> {
    match mode {
        DemodMode::Usb | DemodMode::Lsb => Some(PitchUiConfig {
            min_hz: -1500.0,
            max_hz: 1500.0,
            default_hz: 0.0,
            label: "SSB Pitch (Hz)",
            debounce_delta_hz: 5.0,
            debounce_interval_ms: 40,
        }),
        DemodMode::Cwu | DemodMode::Cwl => Some(PitchUiConfig {
            min_hz: 300.0,
            max_hz: 1200.0,
            default_hz: 600.0,
            label: "CW Pitch (Hz)",
            debounce_delta_hz: 10.0,
            debounce_interval_ms: 50,
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BandwidthLimits {
    pub min_hz: f32,
    pub max_hz: f32,
    pub default_hz: f32,
}

pub fn filter_bandwidth_limits(mode: DemodMode) -> BandwidthLimits {
    match mode {
        DemodMode::Usb | DemodMode::Lsb => BandwidthLimits {
            min_hz: 300.0,
            max_hz: 4000.0,
            default_hz: 2700.0,
        },
        // Data/digital USB: wider default suited to FT8 & friends.
        DemodMode::DgtU => BandwidthLimits {
            min_hz: 300.0,
            max_hz: 4000.0,
            default_hz: 3000.0,
        },
        DemodMode::Cwu | DemodMode::Cwl => BandwidthLimits {
            min_hz: 100.0,
            max_hz: 1500.0,
            default_hz: 500.0,
        },
        DemodMode::Am => BandwidthLimits {
            min_hz: 1000.0,
            max_hz: 10000.0,
            default_hz: 5000.0,
        },
        DemodMode::Nfm => BandwidthLimits {
            min_hz: 1500.0,
            max_hz: 8000.0,
            default_hz: 4000.0,
        },
        DemodMode::Wfm => BandwidthLimits {
            min_hz: 5000.0,
            max_hz: 20000.0,
            default_hz: 15000.0,
        },
    }
}

pub fn clamp_filter_bandwidth(mode: DemodMode, hz: f32) -> f32 {
    let limits = filter_bandwidth_limits(mode);
    hz.clamp(limits.min_hz, limits.max_hz)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeemphasisMode {
    Off,
    Tau50us,
    Tau75us,
}

impl DeemphasisMode {
    pub fn label(self) -> &'static str {
        match self {
            DeemphasisMode::Off => "Off",
            DeemphasisMode::Tau50us => "50 µs",
            DeemphasisMode::Tau75us => "75 µs",
        }
    }

    pub fn tau_seconds(self) -> Option<f32> {
        match self {
            DeemphasisMode::Off => None,
            DeemphasisMode::Tau50us => Some(50e-6),
            DeemphasisMode::Tau75us => Some(75e-6),
        }
    }
}

pub fn default_deemphasis_mode(mode: DemodMode) -> Option<DeemphasisMode> {
    match mode {
        DemodMode::Wfm => Some(DeemphasisMode::Tau75us),
        DemodMode::Nfm => Some(DeemphasisMode::Tau75us),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demod_mode_display_round_trips_through_from_str() {
        for mode in [
            DemodMode::Wfm,
            DemodMode::Nfm,
            DemodMode::Usb,
            DemodMode::Lsb,
            DemodMode::Am,
            DemodMode::Cwu,
            DemodMode::Cwl,
            DemodMode::DgtU,
        ] {
            let s = mode.to_string();
            assert_eq!(
                DemodMode::from_str(&s),
                Ok(mode),
                "round trip failed for {s}"
            );
        }
    }

    #[test]
    fn demod_mode_from_str_rejects_unknown() {
        assert!(DemodMode::from_str("bogus").is_err());
    }

    #[test]
    fn demod_mode_legacy_cw_alias_maps_to_cwu() {
        // Legacy persisted/bookmark value "cw" (pre-CWU/CWL split) must keep
        // resolving to CWU, both via FromStr and via serde's #[serde(alias = "cw")].
        assert_eq!(DemodMode::from_str("cw"), Ok(DemodMode::Cwu));

        let de: DemodMode = serde_json::from_str("\"cw\"").unwrap();
        assert_eq!(de, DemodMode::Cwu);
    }

    #[test]
    fn sideband_display_round_trips_through_from_str() {
        for sb in [Sideband::Usb, Sideband::Lsb] {
            let s = sb.to_string();
            assert_eq!(Sideband::from_str(&s), Ok(sb));
        }
    }

    #[test]
    fn sideband_from_str_rejects_unknown() {
        assert!(Sideband::from_str("bogus").is_err());
    }

    #[test]
    fn pitch_limits_present_for_ssb_and_cw_only() {
        assert!(pitch_limits(DemodMode::Usb).is_some());
        assert!(pitch_limits(DemodMode::Lsb).is_some());
        assert!(pitch_limits(DemodMode::Cwu).is_some());
        assert!(pitch_limits(DemodMode::Cwl).is_some());
        assert!(pitch_limits(DemodMode::Am).is_none());
        assert!(pitch_limits(DemodMode::Nfm).is_none());
        assert!(pitch_limits(DemodMode::Wfm).is_none());
        assert!(pitch_limits(DemodMode::DgtU).is_none());
    }

    #[test]
    fn pitch_limits_default_is_within_its_own_range() {
        for mode in [
            DemodMode::Usb,
            DemodMode::Lsb,
            DemodMode::Cwu,
            DemodMode::Cwl,
        ] {
            let cfg = pitch_limits(mode).unwrap();
            assert!(
                cfg.default_hz >= cfg.min_hz && cfg.default_hz <= cfg.max_hz,
                "default_hz out of range for {mode}"
            );
        }
    }

    #[test]
    fn filter_bandwidth_limits_default_is_within_its_own_range_for_every_mode() {
        for mode in [
            DemodMode::Wfm,
            DemodMode::Nfm,
            DemodMode::Usb,
            DemodMode::Lsb,
            DemodMode::Am,
            DemodMode::Cwu,
            DemodMode::Cwl,
            DemodMode::DgtU,
        ] {
            let limits = filter_bandwidth_limits(mode);
            assert!(
                limits.default_hz >= limits.min_hz && limits.default_hz <= limits.max_hz,
                "default_hz out of range for {mode}"
            );
        }
    }

    #[test]
    fn clamp_filter_bandwidth_clamps_to_mode_limits() {
        let limits = filter_bandwidth_limits(DemodMode::Usb);
        assert_eq!(
            clamp_filter_bandwidth(DemodMode::Usb, limits.min_hz - 100.0),
            limits.min_hz
        );
        assert_eq!(
            clamp_filter_bandwidth(DemodMode::Usb, limits.max_hz + 100.0),
            limits.max_hz
        );
        assert_eq!(
            clamp_filter_bandwidth(DemodMode::Usb, limits.default_hz),
            limits.default_hz
        );
    }

    #[test]
    fn deemphasis_tau_seconds_matches_label_semantics() {
        assert_eq!(DeemphasisMode::Off.tau_seconds(), None);
        assert_eq!(DeemphasisMode::Tau50us.tau_seconds(), Some(50e-6));
        assert_eq!(DeemphasisMode::Tau75us.tau_seconds(), Some(75e-6));
    }

    #[test]
    fn default_deemphasis_only_set_for_fm_modes() {
        assert_eq!(
            default_deemphasis_mode(DemodMode::Wfm),
            Some(DeemphasisMode::Tau75us)
        );
        assert_eq!(
            default_deemphasis_mode(DemodMode::Nfm),
            Some(DeemphasisMode::Tau75us)
        );
        for mode in [
            DemodMode::Usb,
            DemodMode::Lsb,
            DemodMode::Am,
            DemodMode::Cwu,
            DemodMode::Cwl,
            DemodMode::DgtU,
        ] {
            assert_eq!(
                default_deemphasis_mode(mode),
                None,
                "unexpected deemphasis for {mode}"
            );
        }
    }
}

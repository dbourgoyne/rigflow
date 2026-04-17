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

    /// CW
    Cw,
}

impl fmt::Display for DemodMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
	    DemodMode::Wfm => "wfm",
	    DemodMode::Nfm => "nfm",
	    DemodMode::Usb => "usb",
	    DemodMode::Lsb => "lsb",
	    DemodMode::Am  => "am",
	    DemodMode::Cw  => "cw",
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
	    "cw" => Ok(DemodMode::Cw),
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
        DemodMode::Cw => BandwidthLimits {
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

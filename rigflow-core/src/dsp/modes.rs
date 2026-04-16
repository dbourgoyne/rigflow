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

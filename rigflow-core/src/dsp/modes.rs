use serde::{Deserialize, Serialize};

/// Supported demodulation modes.
///
/// These are shared between:
/// - client UI
/// - server DSP pipeline
/// - protocol layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemodMode {
    /// Wideband FM (broadcast FM)
    Wfm,

    /// Narrowband FM (two-way radio, etc.)
    Nfm,

    /// Upper Sideband
    Usb,

    /// Lower Sideband
    Lsb,
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

/// Convert a `DemodMode` to a lowercase string.
///
/// Used for:
/// - logging
/// - UI display
/// - protocol/debug output
pub fn demod_mode_to_string(mode: DemodMode) -> String {
    match mode {
        DemodMode::Wfm => "wfm".to_string(),
        DemodMode::Nfm => "nfm".to_string(),
        DemodMode::Usb => "usb".to_string(),
        DemodMode::Lsb => "lsb".to_string(),
    }
}

/// Convert a `Sideband` to a lowercase string.
pub fn sideband_to_string(sideband: Sideband) -> String {
    match sideband {
        Sideband::Usb => "usb".to_string(),
        Sideband::Lsb => "lsb".to_string(),
    }
}

/// Parse a demodulation mode from a string.
///
/// Accepted values (case-insensitive):
/// - "wfm", "fm"
/// - "nfm"
/// - "usb"
/// - "lsb"
///
/// Returns:
/// - `Ok(DemodMode)` if valid
/// - `Err(String)` if invalid
pub fn parse_demod_mode(s: &str) -> Result<DemodMode, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "wfm" | "fm" => Ok(DemodMode::Wfm),
        "nfm" => Ok(DemodMode::Nfm),
        "usb" => Ok(DemodMode::Usb),
        "lsb" => Ok(DemodMode::Lsb),
        _ => Err(format!("invalid demod mode: '{}'", s)),
    }
}

/// Parse a sideband from a string.
///
/// Accepted values (case-insensitive):
/// - "usb"
/// - "lsb"
pub fn parse_sideband(s: &str) -> Result<Sideband, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "usb" => Ok(Sideband::Usb),
        "lsb" => Ok(Sideband::Lsb),
        _ => Err(format!("invalid sideband: '{}'", s)),
    }
}

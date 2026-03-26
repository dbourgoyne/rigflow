use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemodMode {
    Wfm,
    Usb,
    Lsb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sideband {
    Usb,
    Lsb,
}

pub fn demod_mode_to_string(mode: DemodMode) -> String {
    match mode {
        DemodMode::Wfm => "wfm".to_string(),
        DemodMode::Usb => "usb".to_string(),
        DemodMode::Lsb => "lsb".to_string(),
    }
}

pub fn sideband_to_string(sideband: Sideband) -> String {
    match sideband {
        Sideband::Usb => "usb".to_string(),
        Sideband::Lsb => "lsb".to_string(),
    }
}

pub fn parse_demod_mode(s: &str) -> Result<DemodMode, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "wfm" | "fm" => Ok(DemodMode::Wfm),
        "usb" => Ok(DemodMode::Usb),
        "lsb" => Ok(DemodMode::Lsb),
        _ => Err(format!("invalid demod mode: '{}'", s)),
    }
}

pub fn parse_sideband(s: &str) -> Result<Sideband, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "usb" => Ok(Sideband::Usb),
        "lsb" => Ok(Sideband::Lsb),
        _ => Err(format!("invalid sideband: '{}'", s)),
    }
}

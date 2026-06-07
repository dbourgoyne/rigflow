//! Generic, amplifier-agnostic status model.
//!
//! Read-only telemetry from an attached RF power amplifier (Phase 1: Hardrock-50).
//! Kept deliberately generic so future amplifier models slot in without changing
//! the protocol or the UI: the UI keys off `model` (a top-level "Amplifier: …"
//! row) and shows the detail fields only when a model is present.

use serde::{Deserialize, Serialize};

/// Which amplifier model is attached.  New models are added here; the UI and
/// protocol need no changes beyond a `label()` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmplifierModel {
    /// Hardrock-50 (HR50 / HF50).
    Hr50,
}

impl AmplifierModel {
    /// Short label for the top-level UI row, e.g. "Amplifier: HR50".
    pub fn label(&self) -> &'static str {
        match self {
            AmplifierModel::Hr50 => "HR50",
        }
    }
}

/// Amplifier keying mode (Phase 2 control).  HR50: OFF/PTT/COR/QRP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmplifierKeyingMode {
    Off,
    Ptt,
    Cor,
    Qrp,
}

impl AmplifierKeyingMode {
    pub fn label(&self) -> &'static str {
        match self {
            AmplifierKeyingMode::Off => "OFF",
            AmplifierKeyingMode::Ptt => "PTT",
            AmplifierKeyingMode::Cor => "COR",
            AmplifierKeyingMode::Qrp => "QRP",
        }
    }

    /// HR50 `HRMDx;` numeric code.
    pub fn hr50_code(&self) -> u8 {
        match self {
            AmplifierKeyingMode::Off => 0,
            AmplifierKeyingMode::Ptt => 1,
            AmplifierKeyingMode::Cor => 2,
            AmplifierKeyingMode::Qrp => 3,
        }
    }

    /// Parse the keying-mode string the amp reports in `HRRX;` (mode field).
    pub fn from_label(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "OFF" => Some(AmplifierKeyingMode::Off),
            "PTT" => Some(AmplifierKeyingMode::Ptt),
            "COR" => Some(AmplifierKeyingMode::Cor),
            "QRP" => Some(AmplifierKeyingMode::Qrp),
            _ => None,
        }
    }

    /// All modes, for UI selectors.
    pub const ALL: [AmplifierKeyingMode; 4] = [
        AmplifierKeyingMode::Off,
        AmplifierKeyingMode::Ptt,
        AmplifierKeyingMode::Cor,
        AmplifierKeyingMode::Qrp,
    ];
}

/// Amplifier ATU engagement mode (Phase 2 control).  HR50: bypass/active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmplifierAtuMode {
    Bypass,
    Active,
}

impl AmplifierAtuMode {
    pub fn label(&self) -> &'static str {
        match self {
            AmplifierAtuMode::Bypass => "Bypass",
            AmplifierAtuMode::Active => "Active",
        }
    }

    /// HR50 `HRATx;` numeric code (1=bypass, 2=active; 0 = not present).
    pub fn hr50_code(&self) -> u8 {
        match self {
            AmplifierAtuMode::Bypass => 1,
            AmplifierAtuMode::Active => 2,
        }
    }

    /// Map an `HRATx;` reply code to a mode (0 = not present → `None`).
    pub fn from_hr50_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(AmplifierAtuMode::Bypass),
            2 => Some(AmplifierAtuMode::Active),
            _ => None,
        }
    }
}

/// Read-only amplifier telemetry, carried in the runtime state to the client.
///
/// `model == None` means **no amplifier detected** → the UI shows
/// "Amplifier: None" and hides every detail field.  When a model is present the
/// detail fields are populated as the amplifier reports them (each independently
/// optional, so a momentary read failure on one field doesn't blank the rest).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AmplifierStatus {
    /// Detected amplifier model, or `None` when no amplifier is present.
    pub model: Option<AmplifierModel>,

    /// Keying mode as reported by the amplifier (e.g. "OFF", "PTT", "COR", "QRP").
    pub mode: Option<String>,

    /// Operating band as reported (e.g. "20M").
    pub band: Option<String>,

    /// Heatsink temperature in degrees Celsius.  Normalized to °C regardless of
    /// the amplifier's own display setting (it may report °F).
    pub temperature_c: Option<f32>,

    /// DC input voltage in volts.
    pub voltage_v: Option<f32>,

    /// Last transmission peak envelope power, watts (HR50 `HRMX`).
    pub tx_pep_w: Option<f32>,
    /// Last transmission average forward power, watts (HR50 `HRMX`).
    pub tx_avg_w: Option<f32>,
    /// Last transmission SWR (HR50 `HRMX`); `None` when power was too low to measure.
    pub tx_swr: Option<f32>,

    /// Whether an automatic antenna tuner is installed (HR50 `HRAT` ≠ 0).
    pub atu_present: bool,
    /// Current ATU engagement mode, when an ATU is present.
    pub atu_mode: Option<AmplifierAtuMode>,

    /// Last communication/parse error (diagnostic only; not shown as a field).
    pub last_error: Option<String>,
}

impl AmplifierStatus {
    /// True when an amplifier is currently detected.
    pub fn is_present(&self) -> bool {
        self.model.is_some()
    }
}

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

    /// Last communication/parse error (diagnostic only; not shown as a field).
    pub last_error: Option<String>,
}

impl AmplifierStatus {
    /// True when an amplifier is currently detected.
    pub fn is_present(&self) -> bool {
        self.model.is_some()
    }
}

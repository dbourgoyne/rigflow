use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    SetFrequency { target_freq_hz: f32 },
    SetCenterFrequency { center_freq_hz: f32 },
    SetSideband { sideband: String },
    SetDemodMode { mode: String },
    SetSsbPitch { pitch_hz: f32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Error { message: String },
}

pub mod radio_control;

pub use radio_control::{
    ClientRadioMessage, RadioAvailability, RadioInfo, ServerRadioMessage,
};

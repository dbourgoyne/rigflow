use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    SetFrequency { target_freq_hz: f32 },
    SetSideband { sideband: String },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Ready,
    Pong,
    FrequencyChanged { target_freq_hz: f32 },
    SidebandChanged { sideband: String },
    AudioFrame { samples: Vec<f32> },
    Info { message: String },
    Error { message: String },
}

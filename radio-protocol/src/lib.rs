use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    SetFrequency { target_freq_hz: f32 },
    SetCenterFrequency { center_freq_hz: f32 },
    SetSideband { sideband: String },
    SetDemodMode { mode: String },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Ready,
    Pong,
    FrequencyChanged { target_freq_hz: f32 },
    CenterFrequencyChanged { center_freq_hz: f32 },
    SidebandChanged { sideband: String },
    DemodModeChanged { mode: String },
    StreamConfig {
        audio_sample_rate_hz: f32,
        audio_format: String,
        waterfall_bins: usize,
        waterfall_frame_rate_hz: f32,
        center_freq_hz: f32,
        target_freq_hz: f32,
        input_sample_rate_hz: f32,
    },
    UdpAudioOffer { server_udp_port: u16 },
    Info { message: String },
    Error { message: String },
}

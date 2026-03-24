use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ServerMessage {
    Ready,
    StreamConfig {
        audio_sample_rate_hz: f32,
        audio_format: String,
        waterfall_bins: usize,
        waterfall_frame_rate_hz: f32,
        center_freq_hz: f32,
        target_freq_hz: f32,
        input_sample_rate_hz: f32,
    },
    FrequencyChanged {
        target_freq_hz: f32,
    },
    CenterFrequencyChanged {
        center_freq_hz: f32,
    },
    DemodModeChanged {
        mode: String,
    },
    SidebandChanged {
        sideband: String,
    },
    UdpAudioOffer {
        server_udp_port: u16,
    },
    Error {
        message: String,
    },
    Info {
        message: String,
    },
    Pong,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ClientMessage {
    SetFrequency { target_freq_hz: f32 },
    SetCenterFrequency { center_freq_hz: f32 },
    SetDemodMode { mode: String },
    SetSideband { sideband: String },
    Ping,
}

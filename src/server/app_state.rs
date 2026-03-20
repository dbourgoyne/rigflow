use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

use crate::api::protocol::ServerMessage;
use crate::dsp::demod::Sideband;

#[derive(Debug)]
pub struct RadioState {
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub sideband: Sideband,
}

impl RadioState {
    pub fn new(center_freq_hz: f32, target_freq_hz: f32, sideband: Sideband) -> Self {
        Self {
            center_freq_hz,
            target_freq_hz,
            sideband,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub radio: Arc<RwLock<RadioState>>,
    pub tx: broadcast::Sender<ServerMessage>,
    pub audio_tx: broadcast::Sender<Vec<u8>>,
}

impl AppState {
    pub fn new(center_freq_hz: f32, target_freq_hz: f32, sideband: Sideband) -> Self {
        let (tx, _) = broadcast::channel(256);
        let (audio_tx, _) = broadcast::channel(256);

        Self {
            radio: Arc::new(RwLock::new(RadioState::new(
                center_freq_hz,
                target_freq_hz,
                sideband,
            ))),
            tx,
            audio_tx,
        }
    }
}

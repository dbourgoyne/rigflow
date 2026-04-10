use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use rigflow_protocol::ServerMessage;
use crate::dsp::demod::{DemodMode, Sideband};
use crate::server::config::{WATERFALL_BINS, WATERFALL_FRAME_RATE_HZ};
use crate::server::radio_manager::RadioManager;

#[derive(Clone)]
pub struct AppState {
    pub tx: broadcast::Sender<ServerMessage>,
    pub audio_tx: broadcast::Sender<Vec<u8>>,
    pub waterfall_tx: broadcast::Sender<Vec<u8>>,
    pub udp_audio_target: Arc<RwLock<Option<SocketAddr>>>,
    pub radio_manager: Arc<RadioManager>,
}

impl AppState {
    pub fn new(
        center_freq_hz: f32,
        target_freq_hz: f32,
        sideband: Sideband,
        demod_mode: DemodMode,
        ssb_pitch_hz: f32,
	radio_manager: Arc<RadioManager>,
    ) -> Self {
        let (tx, _) = broadcast::channel(256);
        let (audio_tx, _) = broadcast::channel(256);
        let (waterfall_tx, _) = broadcast::channel(256);

        Self {
            tx,
            audio_tx,
            waterfall_tx,
            udp_audio_target: Arc::new(RwLock::new(None)),
	    radio_manager,
        }
    }
}

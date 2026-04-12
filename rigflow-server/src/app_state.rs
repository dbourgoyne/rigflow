use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use rigflow_protocol::ServerMessage;

use crate::server::radio_manager::RadioManager;

/// Shared application state for the WebSocket/API layer.
///
/// This holds the broadcast channels used to fan out legacy messages, audio,
/// and waterfall payloads, along with shared access to the radio manager.
#[derive(Clone)]
pub struct AppState {
    pub tx: broadcast::Sender<ServerMessage>,
    pub audio_tx: broadcast::Sender<Vec<u8>>,
    pub waterfall_tx: broadcast::Sender<Vec<u8>>,
    pub udp_audio_target: Arc<RwLock<Option<SocketAddr>>>,
    pub radio_manager: Arc<RadioManager>,
}

impl AppState {
    pub fn new(radio_manager: Arc<RadioManager>) -> Self {
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

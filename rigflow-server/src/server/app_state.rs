use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use rigflow_protocol::ServerMessage;
use crate::dsp::demod::{DemodMode, Sideband};
use crate::server::control::RadioCommand;
use crate::server::config::{WATERFALL_BINS, WATERFALL_FRAME_RATE_HZ};
use crate::server::radio_manager::RadioManager;

#[derive(Debug)]
pub struct RadioState {
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub sideband: Sideband,
    pub demod_mode: DemodMode,
    pub ssb_pitch_hz: f32,
}

impl RadioState {
    pub fn new(
        center_freq_hz: f32,
        target_freq_hz: f32,
        sideband: Sideband,
        demod_mode: DemodMode,
        ssb_pitch_hz: f32,
    ) -> Self {
        Self {
            center_freq_hz,
            target_freq_hz,
            sideband,
            demod_mode,
            ssb_pitch_hz,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamState {
    pub audio_sample_rate_hz: f32,
    pub audio_format: String,
    pub waterfall_bins: usize,
    pub waterfall_frame_rate_hz: f32,
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub input_sample_rate_hz: f32,
    pub udp_audio_port: u16,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            audio_sample_rate_hz: 48_000.0,
            audio_format: "i16".to_string(),
            waterfall_bins: WATERFALL_BINS,
            waterfall_frame_rate_hz: WATERFALL_FRAME_RATE_HZ,
            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            input_sample_rate_hz: 0.0,
            udp_audio_port: 9001,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub radio: Arc<RwLock<RadioState>>,
    pub stream: Arc<RwLock<StreamState>>,
    pub tx: broadcast::Sender<ServerMessage>,
    pub audio_tx: broadcast::Sender<Vec<u8>>,
    pub waterfall_tx: broadcast::Sender<Vec<u8>>,
    pub udp_audio_target: Arc<RwLock<Option<SocketAddr>>>,
    pub radio_cmd_tx: mpsc::UnboundedSender<RadioCommand>,
    pub radio_manager: Arc<RadioManager>,
}

impl AppState {
    pub fn new(
        center_freq_hz: f32,
        target_freq_hz: f32,
        sideband: Sideband,
        demod_mode: DemodMode,
        ssb_pitch_hz: f32,
        radio_cmd_tx: mpsc::UnboundedSender<RadioCommand>,
	radio_manager: Arc<RadioManager>,
    ) -> Self {
        let (tx, _) = broadcast::channel(256);
        let (audio_tx, _) = broadcast::channel(256);
        let (waterfall_tx, _) = broadcast::channel(256);

        Self {
            radio: Arc::new(RwLock::new(RadioState::new(
                center_freq_hz,
                target_freq_hz,
                sideband,
                demod_mode,
                ssb_pitch_hz,
            ))),
            stream: Arc::new(RwLock::new(StreamState::default())),
            tx,
            audio_tx,
            waterfall_tx,
            udp_audio_target: Arc::new(RwLock::new(None)),
            radio_cmd_tx,
	    radio_manager,
        }
    }
}

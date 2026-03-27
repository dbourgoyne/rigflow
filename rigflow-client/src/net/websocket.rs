use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use rigflow_protocol::{ClientMessage, ServerMessage};

use crate::app::state::UiState;

pub async fn websocket_control_task(
    ws_url: &str,
    mut rx: mpsc::UnboundedReceiver<ClientMessage>,
    ui_state: Arc<Mutex<UiState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (ws_stream, _) = tokio_tungstenite::connect_async(ws_url).await?;
    let (mut write, mut read) = ws_stream.split();

    {
        let mut state = ui_state.lock().unwrap();
        state.status = "ws connected".to_string();
    }

    loop {
        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let text = serde_json::to_string(&cmd)?;
                        write.send(tokio_tungstenite::tungstenite::Message::Text(text)).await?;
                    }
                    None => break,
                }
            }

            msg = read.next() => {
                match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                            apply_server_message(server_msg, &ui_state);
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(Box::new(e)),
                    None => break,
                }
            }
        }
    }

    Ok(())
}

pub fn apply_server_message(msg: ServerMessage, ui_state: &Arc<Mutex<UiState>>) {
    let mut state = ui_state.lock().unwrap();

    match msg {
        ServerMessage::Ready => {
            state.status = "ready".to_string();
        }
        ServerMessage::Pong => {
            state.status = "pong".to_string();
        }
        ServerMessage::FrequencyChanged { target_freq_hz } => {
            state.target_freq_hz = target_freq_hz;
        }
        ServerMessage::CenterFrequencyChanged { center_freq_hz } => {
            state.center_freq_hz = center_freq_hz;
        }
        ServerMessage::SidebandChanged { sideband } => {
            state.sideband = sideband;
        }
        ServerMessage::DemodModeChanged { mode } => {
            state.demod_mode = mode;
        }
        ServerMessage::StreamConfig {
            audio_sample_rate_hz,
            audio_format,
            waterfall_bins,
            waterfall_frame_rate_hz,
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
        } => {
            state.audio_sample_rate_hz = audio_sample_rate_hz;
            state.audio_format = audio_format;
            state.waterfall_bins = waterfall_bins;
            state.waterfall_frame_rate_hz = waterfall_frame_rate_hz;
            state.center_freq_hz = center_freq_hz;
            state.target_freq_hz = target_freq_hz;
            state.input_sample_rate_hz = input_sample_rate_hz;
            state.status = "stream configured".to_string();
        }
        ServerMessage::UdpAudioOffer { server_udp_port } => {
            state.status = format!("udp audio offered on {}", server_udp_port);
        }
        ServerMessage::Info { message } => {
            state.status = message;
        }
        ServerMessage::Error { message } => {
            state.status = format!("error: {}", message);
        }
        ServerMessage::SsbPitchChanged { pitch_hz } => {
            state.ssb_pitch_hz = pitch_hz;
        }
    }
}

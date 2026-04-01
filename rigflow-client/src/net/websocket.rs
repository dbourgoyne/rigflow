use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};
use rigflow_protocol::{ClientMessage, ServerMessage};
use tokio::sync::mpsc;

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

    let list_msg = ClientRadioMessage::ListRadios;
    let text = serde_json::to_string(&list_msg)?;
    println!("CLIENT sending: {}", text);
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(text))
        .await?;

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
            if let Ok(radio_msg) = serde_json::from_str::<ServerRadioMessage>(&text) {
                println!("CLIENT got radio message: {:?}", radio_msg);

                if let Some(outgoing) = apply_radio_server_message(radio_msg, &ui_state) {
                    let text = serde_json::to_string(&outgoing)?;
                    println!("CLIENT sending AcquireRadio: {}", text);
                    write.send(tokio_tungstenite::tungstenite::Message::Text(text)).await?;
                }
            }
            else if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                apply_server_message(server_msg, &ui_state);
            }
            else {
                println!("CLIENT unknown message: {}", text);
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

pub fn apply_radio_server_message(
    msg: ServerRadioMessage,
    ui_state: &Arc<Mutex<UiState>>,
) -> Option<ClientRadioMessage> {
    let mut state = ui_state.lock().unwrap();

    match msg {
        ServerRadioMessage::RadiosListed { radios } => {
            state.status = "acquiring radio".to_string();

            if let Some(radio) = radios.into_iter().find(|r| !r.is_leased) {
                let audio_udp_peer_string = "192.168.0.225:9001".to_string();
                let waterfall_udp_peer_string = "192.168.0.225:9002".to_string();

                return Some(ClientRadioMessage::AcquireRadio {
                    radio_id: radio.id,
                    center_freq_hz: state.center_freq_hz as u64,
                    target_freq_hz: state.target_freq_hz as u64,
                    audio_udp_peer: audio_udp_peer_string,
                    waterfall_udp_peer: waterfall_udp_peer_string,
                });
            } else {
                state.status = "no radios available".to_string();
            }
        }

        ServerRadioMessage::RadioAcquired { .. } => {
            state.radio_acquired = true;
            state.status = "radio acquired".to_string();
        }

        ServerRadioMessage::RadioReleased { .. } => {
            state.radio_acquired = false;
            state.status = "radio released".to_string();
        }

        ServerRadioMessage::LeaseRenewed { .. } => {
            state.status = "lease renewed".to_string();
        }

        ServerRadioMessage::RadioError { message, .. } => {
            state.status = format!("radio error: {}", message);
        }
    }

    None
}

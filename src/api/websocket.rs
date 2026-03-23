use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{sink::SinkExt, stream::StreamExt};

use crate::{
    api::protocol::{ClientMessage, ServerMessage},
    dsp::demod::Sideband,
    server::app_state::AppState,
};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| client_socket(socket, state))
}

async fn client_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    let mut msg_rx = state.tx.subscribe();
    let mut audio_rx = state.audio_tx.subscribe();
    let mut waterfall_rx = state.waterfall_tx.subscribe();

        {
        let stream = state.stream.read().await;

        let stream_config = ServerMessage::StreamConfig {
            audio_sample_rate_hz: stream.audio_sample_rate_hz,
            audio_format: stream.audio_format.clone(),
            waterfall_bins: stream.waterfall_bins,
            waterfall_frame_rate_hz: stream.waterfall_frame_rate_hz,
            center_freq_hz: stream.center_freq_hz,
            input_sample_rate_hz: stream.input_sample_rate_hz,
        };

        let udp_offer = ServerMessage::UdpAudioOffer {
            server_udp_port: stream.udp_audio_port,
        };

        let text = serde_json::to_string(&stream_config).unwrap();
        if sender.send(Message::Text(text.into())).await.is_err() {
            return;
        }

        let text = serde_json::to_string(&udp_offer).unwrap();
        if sender.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }

    //let ready = serde_json::to_string(&ServerMessage::Ready).unwrap();
    //sender.send(Message::Text(ready.into())).await?;
    //if sender.send(Message::Text(ready.into())).await.is_err() {
    //    return;
    //}

    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = msg_rx.recv() => {
                    match msg {
                        Ok(msg) => {
                            let text = match serde_json::to_string(&msg) {
                                Ok(t) => t,
                                Err(_) => continue,
                            };

                            if sender.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                audio = audio_rx.recv() => {
                    match audio {
                        Ok(mut bytes) => {
                            let mut framed = Vec::with_capacity(bytes.len() + 1);
                            framed.push(b'A');
                            framed.append(&mut bytes);

                            if sender.send(Message::Binary(framed.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                wf = waterfall_rx.recv() => {
                    match wf {
                        Ok(mut bytes) => {
                            let mut framed = Vec::with_capacity(bytes.len() + 1);
                            framed.push(b'W');
                            framed.append(&mut bytes);

                            if sender.send(Message::Binary(framed.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });

    let state_for_recv = state.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    if let Err(err) = handle_client_text(&text, &state_for_recv).await {
                        let _ = state_for_recv.tx.send(ServerMessage::Error { message: err });
                    }
                }
                Message::Close(_) => break,
                Message::Ping(payload) => {
                    let _ = state_for_recv.tx.send(ServerMessage::Info {
                        message: format!("received ping ({} bytes)", payload.len()),
                    });
                }
                Message::Pong(_) | Message::Binary(_) => {}
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
}

async fn handle_client_text(text: &str, state: &AppState) -> Result<(), String> {
    let msg: ClientMessage =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON message: {e}"))?;

    match msg {
        ClientMessage::Ping => {
            let _ = state.tx.send(ServerMessage::Pong);
        }

        ClientMessage::SetFrequency { target_freq_hz } => {
            {
                let mut radio = state.radio.write().await;
                radio.target_freq_hz = target_freq_hz;
            }

            let _ = state.tx.send(ServerMessage::FrequencyChanged { target_freq_hz });
        }

        ClientMessage::SetCenterFrequency { center_freq_hz } => {
            {
                let mut radio = state.radio.write().await;
                radio.center_freq_hz = center_freq_hz;
            }

            let _ = state.tx.send(ServerMessage::CenterFrequencyChanged { center_freq_hz });
        }

	ClientMessage::SetDemodMode { mode } => {
	    let parsed = match mode.as_str() {
		"usb" => Some(crate::dsp::demod::DemodMode::Usb),
		"lsb" => Some(crate::dsp::demod::DemodMode::Lsb),
		"wfm" => Some(crate::dsp::demod::DemodMode::Wfm),
		_ => None,
	    };

	    if let Some(parsed_mode) = parsed {
		{
		    let mut radio = state.radio.write().await;
		    radio.demod_mode = parsed_mode;
		}

		let _ = state.tx.send(ServerMessage::DemodModeChanged { mode });
	    } else {
		let _ = state.tx.send(ServerMessage::Error {
		    message: format!("unknown demod mode: {}", mode),
		});
	    }
	}

        ClientMessage::SetSideband { sideband } => {
            let parsed = match sideband.to_ascii_lowercase().as_str() {
                "usb" => Sideband::Usb,
                "lsb" => Sideband::Lsb,
                _ => return Err(format!("invalid sideband '{sideband}', expected usb or lsb")),
            };

            {
                let mut radio = state.radio.write().await;
                radio.sideband = parsed;
            }

            let _ = state.tx.send(ServerMessage::SidebandChanged {
                sideband: match parsed {
                    Sideband::Usb => "usb".to_string(),
                    Sideband::Lsb => "lsb".to_string(),
                },
            });
        }
    }

    Ok(())
}

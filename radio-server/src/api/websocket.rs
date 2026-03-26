use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{sink::SinkExt, stream::StreamExt};
use rigflow_protocol::{ClientMessage, ServerMessage};
use crate::{
    dsp::demod::{DemodMode, Sideband},
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

    if send_initial_state(&mut sender, &state).await.is_err() {
        return;
    }

    let mut msg_rx = state.tx.subscribe();
    let mut audio_rx = state.audio_tx.subscribe();
    let mut waterfall_rx = state.waterfall_tx.subscribe();

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
			    println!("ws rx: {}", text);


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

async fn send_initial_state(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    state: &AppState,
) -> Result<(), ()> {
    let (
        audio_sample_rate_hz,
        audio_format,
        waterfall_bins,
        waterfall_frame_rate_hz,
        udp_audio_port,
        input_sample_rate_hz,
    ) = {
        let stream = state.stream.read().await;
        (
            stream.audio_sample_rate_hz,
            stream.audio_format.clone(),
            stream.waterfall_bins,
            stream.waterfall_frame_rate_hz,
            stream.udp_audio_port,
            stream.input_sample_rate_hz,
        )
    };

    let (center_freq_hz, target_freq_hz, demod_mode, sideband, ssb_pitch_hz) = {
        let radio = state.radio.read().await;
        (
            radio.center_freq_hz,
            radio.target_freq_hz,
            radio.demod_mode,
            radio.sideband,
	    radio.ssb_pitch_hz,
        )
    };

    send_server_message(
        sender,
        &ServerMessage::StreamConfig {
            audio_sample_rate_hz,
            audio_format,
            waterfall_bins,
            waterfall_frame_rate_hz,
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
        },
    ).await?;

    send_server_message(
        sender,
        &ServerMessage::UdpAudioOffer {
            server_udp_port: udp_audio_port,
        },
    ).await?;

    send_server_message(
        sender,
        &ServerMessage::CenterFrequencyChanged { center_freq_hz },
    ).await?;

    send_server_message(
        sender,
        &ServerMessage::FrequencyChanged { target_freq_hz },
    ).await?;

    send_server_message(
        sender,
        &ServerMessage::DemodModeChanged {
            mode: demod_mode_to_string(demod_mode),
        },
    ).await?;

    send_server_message(
        sender,
        &ServerMessage::SidebandChanged {
            sideband: sideband_to_string(sideband),
        },
    ).await?;

    send_server_message(
	sender,
	&ServerMessage::SsbPitchChanged {
            pitch_hz: ssb_pitch_hz,
	},
    ).await?;

    Ok(())
}

async fn send_server_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMessage,
) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sender.send(Message::Text(text.into())).await.map_err(|_| ())
}

fn demod_mode_to_string(mode: DemodMode) -> String {
    match mode {
        DemodMode::Wfm => "wfm".to_string(),
        DemodMode::Usb => "usb".to_string(),
        DemodMode::Lsb => "lsb".to_string(),
    }
}

fn sideband_to_string(sideband: Sideband) -> String {
    match sideband {
        Sideband::Usb => "usb".to_string(),
        Sideband::Lsb => "lsb".to_string(),
    }
}

//use crate::api::protocol::{ClientMessage, ServerMessage};
//use crate::server::app_state::AppState;

//pub
async fn handle_client_text(
    text: &str,
    state: &AppState,
) -> Result<(), String> {
    let cmd: ClientMessage = serde_json::from_str(text)
        .map_err(|e| format!("invalid json: {}", e))?;

    match cmd {
        ClientMessage::SetFrequency { target_freq_hz } => {
            let new_target = {
                let mut radio = state.radio.write().await;
                radio.target_freq_hz = target_freq_hz;
                radio.target_freq_hz
            };

            let _ = state.tx.send(ServerMessage::FrequencyChanged {
                target_freq_hz: new_target,
            });
        }

        ClientMessage::SetCenterFrequency { center_freq_hz } => {
            let new_center = {
                let mut radio = state.radio.write().await;
                radio.center_freq_hz = center_freq_hz;
                radio.center_freq_hz
            };

            let _ = state.tx.send(ServerMessage::CenterFrequencyChanged {
                center_freq_hz: new_center,
            });
        }

        ClientMessage::SetDemodMode { mode } => {
            let new_mode = {
                let mut radio = state.radio.write().await;
                radio.demod_mode = parse_demod_mode(&mode)?;
                radio.demod_mode
            };

            let _ = state.tx.send(ServerMessage::DemodModeChanged {
                mode: demod_mode_to_string(new_mode),
            });
        }

        ClientMessage::SetSideband { sideband } => {
            let new_sideband = {
                let mut radio = state.radio.write().await;
                radio.sideband = parse_sideband(&sideband)?;
                radio.sideband
            };

            let _ = state.tx.send(ServerMessage::SidebandChanged {
                sideband: sideband_to_string(new_sideband),
            });
	}

	ClientMessage::SetSsbPitch { pitch_hz } => {
	    let new_pitch = {
		let mut radio = state.radio.write().await;
		radio.ssb_pitch_hz = pitch_hz;
		println!("radio.ssb_pitch_hz = {}", radio.ssb_pitch_hz);
		radio.ssb_pitch_hz
	    };

	    let _ = state.tx.send(ServerMessage::SsbPitchChanged {
		pitch_hz: new_pitch,
	    });
	}

        ClientMessage::Ping => {
            let _ = state.tx.send(ServerMessage::Pong);
        }
    }

    Ok(())
}

fn parse_sideband(s: &str) -> Result<Sideband, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "usb" => Ok(Sideband::Usb),
        "lsb" => Ok(Sideband::Lsb),
        _ => Err(format!("invalid sideband: '{}'", s)),
    }
}


fn parse_demod_mode(s: &str) -> Result<DemodMode, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "wfm" | "fm" => Ok(DemodMode::Wfm),
        "usb" => Ok(DemodMode::Usb),
        "lsb" => Ok(DemodMode::Lsb),
        _ => Err(format!("invalid demod mode: '{}'", s)),
    }
}

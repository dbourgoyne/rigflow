use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{sink::SinkExt, stream::StreamExt};
use tokio::sync::mpsc;

use rigflow_protocol::{ClientMessage, ServerMessage};
use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};

use crate::{
    dsp::demod::{DemodMode, Sideband},
    server::{
        app_state::AppState,
        control::RadioCommand,
        radio_protocol::{
            manager_error_to_protocol, parse_acquire_request, radio_summary_to_protocol,
        },
        radio_types::{ClientId, RadioManagerError, StopReason, WorkerCommand},
        session::SessionState,
    },
};

enum ConnectionMessage {
    Legacy(ServerMessage),
    Radio(ServerRadioMessage),
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| client_socket(socket, state))
}

async fn client_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    let mut session = SessionState::new(ClientId(uuid::Uuid::new_v4().to_string()));

    if send_initial_state(&mut sender, &state).await.is_err() {
        return;
    }

    let mut msg_rx = state.tx.subscribe();
    let mut audio_rx = state.audio_tx.subscribe();
    let mut waterfall_rx = state.waterfall_tx.subscribe();
    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<ConnectionMessage>();

    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                maybe_local = local_rx.recv() => {
                    match maybe_local {
                        Some(msg) => {
                            if send_connection_message(&mut sender, &msg).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }

                msg = msg_rx.recv() => {
                    match msg {
                        Ok(msg) => {
                            if send_connection_message(&mut sender, &ConnectionMessage::Legacy(msg)).await.is_err() {
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
                    handle_incoming_text(
                        &text,
                        &state_for_recv,
                        &mut session,
                        &local_tx,
                    )
                    .await;
                }

                Message::Close(_) => break,

                Message::Ping(payload) => {
                    let _ = local_tx.send(ConnectionMessage::Legacy(ServerMessage::Info {
                        message: format!("received ping ({} bytes)", payload.len()),
                    }));
                }

                Message::Pong(_) | Message::Binary(_) => {}
            }
        }

        release_session_radio_on_disconnect(&state_for_recv, &mut session).await;
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
    )
    .await?;

    send_server_message(
        sender,
        &ServerMessage::UdpAudioOffer {
            server_udp_port: udp_audio_port,
        },
    )
    .await?;

    send_server_message(
        sender,
        &ServerMessage::CenterFrequencyChanged { center_freq_hz },
    )
    .await?;

    send_server_message(
        sender,
        &ServerMessage::FrequencyChanged { target_freq_hz },
    )
    .await?;

    send_server_message(
        sender,
        &ServerMessage::DemodModeChanged {
            mode: demod_mode_to_string(demod_mode),
        },
    )
    .await?;

    send_server_message(
        sender,
        &ServerMessage::SidebandChanged {
            sideband: sideband_to_string(sideband),
        },
    )
    .await?;

    send_server_message(
        sender,
        &ServerMessage::SsbPitchChanged {
            pitch_hz: ssb_pitch_hz,
        },
    )
    .await?;

    Ok(())
}

async fn send_connection_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &ConnectionMessage,
) -> Result<(), ()> {
    let text = match msg {
        ConnectionMessage::Legacy(msg) => serde_json::to_string(msg).map_err(|_| ())?,
        ConnectionMessage::Radio(msg) => serde_json::to_string(msg).map_err(|_| ())?,
    };

    sender
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}

async fn send_server_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMessage,
) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sender.send(Message::Text(text.into())).await.map_err(|_| ())
}

async fn handle_incoming_text(
    text: &str,
    app_state: &AppState,
    session: &mut SessionState,
    local_tx: &mpsc::UnboundedSender<ConnectionMessage>,
) {
    if let Ok(cmd) = serde_json::from_str::<ClientRadioMessage>(text) {
        let response = handle_radio_message(app_state, session, cmd).await;
        let _ = local_tx.send(ConnectionMessage::Radio(response));
        return;
    }

    match handle_legacy_client_text(text, app_state, session).await {
        Ok(Some(reply)) => {
            let _ = local_tx.send(ConnectionMessage::Legacy(reply));
        }
        Ok(None) => {}
        Err(err) => {
            let _ = local_tx.send(ConnectionMessage::Legacy(ServerMessage::Error {
                message: err,
            }));
        }
    }
}

async fn handle_legacy_client_text(
    text: &str,
    state: &AppState,
    session: &SessionState,
) -> Result<Option<ServerMessage>, String> {
    let cmd: ClientMessage =
        serde_json::from_str(text).map_err(|e| format!("invalid json: {}", e))?;

    match cmd {
        ClientMessage::SetFrequency { target_freq_hz } => {
            match send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetTargetFrequency { hz: target_freq_hz as u64 },
            )
            .await
            {
                Ok(()) => {}
                Err(RadioManagerError::NoActiveLease) => {
                    state
                        .radio_cmd_tx
                        .send(RadioCommand::SetTargetFrequency(target_freq_hz))
                        .map_err(|_| "failed to send radio command".to_string())?;
                }
                Err(err) => return Err(radio_manager_error_string(err)),
            }

            let new_target = {
                let mut radio = state.radio.write().await;
                radio.target_freq_hz = target_freq_hz;
                radio.target_freq_hz
            };

            let _ = state.tx.send(ServerMessage::FrequencyChanged {
                target_freq_hz: new_target,
            });

            Ok(None)
        }

        ClientMessage::SetCenterFrequency { center_freq_hz } => {
            match send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetCenterFrequency { hz: center_freq_hz as u64 },
            )
            .await
            {
                Ok(()) => {}
                Err(RadioManagerError::NoActiveLease) => {
                    state
                        .radio_cmd_tx
                        .send(RadioCommand::SetCenterFrequency(center_freq_hz))
                        .map_err(|_| "failed to send radio command".to_string())?;
                }
                Err(err) => return Err(radio_manager_error_string(err)),
            }

            let new_center = {
                let mut radio = state.radio.write().await;
                radio.center_freq_hz = center_freq_hz;
                radio.center_freq_hz
            };

            let _ = state.tx.send(ServerMessage::CenterFrequencyChanged {
                center_freq_hz: new_center,
            });

            Ok(None)
        }

        ClientMessage::SetDemodMode { mode } => {
            let cmd = RadioCommand::SetDemodMode(parse_demod_mode(&mode)?);
            state
                .radio_cmd_tx
                .send(cmd)
                .map_err(|_| "failed to send radio command".to_string())?;
            Ok(None)
        }

        ClientMessage::SetSideband { sideband } => {
            let cmd = RadioCommand::SetSideband(parse_sideband(&sideband)?);
            state
                .radio_cmd_tx
                .send(cmd)
                .map_err(|_| "failed to send radio command".to_string())?;
            Ok(None)
        }

        ClientMessage::SetSsbPitch { pitch_hz } => {
            state
                .radio_cmd_tx
                .send(RadioCommand::SetSsbPitch(pitch_hz))
                .map_err(|_| "failed to send radio command".to_string())?;
            Ok(None)
        }

        ClientMessage::Ping => Ok(Some(ServerMessage::Pong)),
    }
}

pub async fn handle_radio_message(
    app_state: &AppState,
    session: &mut SessionState,
    msg: ClientRadioMessage,
) -> ServerRadioMessage {
    match msg {
        ClientRadioMessage::ListRadios => {
            let radios = app_state
                .radio_manager
                .list_radios()
                .await
                .into_iter()
                .map(radio_summary_to_protocol)
                .collect();

            ServerRadioMessage::RadiosListed { radios }
        }

        ClientRadioMessage::AcquireRadio {
            radio_id,
            center_freq_hz,
            target_freq_hz,
            audio_udp_peer,
            waterfall_udp_peer,
        } => {
            if session.has_radio() {
                return ServerRadioMessage::RadioError {
                    code: "radio_already_acquired".to_string(),
                    message: "session already owns a radio; release it first".to_string(),
                };
            }

            let request = match parse_acquire_request(
                center_freq_hz,
                target_freq_hz,
                audio_udp_peer,
                waterfall_udp_peer,
            ) {
                Ok(request) => request,
                Err(err) => return err,
            };

            match app_state
                .radio_manager
                .acquire_radio(session.client_id.clone(), &radio_id, request)
                .await
            {
                Ok(result) => {
                    session.set_acquired_radio(result.radio_id.clone(), result.lease_id.clone());

                    let lease_ttl_ms = result
                        .lease_expires_at
                        .saturating_duration_since(std::time::Instant::now())
                        .as_millis() as u64;

                    ServerRadioMessage::RadioAcquired {
                        radio_id: result.radio_id,
                        lease_id: result.lease_id,
                        lease_ttl_ms,
                    }
                }
                Err(err) => manager_error_to_protocol(err),
            }
        }

        ClientRadioMessage::ReleaseRadio => {
            let acquired = match session.acquired_radio().cloned() {
                Some(acquired) => acquired,
                None => {
                    return ServerRadioMessage::RadioError {
                        code: "no_radio_acquired".to_string(),
                        message: "session has no acquired radio".to_string(),
                    };
                }
            };

            match app_state
                .radio_manager
                .release_radio(
                    &session.client_id,
                    &acquired.radio_id,
                    &acquired.lease_id,
                    StopReason::ClientRelease,
                )
                .await
            {
                Ok(()) => {
                    session.clear_acquired_radio();
                    ServerRadioMessage::RadioReleased {
                        radio_id: acquired.radio_id,
                    }
                }
                Err(err) => manager_error_to_protocol(err),
            }
        }

        ClientRadioMessage::RenewLease => {
            let acquired = match session.acquired_radio().cloned() {
                Some(acquired) => acquired,
                None => {
                    return ServerRadioMessage::RadioError {
                        code: "no_radio_acquired".to_string(),
                        message: "session has no acquired radio".to_string(),
                    };
                }
            };

            match app_state
                .radio_manager
                .renew_lease(
                    &session.client_id,
                    &acquired.radio_id,
                    &acquired.lease_id,
                )
                .await
            {
                Ok(lease) => {
                    let lease_ttl_ms = lease
                        .expires_at
                        .saturating_duration_since(std::time::Instant::now())
                        .as_millis() as u64;

                    ServerRadioMessage::LeaseRenewed {
                        radio_id: acquired.radio_id,
                        lease_ttl_ms,
                    }
                }
                Err(err) => manager_error_to_protocol(err),
            }
        }
    }
}

pub async fn release_session_radio_on_disconnect(
    app_state: &AppState,
    session: &mut SessionState,
) {
    let Some(acquired) = session.acquired_radio().cloned() else {
        return;
    };

    let _ = app_state
        .radio_manager
        .release_radio(
            &session.client_id,
            &acquired.radio_id,
            &acquired.lease_id,
            StopReason::ClientDisconnected,
        )
        .await;

    session.clear_acquired_radio();
}

pub async fn send_worker_command_for_session(
    app_state: &AppState,
    session: &SessionState,
    cmd: WorkerCommand,
) -> Result<(), RadioManagerError> {
    let acquired = session
        .acquired_radio()
        .ok_or(RadioManagerError::NoActiveLease)?;

    app_state
        .radio_manager
        .send_command(
            &session.client_id,
            &acquired.radio_id,
            &acquired.lease_id,
            cmd,
        )
        .await
}

fn radio_manager_error_string(err: RadioManagerError) -> String {
    match manager_error_to_protocol(err) {
        ServerRadioMessage::RadioError { message, .. } => message,
        _ => "radio manager error".to_string(),
    }
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
        "nfm" => Ok(DemodMode::Nfm),
        "usb" => Ok(DemodMode::Usb),
        "lsb" => Ok(DemodMode::Lsb),
        _ => Err(format!("invalid demod mode: '{}'", s)),
    }
}

fn demod_mode_to_string(mode: DemodMode) -> String {
    match mode {
        DemodMode::Wfm => "wfm".to_string(),
        DemodMode::Nfm => "nfm".to_string(),
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

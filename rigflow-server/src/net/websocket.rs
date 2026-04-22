use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
};
use futures::{sink::SinkExt, stream::StreamExt};
use log::{debug, info};
use tokio::sync::mpsc;

use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};
use rigflow_protocol::{ClientMessage, ServerMessage};
use rigflow_core::dsp::modes::{DemodMode, Sideband};

use crate::{
    app_state::AppState,
    radio::{
        api::{manager_error_to_protocol, parse_acquire_request, radio_summary_to_protocol},
        session::SessionState,
        types::{ClientId, RadioManagerError, StopReason, WorkerCommand, WorkerStatus},
    },
};

/// Messages that may be sent to a connected WebSocket client.
///
/// We currently support two logical protocols over the same socket:
/// - legacy JSON control/status messages
/// - radio-leasing / multi-radio control messages
enum ConnectionMessage {
    Legacy(ServerMessage),
    Radio(ServerRadioMessage),
}

/// Axum entry point for `/ws`.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| client_socket(socket, state))
}

/// Handle a single WebSocket client connection.
///
/// This function splits the socket into send/receive halves and runs:
/// - a send task that forwards server-side events to the client
/// - a receive task that parses inbound client messages
///
/// When either side exits, the connection is considered done.
async fn client_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Each connection gets a unique logical client id used by the radio manager.
    let mut session = SessionState::new(ClientId(uuid::Uuid::new_v4().to_string()));

    // Legacy broadcast channels.
    let mut msg_rx = state.tx.subscribe();
    let mut audio_rx = state.audio_tx.subscribe();
    let mut waterfall_rx = state.waterfall_tx.subscribe();

    // Per-connection local channel used for targeted responses.
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
                            let wrapped = ConnectionMessage::Legacy(msg);
                            if send_connection_message(&mut sender, &wrapped).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                audio = audio_rx.recv() => {
                    match audio {
                        Ok(mut bytes) => {
                            // Prefix audio frames with a type byte.
                            let mut framed = Vec::with_capacity(bytes.len() + 1);
                            framed.push(b'A');
                            framed.append(&mut bytes);

                            if sender.send(Message::Binary(framed)).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                wf = waterfall_rx.recv() => {
                    match wf {
                        Ok(mut bytes) => {
                            // Prefix waterfall frames with a type byte.
                            let mut framed = Vec::with_capacity(bytes.len() + 1);
                            framed.push(b'W');
                            framed.append(&mut bytes);

                            if sender.send(Message::Binary(framed)).await.is_err() {
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
                    handle_incoming_text(&text, &state_for_recv, &mut session, &local_tx).await;
                }
                Message::Close(_) => break,

                // We currently ignore inbound websocket ping/pong/binary frames from clients.
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
            }
        }

        // Best-effort cleanup: release any leased radio if the client disconnects.
        release_session_radio_on_disconnect(&state_for_recv, &mut session).await;
    });

    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }
}

/// Serialize and send a connection-scoped message to the client.
async fn send_connection_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &ConnectionMessage,
) -> Result<(), ()> {
    let text = match msg {
        ConnectionMessage::Legacy(msg) => serde_json::to_string(msg).map_err(|_| ())?,
        ConnectionMessage::Radio(msg) => serde_json::to_string(msg).map_err(|_| ())?,
    };

    sender.send(Message::Text(text)).await.map_err(|_| ())
}

/// Parse one inbound text frame.
///
/// We first try the newer radio-control protocol. If that fails, we fall back to
/// legacy text control messages.
async fn handle_incoming_text(
    text: &str,
    app_state: &AppState,
    session: &mut SessionState,
    local_tx: &mpsc::UnboundedSender<ConnectionMessage>,
) {
    if let Ok(cmd) = serde_json::from_str::<ClientRadioMessage>(text) {
        handle_radio_message(app_state, session, cmd, local_tx).await;
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

/// Handle legacy JSON control messages.
///
/// These messages ultimately become worker commands routed through the radio manager
/// for the currently acquired radio in this session.
async fn handle_legacy_client_text(
    text: &str,
    state: &AppState,
    session: &SessionState,
) -> Result<Option<ServerMessage>, String> {
    let cmd: ClientMessage =
        serde_json::from_str(text).map_err(|e| format!("invalid json: {e}"))?;

    match cmd {
	
        ClientMessage::SetFrequency { target_freq_hz } => {
            send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetTargetFrequency {
                    hz: target_freq_hz as u64,
                },
            )
            .await
            .map_err(radio_manager_error_string)?;

            Ok(None)
        }

        ClientMessage::SetCenterFrequency { center_freq_hz } => {
            send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetCenterFrequency {
                    hz: center_freq_hz as u64,
                },
            )
            .await
            .map_err(radio_manager_error_string)?;

            Ok(None)
        }

        ClientMessage::SetDemodMode { mode } => {
            send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetDemodMode {
                    mode,
                },
            )
            .await
            .map_err(radio_manager_error_string)?;

            Ok(None)
        }

        ClientMessage::SetSideband { sideband } => {
            send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetSideband {
                    sideband,
                },
            )
            .await
            .map_err(radio_manager_error_string)?;

            Ok(None)
        }

        ClientMessage::SetPitch { pitch_hz } => {
            send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetPitch { pitch_hz },
            )
            .await
            .map_err(radio_manager_error_string)?;

            Ok(None)
        }

	ClientMessage::SetFilterBandwidth { bandwidth_hz } => {
            send_worker_command_for_session(
                state,
                session,
                WorkerCommand::SetFilterBandwidth { bandwidth_hz },
            )
            .await
            .map_err(radio_manager_error_string)?;

            Ok(None)
        }
    }
}

/// Handle modern radio-control messages.
async fn handle_radio_message(
    app_state: &AppState,
    session: &mut SessionState,
    msg: ClientRadioMessage,
    local_tx: &mpsc::UnboundedSender<ConnectionMessage>,
) {
    match msg {
        ClientRadioMessage::ListRadios => {
            let radios = app_state
                .radio_manager
                .list_radios()
                .await
                .into_iter()
                .map(radio_summary_to_protocol)
                .collect();

            send_radio(local_tx, ServerRadioMessage::RadiosListed { radios });
        }

        ClientRadioMessage::AcquireRadio {
            radio_id,
            center_freq_hz,
            target_freq_hz,
            audio_udp_peer,
            waterfall_udp_peer,
        } => {
            if session.has_radio() {
                send_radio_error(
                    local_tx,
                    "radio_already_acquired",
                    "session already owns a radio; release it first",
                );
                return;
            }

            let request = match parse_acquire_request(
                center_freq_hz,
                target_freq_hz,
                audio_udp_peer,
                waterfall_udp_peer,
            ) {
                Ok(request) => request,
                Err(err) => {
                    send_radio(local_tx, err);
                    return;
                }
            };

            let acquire_result = match app_state
                .radio_manager
                .acquire_radio(session.client_id.clone(), &radio_id, request)
                .await
            {
                Ok(result) => result,
                Err(err) => {
                    send_radio(local_tx, manager_error_to_protocol(err));
                    return;
                }
            };

            session.set_acquired_radio(
                acquire_result.radio_id.clone(),
                acquire_result.lease_id.clone(),
            );

            let lease_ttl_ms = acquire_result
                .lease_expires_at
                .saturating_duration_since(std::time::Instant::now())
                .as_millis() as u64;

            info!(
                "radio acquired: client_id={:?} radio_id={:?} lease_id={:?}",
                session.client_id,
                acquire_result.radio_id,
                acquire_result.lease_id
            );

            // Send acquisition confirmation first.
            send_radio(
                local_tx,
                ServerRadioMessage::RadioAcquired {
                    radio_id: acquire_result.radio_id.clone(),
                    lease_id: acquire_result.lease_id.clone(),
                    lease_ttl_ms,
                },
            );

            // Subscribe to worker runtime state updates for this leased radio.
            let mut status_rx = match app_state
                .radio_manager
                .subscribe_runtime_status(
                    &session.client_id,
                    &acquire_result.radio_id,
                    &acquire_result.lease_id,
                )
                .await
            {
                Ok(status_rx) => status_rx,
                Err(err) => {
                    send_radio(local_tx, manager_error_to_protocol(err));
                    return;
                }
            };

            // Immediately send a full snapshot based on current worker status.
            let initial_status = status_rx.borrow().clone();
            if let Some(snapshot) =
                runtime_snapshot_from_status(acquire_result.radio_id.clone(), &initial_status)
            {
                log_runtime_snapshot(&snapshot);
                send_radio(local_tx, snapshot);
            }

            // Forward future runtime changes asynchronously.
            let local_tx_clone = local_tx.clone();
            let radio_id_clone = acquire_result.radio_id.clone();

            tokio::spawn(async move {
                loop {
                    if status_rx.changed().await.is_err() {
                        break;
                    }

                    let status = status_rx.borrow().clone();
                    if let Some(changed) =
                        runtime_changed_from_status(radio_id_clone.clone(), &status)
                    {
                        log_runtime_changed(&changed);
                        send_radio(&local_tx_clone, changed);
                    }
                }
            });
        }

        ClientRadioMessage::ReleaseRadio => {
            let acquired = match session.acquired_radio().cloned() {
                Some(acquired) => acquired,
                None => {
                    send_radio_error(
                        local_tx,
                        "no_radio_acquired",
                        "session has no acquired radio",
                    );
                    return;
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
                    send_radio(
                        local_tx,
                        ServerRadioMessage::RadioReleased {
                            radio_id: acquired.radio_id,
                        },
                    );
                }
                Err(err) => {
                    send_radio(local_tx, manager_error_to_protocol(err));
                }
            }
        }

        ClientRadioMessage::RenewLease => {
            let acquired = match session.acquired_radio().cloned() {
                Some(acquired) => acquired,
                None => {
                    send_radio_error(
                        local_tx,
                        "no_radio_acquired",
                        "session has no acquired radio",
                    );
                    return;
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

                    send_radio(
                        local_tx,
                        ServerRadioMessage::LeaseRenewed {
                            radio_id: acquired.radio_id,
                            lease_ttl_ms,
                        },
                    );
                }
                Err(err) => {
                    send_radio(local_tx, manager_error_to_protocol(err));
                }
            }
        }

	ClientRadioMessage::SetTargetFrequency { target_freq_hz } => {
            let _ = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetTargetFrequency {
                    hz: target_freq_hz as u64,
                },
            )
		.await
		.map_err(radio_manager_error_string);
        }


	ClientRadioMessage::SetCenterFrequency { center_freq_hz } => {
            let _ = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetCenterFrequency {
                    hz: center_freq_hz as u64,
                },
            )
		.await
		.map_err(radio_manager_error_string);
        }

	ClientRadioMessage::SetDemodMode { mode } => {
            let _ = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetDemodMode {
                    mode: mode as DemodMode,
                },
            )
		.await
		.map_err(radio_manager_error_string);
        }

	ClientRadioMessage::SetSideband { sideband } => {
            let _ = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetSideband {
                    sideband: sideband as Sideband,
                },
            )
		.await
		.map_err(radio_manager_error_string);
        }

	ClientRadioMessage::SetPitch { pitch_hz } => {
            let _ = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetPitch {
                    pitch_hz: pitch_hz as f32,
                },
            )
		.await
		.map_err(radio_manager_error_string);
        }

	ClientRadioMessage::SetFilterBandwidth { bandwidth_hz } => {
            let _ = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetFilterBandwidth {
                    bandwidth_hz: bandwidth_hz as f32,
                },
            )
		.await
		.map_err(radio_manager_error_string);
        }

    }
}

/// Best-effort cleanup when the WebSocket disconnects.
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

/// Route a worker command to the leased radio owned by this session.
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

/// Extract a human-readable message from a radio-manager error.
fn radio_manager_error_string(err: RadioManagerError) -> String {
    match manager_error_to_protocol(err) {
        ServerRadioMessage::RadioError { message, .. } => message,
        _ => "radio manager error".to_string(),
    }
}

/// Convert the current worker status into a full runtime snapshot for newly acquired clients.
fn runtime_snapshot_from_status(
    radio_id: rigflow_core::radio::RadioId,
    status: &WorkerStatus,
) -> Option<ServerRadioMessage> {
    match status {
        WorkerStatus::Running { runtime } => Some(ServerRadioMessage::RuntimeSnapshot {
            radio_id,
            center_freq_hz: runtime.center_freq_hz,
            target_freq_hz: runtime.target_freq_hz,
            input_sample_rate_hz: runtime.input_sample_rate_hz,
            audio_sample_rate_hz: runtime.audio_sample_rate_hz,
            audio_format: runtime.audio_format.clone(),
            waterfall_bins: runtime.waterfall_bins,
            waterfall_frame_rate_hz: runtime.waterfall_frame_rate_hz,
            demod_mode: runtime.demod_mode,
            sideband: runtime.sideband,
            ssb_pitch_hz: runtime.ssb_pitch_hz,
	    cw_pitch_hz: runtime.cw_pitch_hz,
	    filter_bandwidth_hz: runtime.filter_bandwidth_hz,
        }),
        _ => None,
    }
}

/// Convert the current worker status into an incremental runtime-changed message.
fn runtime_changed_from_status(
    radio_id: rigflow_core::radio::RadioId,
    status: &WorkerStatus,
) -> Option<ServerRadioMessage> {
    match status {
        WorkerStatus::Running { runtime } => Some(ServerRadioMessage::RuntimeChanged {
            radio_id,
            center_freq_hz: Some(runtime.center_freq_hz),
            target_freq_hz: Some(runtime.target_freq_hz),
            demod_mode: Some(runtime.demod_mode),
            sideband: Some(runtime.sideband),
            ssb_pitch_hz: Some(runtime.ssb_pitch_hz),
	    cw_pitch_hz: Some(runtime.cw_pitch_hz),
	    filter_bandwidth_hz: Some(runtime.filter_bandwidth_hz),
        }),
        _ => None,
    }
}

/// Send a radio-control protocol message over the local connection queue.
fn send_radio(
    local_tx: &mpsc::UnboundedSender<ConnectionMessage>,
    msg: ServerRadioMessage,
) {
    let _ = local_tx.send(ConnectionMessage::Radio(msg));
}

/// Send a standardized radio error.
fn send_radio_error(
    local_tx: &mpsc::UnboundedSender<ConnectionMessage>,
    code: &str,
    message: &str,
) {
    send_radio(
        local_tx,
        ServerRadioMessage::RadioError {
            code: code.to_string(),
            message: message.to_string(),
        },
    );
}

/// Best-effort debug logging for a full runtime snapshot.
fn log_runtime_snapshot(msg: &ServerRadioMessage) {
    if let ServerRadioMessage::RuntimeSnapshot {
        radio_id,
        center_freq_hz,
        target_freq_hz,
        input_sample_rate_hz,
        audio_sample_rate_hz,
        audio_format,
        waterfall_bins,
        waterfall_frame_rate_hz,
        demod_mode,
        sideband,
        ssb_pitch_hz,
	cw_pitch_hz,
	filter_bandwidth_hz,
    } = msg
    {
        debug!(
            "[websocket] RuntimeSnapshot radio={} center={} target={} input_sr={} audio_sr={} audio_fmt={} bins={} fps={} demod={} sideband={} ssb_pitch={} cw_pitch={} filter_bandwidth={}",
            radio_id.0,
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
            audio_sample_rate_hz,
            audio_format,
            waterfall_bins,
            waterfall_frame_rate_hz,
            demod_mode,
            sideband,
            ssb_pitch_hz,
	    cw_pitch_hz,
	    filter_bandwidth_hz,
        );
    }
}

/// Best-effort debug logging for an incremental runtime update.
fn log_runtime_changed(msg: &ServerRadioMessage) {
    if let ServerRadioMessage::RuntimeChanged {
        radio_id,
        center_freq_hz,
        target_freq_hz,
        demod_mode,
        sideband,
        ssb_pitch_hz,
	cw_pitch_hz,
	filter_bandwidth_hz,
    } = msg
    {
        info!(
            "[websocket] RuntimeChanged radio={} center={:?} target={:?} demod={:?} sideband={:?} ssb_pitch={:?} cw_pitch={:?} filter_bandwidth={:?}",
            radio_id.0,
            center_freq_hz,
            target_freq_hz,
            demod_mode,
            sideband,
            ssb_pitch_hz,
	    cw_pitch_hz,
	    filter_bandwidth_hz,
        );
    }
}

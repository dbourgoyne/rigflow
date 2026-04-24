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

use crate::{
    app_state::AppState,
    radio::{
        api::{manager_error_to_protocol, parse_acquire_request, radio_summary_to_protocol},
        session::SessionState,
        types::{ClientId, RadioManagerError, StopReason, WorkerCommand, WorkerStatus},
    },
};

type WsSender = futures::stream::SplitSink<WebSocket, Message>;

/// Axum entry point for `/ws`.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| client_socket(socket, state))
}

/// Handle a single WebSocket client connection.
///
/// Responsibilities:
/// - parse inbound `ClientRadioMessage`s
/// - send outbound `ServerRadioMessage`s
/// - manage per-connection lease/session cleanup
async fn client_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Each websocket connection maps to one logical client id.
    let mut session = SessionState::new(ClientId(uuid::Uuid::new_v4().to_string()));

    // Per-connection local channel for targeted responses and worker status updates.
    let (local_tx, mut local_rx) = mpsc::unbounded_channel::<ServerRadioMessage>();

    let send_task = tokio::spawn(async move {
        while let Some(msg) = local_rx.recv().await {
            if send_connection_message(&mut sender, &msg).await.is_err() {
                break;
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

                // No client-originated binary/media traffic over websocket.
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

/// Serialize and send one server radio message to the client.
async fn send_connection_message(
    sender: &mut WsSender,
    msg: &ServerRadioMessage,
) -> Result<(), ()> {
    let text = serde_json::to_string(msg).map_err(|_| ())?;
    sender.send(Message::Text(text.into())).await.map_err(|_| ())
}

/// Parse one inbound text frame.
///
/// We now support only the unified radio-control protocol.
async fn handle_incoming_text(
    text: &str,
    app_state: &AppState,
    session: &mut SessionState,
    local_tx: &mpsc::UnboundedSender<ServerRadioMessage>,
) {
    match serde_json::from_str::<ClientRadioMessage>(text) {
        Ok(cmd) => {
            handle_radio_message(app_state, session, cmd, local_tx).await;
        }
        Err(err) => {
            send_radio_error(
                local_tx,
                "invalid_message",
                &format!("failed to parse client message: {err}"),
            );
        }
    }
}

/// Handle unified radio-control messages.
async fn handle_radio_message(
    app_state: &AppState,
    session: &mut SessionState,
    msg: ClientRadioMessage,
    local_tx: &mpsc::UnboundedSender<ServerRadioMessage>,
) {
    info!("WEBSOCKET: handle_radio_message: msg = {:?}", msg);
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

            send_radio(
                local_tx,
                ServerRadioMessage::RadioAcquired {
                    radio_id: acquire_result.radio_id.clone(),
                    lease_id: acquire_result.lease_id.clone(),
                    lease_ttl_ms,
                },
            );

            // Subscribe to runtime state updates for this leased radio.
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

            // Send initial full snapshot.
            let initial_status = status_rx.borrow().clone();
            if let Some(snapshot) =
                runtime_snapshot_from_status(acquire_result.radio_id.clone(), &initial_status)
            {
                log_runtime_snapshot(&snapshot);
                send_radio(local_tx, snapshot);
            }

            // Forward future runtime changes.
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
            if let Err(err) = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetTargetFrequency { hz: target_freq_hz },
            )
            .await
            {
                send_radio_error(local_tx, "set_target_frequency_failed", &radio_manager_error_string(err));
            }
        }

        ClientRadioMessage::SetCenterFrequency { center_freq_hz } => {
            if let Err(err) = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetCenterFrequency { hz: center_freq_hz },
            )
            .await
            {
                send_radio_error(local_tx, "set_center_frequency_failed", &radio_manager_error_string(err));
            }
        }

        ClientRadioMessage::SetDemodMode { mode } => {
            if let Err(err) = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetDemodMode { mode },
            )
            .await
            {
                send_radio_error(local_tx, "set_demod_mode_failed", &radio_manager_error_string(err));
            }
        }

        ClientRadioMessage::SetSideband { sideband } => {
            if let Err(err) = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetSideband { sideband },
            )
            .await
            {
                send_radio_error(local_tx, "set_sideband_failed", &radio_manager_error_string(err));
            }
        }

        ClientRadioMessage::SetPitch { pitch_hz } => {
            if let Err(err) = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetPitch { pitch_hz },
            )
            .await
            {
                send_radio_error(local_tx, "set_pitch_failed", &radio_manager_error_string(err));
            }
        }

        ClientRadioMessage::SetFilterBandwidth { bandwidth_hz } => {
            if let Err(err) = send_worker_command_for_session(
                app_state,
                session,
                WorkerCommand::SetFilterBandwidth { bandwidth_hz },
            )
            .await
            {
                send_radio_error(
                    local_tx,
                    "set_filter_bandwidth_failed",
                    &radio_manager_error_string(err),
                );
            }
        }

	ClientRadioMessage::SetDeemphasisMode { mode } => {
	    info!("WEBSOCKET: SetDeemphasis: mode = {:?}", mode);
	    if let Err(err) = send_worker_command_for_session(
		app_state,
		session,
		WorkerCommand::SetDeemphasisMode { mode },
	    )
		.await
	    {
		send_radio_error(
		    local_tx,
		    "set_deemphasis_mode_failed",
		    &radio_manager_error_string(err),
		);
	    }
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
	    deemphasis_mode: runtime.deemphasis_mode,
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
	    deemphasis_mode: Some(runtime.deemphasis_mode),
        }),
        _ => None,
    }
}

/// Send a radio-control protocol message over the local connection queue.
fn send_radio(
    local_tx: &mpsc::UnboundedSender<ServerRadioMessage>,
    msg: ServerRadioMessage,
) {
    let _ = local_tx.send(msg);
}

/// Send a standardized radio error.
fn send_radio_error(
    local_tx: &mpsc::UnboundedSender<ServerRadioMessage>,
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
	deemphasis_mode,
    } = msg
    {
        debug!(
            "[websocket] RuntimeSnapshot radio={} center={} target={} input_sr={} audio_sr={} audio_fmt={} bins={} fps={} demod={:?} sideband={:?} ssb_pitch={} cw_pitch={} filter_bandwidth={}, deemphais_mode = {:?}",
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
	    deemphasis_mode,
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
	deemphasis_mode,
    } = msg
    {
        info!(
            "[websocket] RuntimeChanged radio={} center={:?} target={:?} demod={:?} sideband={:?} ssb_pitch={:?} cw_pitch={:?} filter_bandwidth={:?} deemphasis_mode={:?}",
            radio_id.0,
            center_freq_hz,
            target_freq_hz,
            demod_mode,
            sideband,
            ssb_pitch_hz,
            cw_pitch_hz,
            filter_bandwidth_hz,
	    deemphasis_mode,
        );
    }
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, error, info};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};

use tokio::net::TcpStream;
use tokio::sync::mpsc;

use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};
use rigflow_protocol::ServerMessage;

use crate::client_runtime::MediaCommand;
use crate::net::control::ControlCommand;
use crate::ui::state::UiState;

// --- Type aliases ----------------------------------------------------------

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

/// Main WebSocket control task.
///
/// Responsibilities:
/// - manage connection lifecycle
/// - forward UI commands to the server
/// - process server messages and update UI state
/// - coordinate lease renewal
/// - coordinate media runtime startup/reset behavior
pub async fn websocket_control_task(
    mut rx: mpsc::UnboundedReceiver<ControlCommand>,
    ui_state: Arc<Mutex<UiState>>,
    media_cmd_tx: mpsc::UnboundedSender<MediaCommand>,
    audio_session_generation: Arc<AtomicU64>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Current connection state. `write_opt` and `read_opt` are populated only
    // while connected.
    let mut write_opt: Option<WsWrite> = None;
    let mut read_opt: Option<WsRead> = None;
    let mut connected_server_ip: Option<String> = None;

    // Renew radio lease periodically while a radio is acquired.
    let mut renew_interval = tokio::time::interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            // --- Periodic lease renewal ------------------------------------
            _ = renew_interval.tick() => {
                let should_renew = {
                    let state = ui_state.lock().unwrap();
                    state.radio_acquired
                };

                if should_renew {
                    if let Some(write) = write_opt.as_mut() {
                        let renew = ClientRadioMessage::RenewLease;
                        let text = serde_json::to_string(&renew)?;

                        info!("CLIENT sending RenewLease");

                        write.send(Message::Text(text.into())).await?;
                    }
                }
            }

            // --- UI → control command handling -----------------------------
            cmd = rx.recv() => {
                match cmd {
                    Some(ControlCommand::AcquireRadio { radio_id }) => {
                        let Some(server_ip) = connected_server_ip.clone() else {
                            let mut state = ui_state.lock().unwrap();
                            state.server_status =
                                "acquire failed: not connected to a server".to_string();
                            continue;
                        };

                        let (udp_listen_port, ws_port, center_freq_hz, target_freq_hz) = {
                            let state = ui_state.lock().unwrap();
                            (
                                state.udp_listen_port,
                                state.rigflow_server_ws_port,
                                state.center_freq_hz as u64,
                                state.target_freq_hz as u64,
                            )
                        };

                        let udp_peer_addr = match build_udp_peer_addr(
                            &server_ip,
                            ws_port,
                            udp_listen_port,
                        ) {
                            Ok(addr) => addr,
                            Err(err) => {
                                let mut state = ui_state.lock().unwrap();
                                state.server_status = format!("acquire failed: {}", err);
                                continue;
                            }
                        };

                        if let Some(write) = write_opt.as_mut() {
                            let acquire = ClientRadioMessage::AcquireRadio {
                                radio_id: rigflow_core::radio::RadioId(radio_id),
                                center_freq_hz,
                                target_freq_hz,
                                audio_udp_peer: udp_peer_addr.clone(),
                                waterfall_udp_peer: udp_peer_addr,
                            };

                            let text = serde_json::to_string(&acquire)?;
                            info!("CLIENT sending AcquireRadio: {}", text);

                            write.send(Message::Text(text.into())).await?;
                        }
                    }

                    Some(ControlCommand::ReleaseRadio) => {
                        if let Some(write) = write_opt.as_mut() {
                            let msg = ClientRadioMessage::ReleaseRadio;
                            let text = serde_json::to_string(&msg)?;

                            info!("CLIENT sending ReleaseRadio");

                            write.send(Message::Text(text.into())).await?;
                        }
                    }

                    Some(ControlCommand::Connect { server_ip }) => {
                        if write_opt.is_some() {
                            continue;
                        }

                        let ws_port = {
                            let state = ui_state.lock().unwrap();
                            state.rigflow_server_ws_port
                        };

                        let ws_url = format!("ws://{}:{}/ws", server_ip, ws_port);
                        info!("CLIENT connecting to {}", ws_url);

                        match tokio_tungstenite::connect_async(&ws_url).await {
                            Ok((ws_stream, _)) => {
                                let (write, read) = ws_stream.split();

                                write_opt = Some(write);
                                read_opt = Some(read);
                                connected_server_ip = Some(server_ip.clone());

                                {
                                    let mut state = ui_state.lock().unwrap();
                                    state.server_connected = true;
                                    state.server_status =
                                        format!("connected to server {}", server_ip);
                                }

                                // Register the UDP media plane immediately after
                                // WebSocket connect succeeds.
                                let server_udp_port = {
                                    let state = ui_state.lock().unwrap();
                                    state.rigflow_server_udp_port
                                };

                                let _ = media_cmd_tx.send(MediaCommand::RegisterUdp {
                                    server_ip: server_ip.clone(),
                                    server_udp_port,
                                });

                                // Ask the server for the current radio list.
                                if let Some(write) = write_opt.as_mut() {
                                    let list_msg = ClientRadioMessage::ListRadios;
                                    let text = serde_json::to_string(&list_msg)?;

                                    debug!("CLIENT sending: {}", text);

                                    write.send(Message::Text(text.into())).await?;
                                }
                            }

                            Err(err) => {
                                let mut state = ui_state.lock().unwrap();
                                state.server_connected = false;
                                state.server_status = format!("connect failed: {}", err);
                            }
                        }
                    }

                    Some(ControlCommand::Disconnect) => {
                        info!("CLIENT disconnecting");

                        if let Some(mut write) = write_opt.take() {
                            let _ = write.close().await;
                        }

                        read_opt = None;
                        connected_server_ip = None;

                        let mut state = ui_state.lock().unwrap();
                        state.server_connected = false;
                        state.server_status = "no server".to_string();
                        state.radio_acquired = false;
                        state.selected_radio_id = None;
                        state.available_radios.clear();
                    }

            Some(ControlCommand::RadioMessage(cmd)) => {
            info!("WEBSOCKET got RadioMessage: {:?}", cmd);

            if let Some(write) = write_opt.as_mut() {
                let text = serde_json::to_string(&cmd)?;
                info!("WEBSOCKET sending text: {}", text);

                write.send(Message::Text(text.into())).await?;
            }
            }

                    None => break,
                }
            }

            // --- Server → client message handling --------------------------
            msg = async {
                match read_opt.as_mut() {
                    Some(read) => read.next().await,
                    None => None,
                }
            }, if read_opt.is_some() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Try radio-specific messages first.
                        if let Ok(radio_msg) =
                            serde_json::from_str::<ServerRadioMessage>(&text)
                        {
                            info!("CLIENT got radio message: {:?}", radio_msg);

                            if let Some(outgoing) = apply_radio_server_message(
                                radio_msg,
                                &ui_state,
                                &audio_session_generation,
                            ) {
                                let text = serde_json::to_string(&outgoing)?;
                                info!("CLIENT sending radio message: {}", text);

                                if let Some(write) = write_opt.as_mut() {
                                    write.send(Message::Text(text.into())).await?;
                                }
                            }
                        } else if let Ok(server_msg) =
                            serde_json::from_str::<ServerMessage>(&text)
                        {
                            let ServerMessage::Error { message } = server_msg;
                            let mut state = ui_state.lock().unwrap();
                            state.runtime_error = format!("error: {}", message);
                        } else {
                            info!("CLIENT unknown message: {}", text);
                        }
                    }

                    Some(Ok(_)) => {}

                    Some(Err(error)) => {
                        error!("CLIENT websocket error: {}", error);

                        write_opt = None;
                        read_opt = None;
                        connected_server_ip = None;

                        let mut state = ui_state.lock().unwrap();
                        state.server_connected = false;
                        state.server_status = format!("connection error: {}", error);
                        state.radio_acquired = false;

                        continue;
                    }

                    None => {
                        debug!("CLIENT websocket closed");

                        write_opt = None;
                        read_opt = None;
                        connected_server_ip = None;

                        let mut state = ui_state.lock().unwrap();
                        state.server_connected = false;
                        state.server_status = "no server".to_string();
                        state.radio_acquired = false;

                        continue;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Apply a radio-specific server message to client UI state.
///
/// Returns an optional outbound radio message if the client needs to reply.
/// Currently this always returns `None`, but the return type allows the
/// control loop to support request/response workflows later.
pub fn apply_radio_server_message(
    msg: ServerRadioMessage,
    ui_state: &Arc<Mutex<UiState>>,
    audio_session_generation: &Arc<AtomicU64>,
) -> Option<ClientRadioMessage> {
    let mut state = ui_state.lock().unwrap();

    match msg {
        ServerRadioMessage::RadiosListed { radios } => {
            state.available_radios = radios.clone();

            let caps = state
                .selected_radio_id
                .as_deref()
                .and_then(|selected_id| radios.iter().find(|r| r.id.0 == selected_id))
                .map(|r| (r.source_capabilities.clone(), r.radio_capabilities.clone()));

            if let Some((sc, rc)) = caps {
                state.source_capabilities = sc;
                state.radio_capabilities = rc;
            }

            if radios.is_empty() {
                state.server_status = "connected, no radios available".to_string();
            } else {
                state.server_status = format!("connected, {} radios available", radios.len());
            }
        }

        ServerRadioMessage::RadioAcquired { radio_id, .. } => {
            state.radio_acquired = true;
            state.selected_radio_id = Some(radio_id.0.clone());
            state.server_status = format!("radio acquired: {}", radio_id.0);

            let caps = state
                .available_radios
                .iter()
                .find(|radio| radio.id == radio_id)
                .map(|r| (r.source_capabilities.clone(), r.radio_capabilities.clone()));

            if let Some((sc, rc)) = caps {
                state.source_capabilities = sc;
                state.radio_capabilities = rc;
            }

            if state.auto_apply_default_bookmark_on_acquire
                && state.default_bookmark_id.is_some()
                && !state.bookmarks.is_empty()
            {
                state.pending_apply_default_bookmark = true;
            }

            // Reapply current mode controls on every acquire.
            state.pending_apply_mode_controls = true;

            // Force client audio pipeline to reset on radio switch/acquire.
            audio_session_generation.fetch_add(1, Ordering::Relaxed);
        }

        ServerRadioMessage::RadioReleased { .. } => {
            state.radio_acquired = false;
            state.server_status = "radio released".to_string();

            // Force client audio pipeline to reset on release.
            audio_session_generation.fetch_add(1, Ordering::Relaxed);
        }

        ServerRadioMessage::LeaseRenewed { .. } => {
            // No UI update currently required.
        }

        ServerRadioMessage::RuntimeSnapshot {
            radio_id: _,
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
            demod_mode,
            sideband,
            source_control,
            ..
        } => {
            state.center_freq_hz = center_freq_hz as f32;
            state.target_freq_hz = target_freq_hz as f32;
            state.input_sample_rate_hz = input_sample_rate_hz;
            state.demod_mode = demod_mode;
            state.sideband = sideband;
            state.source_control = source_control;

            // Do NOT overwrite persisted per-demod prefs here.
            state.pending_apply_mode_controls = true;
        }

        ServerRadioMessage::RuntimeChanged {
            radio_id: _,
            center_freq_hz,
            target_freq_hz,
            demod_mode,
            sideband,
            source_control,
            ..
        } => {
            if let Some(value) = center_freq_hz {
                state.center_freq_hz = value as f32;
            }

            if let Some(value) = target_freq_hz {
                state.target_freq_hz = value as f32;
            }

            if let Some(value) = demod_mode {
                let mode_changed = state.demod_mode != value;
                state.demod_mode = value;

                if mode_changed {
                    state.pending_apply_mode_controls = true;
                }
            }

            if let Some(ref value) = sideband {
                state.sideband = *value;
            }

            if let Some(value) = source_control {
                if value.sample_rate_hz != state.source_control.sample_rate_hz {
                    audio_session_generation.fetch_add(1, Ordering::Relaxed);
                }
                state.source_control = value;
            }
        }

        ServerRadioMessage::RadioError { message, .. } => {
            state.runtime_error = format!("radio error: {}", message);
        }
    }

    None
}

/// Build the UDP endpoint string that the client should advertise to the server.
///
/// This determines the correct local IP by binding a temporary probe socket,
/// connecting it to the server, and then reading the OS-selected local route.
/// The listen port comes from the already-bound media socket.
fn build_udp_peer_addr(
    server_ip: &str,
    server_port_for_route_probe: u16,
    udp_listen_port: u16,
) -> Result<String, String> {
    if udp_listen_port == 0 {
        return Err("udp listen port is not initialized".to_string());
    }

    let probe = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("failed to bind UDP probe socket: {e}"))?;

    probe
        .connect((server_ip, server_port_for_route_probe))
        .map_err(|e| {
            format!(
                "failed to probe route to server {server_ip}:{server_port_for_route_probe}: {e}"
            )
        })?;

    let local_ip = probe
        .local_addr()
        .map_err(|e| format!("failed to get local probe socket address: {e}"))?
        .ip();

    Ok(format!("{}:{}", local_ip, udp_listen_port))
}

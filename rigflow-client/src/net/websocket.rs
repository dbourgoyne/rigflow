use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, error, info};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

use rigflow_protocol::ServerMessage;
use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};

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

    // Auto-reconnect state.  Armed on a successful manual Connect, disarmed on a
    // user Disconnect — so a network drop reconnects but an intentional
    // disconnect does not.  `reconnect_at`/`reacquire_at` are deadlines that gate
    // the two timer arms in the select! below.
    let mut auto_reconnect_enabled = false;
    let mut reconnect_target_ip: Option<String> = None;
    let mut reconnect_at: Option<Instant> = None;
    let mut reconnect_backoff = Duration::from_secs(1);
    let mut reconnect_attempt: u32 = 0;
    // Radio to re-acquire after a reconnect; captured when an AcquireRadio is
    // sent, cleared on user ReleaseRadio/Disconnect.
    let mut last_radio_id: Option<String> = None;
    let mut reacquire_at: Option<Instant> = None;
    let mut reacquire_deadline: Option<Instant> = None;

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

                        if let Some(write) = write_opt.as_mut() {
                            match send_acquire(&radio_id, &server_ip, write, &ui_state).await {
                                // Remember it so a later drop is auto-recoverable.
                                Ok(()) => last_radio_id = Some(radio_id),
                                Err(err) => {
                                    let mut state = ui_state.lock().unwrap();
                                    state.server_status = format!("acquire failed: {}", err);
                                }
                            }
                        }
                    }

                    Some(ControlCommand::ReleaseRadio) => {
                        // A user release must not be auto-re-acquired.
                        last_radio_id = None;
                        reacquire_at = None;
                        reacquire_deadline = None;

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
                        // A manual Connect mid-reconnect short-circuits the backoff.
                        reconnect_at = None;

                        match try_connect(&server_ip, &ui_state, &media_cmd_tx).await {
                            Ok((write, read)) => {
                                write_opt = Some(write);
                                read_opt = Some(read);
                                connected_server_ip = Some(server_ip.clone());

                                {
                                    let mut state = ui_state.lock().unwrap();
                                    state.server_connected = true;
                                    state.server_status =
                                        format!("connected to server {}", server_ip);
                                }

                                // Arm auto-reconnect for this server.
                                auto_reconnect_enabled = true;
                                reconnect_target_ip = Some(server_ip);
                                reconnect_backoff = Duration::from_secs(1);
                                reconnect_attempt = 0;
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

                        // User intent: do NOT auto-reconnect. Disarm everything.
                        auto_reconnect_enabled = false;
                        reconnect_target_ip = None;
                        reconnect_at = None;
                        reconnect_attempt = 0;
                        last_radio_id = None;
                        reacquire_at = None;
                        reacquire_deadline = None;

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

                        // Arm auto-reconnect on an unexpected drop (vs the user's
                        // Disconnect, which disarms it). `last_radio_id` is kept so
                        // the radio is re-acquired once reconnected.
                        let status = if auto_reconnect_enabled {
                            reconnect_attempt = 1;
                            reconnect_backoff = Duration::from_secs(1);
                            reconnect_at = Some(Instant::now() + reconnect_backoff);
                            "reconnecting (attempt 1)…".to_string()
                        } else {
                            format!("connection error: {}", error)
                        };

                        let mut state = ui_state.lock().unwrap();
                        state.server_connected = false;
                        state.radio_acquired = false;
                        state.server_status = status;

                        continue;
                    }

                    None => {
                        debug!("CLIENT websocket closed");

                        write_opt = None;
                        read_opt = None;
                        connected_server_ip = None;

                        let status = if auto_reconnect_enabled {
                            reconnect_attempt = 1;
                            reconnect_backoff = Duration::from_secs(1);
                            reconnect_at = Some(Instant::now() + reconnect_backoff);
                            "reconnecting (attempt 1)…".to_string()
                        } else {
                            "no server".to_string()
                        };

                        let mut state = ui_state.lock().unwrap();
                        state.server_connected = false;
                        state.radio_acquired = false;
                        state.server_status = status;

                        continue;
                    }
                }
            }

            // --- Auto-reconnect: reopen the WS after an unexpected drop ----
            _ = async { sleep_until(reconnect_at.unwrap()).await }, if reconnect_at.is_some() => {
                let ip = reconnect_target_ip.clone().unwrap_or_default();
                match try_connect(&ip, &ui_state, &media_cmd_tx).await {
                    Ok((write, read)) => {
                        write_opt = Some(write);
                        read_opt = Some(read);
                        connected_server_ip = Some(ip.clone());
                        reconnect_at = None;
                        reconnect_backoff = Duration::from_secs(1);
                        reconnect_attempt = 0;

                        let mut state = ui_state.lock().unwrap();
                        state.server_connected = true;
                        if last_radio_id.is_some() {
                            // Re-acquire the radio we held; fire immediately.
                            reacquire_at = Some(Instant::now());
                            reacquire_deadline = Some(Instant::now() + REACQUIRE_GIVE_UP);
                            state.server_status = "re-acquiring radio…".to_string();
                        } else {
                            state.server_status = format!("connected to server {}", ip);
                        }
                    }
                    Err(err) => {
                        reconnect_attempt += 1;
                        reconnect_backoff = (reconnect_backoff * 2).min(RECONNECT_BACKOFF_MAX);
                        reconnect_at = Some(Instant::now() + reconnect_backoff);
                        debug!("CLIENT reconnect attempt {reconnect_attempt} failed: {err}");
                        ui_state.lock().unwrap().server_status =
                            format!("reconnecting (attempt {reconnect_attempt})…");
                    }
                }
            }

            // --- Auto-re-acquire: restore the radio after a reconnect ------
            _ = async { sleep_until(reacquire_at.unwrap()).await }, if reacquire_at.is_some() => {
                let acquired = ui_state.lock().unwrap().radio_acquired;
                if acquired {
                    // Success (RadioAcquired set the status) — stop retrying.
                    reacquire_at = None;
                    reacquire_deadline = None;
                } else if reacquire_deadline.is_some_and(|d| Instant::now() >= d) {
                    // Gave up: the old lease never freed within the window.
                    reacquire_at = None;
                    reacquire_deadline = None;
                    last_radio_id = None;
                    ui_state.lock().unwrap().server_status =
                        "re-acquire failed: radio still busy".to_string();
                } else if let (Some(rid), Some(ip), Some(write)) = (
                    last_radio_id.clone(),
                    connected_server_ip.clone(),
                    write_opt.as_mut(),
                ) {
                    // Re-send AcquireRadio; a `radio_busy` reply is tolerated and
                    // the timer fires again until the old lease frees.
                    let _ = send_acquire(&rid, &ip, write, &ui_state).await;
                    reacquire_at = Some(Instant::now() + REACQUIRE_RETRY);
                    ui_state.lock().unwrap().server_status = "re-acquiring radio…".to_string();
                } else {
                    // Lost the connection again mid-re-acquire; the reconnect arm
                    // will take over.
                    reacquire_at = None;
                }
            }
        }
    }

    Ok(())
}

/// Reconnect backoff cap.  Backoff grows 1→2→4→8→16→30 s and stays at 30 s.
const RECONNECT_BACKOFF_MAX: Duration = Duration::from_secs(30);
/// How long to keep retrying re-acquire (slightly over the server's 30 s lease
/// TTL) before giving up, so an ungraceful drop's stale lease has time to expire.
const REACQUIRE_GIVE_UP: Duration = Duration::from_secs(35);
/// Cadence of re-acquire retries while waiting for the old lease to free.
const REACQUIRE_RETRY: Duration = Duration::from_secs(4);

/// Open a WebSocket to `server_ip`, register the UDP media plane, and request
/// the radio list.  Shared by the manual `Connect` path and the auto-reconnect
/// timer.  Does not touch `server_status` — callers word it (manual vs attempt
/// N) — and returns the underlying error string on failure.
async fn try_connect(
    server_ip: &str,
    ui_state: &Arc<Mutex<UiState>>,
    media_cmd_tx: &mpsc::UnboundedSender<MediaCommand>,
) -> Result<(WsWrite, WsRead), String> {
    let (ws_port, server_udp_port) = {
        let state = ui_state.lock().unwrap();
        (state.rigflow_server_ws_port, state.rigflow_server_udp_port)
    };

    let ws_url = format!("ws://{}:{}/ws", server_ip, ws_port);
    info!("CLIENT connecting to {}", ws_url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| e.to_string())?;
    let (mut write, read) = ws_stream.split();

    // Register the UDP media plane immediately (stateless, repeatable).
    let _ = media_cmd_tx.send(MediaCommand::RegisterUdp {
        server_ip: server_ip.to_string(),
        server_udp_port,
    });

    // Ask the server for the current radio list.
    let list_msg = ClientRadioMessage::ListRadios;
    let text = serde_json::to_string(&list_msg).map_err(|e| e.to_string())?;
    write
        .send(Message::Text(text.into()))
        .await
        .map_err(|e| e.to_string())?;

    Ok((write, read))
}

/// Build and send an `AcquireRadio` for `radio_id`.  Shared by the manual
/// `AcquireRadio` path and the auto-re-acquire timer.
async fn send_acquire(
    radio_id: &str,
    server_ip: &str,
    write: &mut WsWrite,
    ui_state: &Arc<Mutex<UiState>>,
) -> Result<(), String> {
    let (udp_listen_port, ws_port, center_freq_hz, target_freq_hz) = {
        let state = ui_state.lock().unwrap();
        (
            state.udp_listen_port,
            state.rigflow_server_ws_port,
            state.center_freq_hz as u64,
            state.target_freq_hz as u64,
        )
    };

    let udp_peer_addr = build_udp_peer_addr(server_ip, ws_port, udp_listen_port)?;

    let acquire = ClientRadioMessage::AcquireRadio {
        radio_id: rigflow_core::radio::RadioId(radio_id.to_string()),
        center_freq_hz,
        target_freq_hz,
        audio_udp_peer: udp_peer_addr.clone(),
        waterfall_udp_peer: udp_peer_addr,
    };

    let text = serde_json::to_string(&acquire).map_err(|e| e.to_string())?;
    info!("CLIENT sending AcquireRadio: {}", text);
    write
        .send(Message::Text(text.into()))
        .await
        .map_err(|e| e.to_string())?;

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

            // No test is running on a fresh acquire.
            state.tx_tune_running = false;

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
            radio_id,
            center_freq_hz,
            target_freq_hz,
            input_sample_rate_hz,
            demod_mode,
            sideband,
            squelch_enabled,
            squelch_threshold_db,
            squelch_open,
            nr2_enabled,
            nr2_strength,
            agc_enabled,
            agc_strength,
            signal_dbm,
            signal_s_units,
            source_control,
            source_status,
            amplifier_status,
            iq_recording_status,
            tx_audio_diag,
            tx_tune_result,
            swr_sweep_result,
            swr_sweep_progress,
            ..
        } => {
            state.center_freq_hz = center_freq_hz as f32;
            state.target_freq_hz = target_freq_hz as f32;
            state.input_sample_rate_hz = input_sample_rate_hz;
            state.swr_sweep_result = swr_sweep_result;
            state.swr_sweep_progress = swr_sweep_progress;
            state.demod_mode = demod_mode;
            state.sideband = sideband;
            state.squelch_enabled = squelch_enabled;
            state.squelch_threshold_db = squelch_threshold_db;
            state.squelch_open = squelch_open;
            state.nr2_enabled = nr2_enabled;
            state.nr2_strength = nr2_strength;
            state.agc_enabled = agc_enabled;
            state.agc_strength = agc_strength;
            state.signal_dbm = signal_dbm;
            state.signal_s_units = signal_s_units;
            // Apply server default first, then override with saved prefs if present.
            state.source_control = source_control;
            if let Some(saved) = state.source_control_preferences.get(&radio_id.0).cloned() {
                state.source_control = saved;
                state.pending_apply_source_control = true;
            }
            state.source_status = source_status;
            state.amplifier_status = amplifier_status;
            state.iq_recording_status = iq_recording_status;
            state.tx_audio_diag = tx_audio_diag;
            if let Some(result) = tx_tune_result {
                state.last_tx_tune_result = result;
            }

            // Do NOT overwrite persisted per-demod prefs here.
            state.pending_apply_mode_controls = true;
        }

        ServerRadioMessage::RuntimeChanged {
            radio_id: _,
            center_freq_hz,
            target_freq_hz,
            demod_mode,
            sideband,
            squelch_enabled,
            squelch_threshold_db,
            squelch_open,
            nr2_enabled,
            nr2_strength,
            agc_enabled,
            agc_strength,
            signal_dbm,
            signal_s_units,
            volume_percent,
            source_control,
            source_status,
            amplifier_status,
            iq_recording_status,
            tx_audio_diag,
            tx_tune_result,
            swr_sweep_result,
            swr_sweep_progress,
            ..
        } => {
            if let Some(progress) = swr_sweep_progress {
                state.swr_sweep_progress = Some(progress);
            }
            if let Some(result) = swr_sweep_result {
                state.swr_sweep_result = Some(result);
                // A finished sweep result arrived — open the results popup.
                state.show_swr_sweep_window = true;
                state.swr_sweep_csv_status = None;
            }
            if let Some(value) = volume_percent {
                state.volume_percent = value;
            }
            if let Some(value) = squelch_enabled {
                state.squelch_enabled = value;
            }
            if let Some(value) = squelch_threshold_db {
                state.squelch_threshold_db = value;
            }
            if let Some(value) = squelch_open {
                state.squelch_open = value;
            }
            if let Some(value) = nr2_enabled {
                state.nr2_enabled = value;
            }
            if let Some(value) = nr2_strength {
                state.nr2_strength = value;
            }
            if let Some(value) = agc_enabled {
                state.agc_enabled = value;
            }
            if let Some(value) = agc_strength {
                state.agc_strength = value;
            }
            if let Some(value) = signal_dbm {
                state.signal_dbm = value;
            }
            if let Some(value) = signal_s_units {
                state.signal_s_units = value;
            }

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

            if let Some(value) = source_status {
                state.source_status = value;
            }

            if let Some(value) = amplifier_status {
                state.amplifier_status = value;
            }

            if let Some(value) = iq_recording_status {
                state.iq_recording_status = value;
            }

            if let Some(value) = tx_audio_diag {
                state.tx_audio_diag = value;
            }

            if let Some(result) = tx_tune_result {
                state.last_tx_tune_result = result;
                // Test completed (ok or fault) — clear the running indicator.
                state.tx_tune_running = false;
            }
        }

        ServerRadioMessage::RadioError { code, message } => {
            // During auto-re-acquire the old lease may still be held; the
            // re-acquire timer owns the status and retries, so don't surface a
            // spurious per-retry error for `radio_busy`.
            if code != "radio_busy" {
                state.runtime_error = format!("radio error: {}", message);
            }
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

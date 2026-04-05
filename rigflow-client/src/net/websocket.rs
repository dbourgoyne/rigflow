use std::sync::{Arc, Mutex};
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use futures_util::stream::{SplitSink, SplitStream};
use tokio::sync::mpsc;
use tokio_tungstenite::{
    tungstenite::Message,
    MaybeTlsStream, WebSocketStream,
};
use tokio::net::TcpStream;

use rigflow_protocol::radio_control::{ClientRadioMessage, ServerRadioMessage};
use rigflow_protocol::{ClientMessage, ServerMessage};

use crate::app::state::UiState;
use crate::net::control::ControlCommand;
use crate::client_runtime::MediaCommand;

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

pub async fn websocket_control_task(
    mut rx: mpsc::UnboundedReceiver<ControlCommand>,
    ui_state: Arc<Mutex<UiState>>,
    media_cmd_tx: mpsc::UnboundedSender<MediaCommand>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

    let mut write_opt: Option<WsWrite> = None;
    let mut read_opt: Option<WsRead> = None;
    let mut connected_server_ip: Option<String> = None;
    let mut renew_interval = tokio::time::interval(Duration::from_secs(10));

    loop {
	tokio::select! {
            _ = renew_interval.tick() => {
		let should_renew = {
                    let state = ui_state.lock().unwrap();
                    state.radio_acquired
		};

		if should_renew {
		    if let Some(write) = write_opt.as_mut() {
			let renew = ClientRadioMessage::RenewLease;
			let text = serde_json::to_string(&renew)?;
			println!("CLIENT sending RenewLease");
			    write
				.send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
				.await?;
		    }
		}

            }

	    cmd = rx.recv() => {
		match cmd {

		    Some(ControlCommand::AcquireRadio { radio_id }) => {
			let Some(server_ip) = connected_server_ip.clone() else {
			    let mut state = ui_state.lock().unwrap();
			    state.server_status = "acquire failed: not connected to a server".to_string();
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

			let udp_peer_addr = match build_udp_peer_addr(&server_ip, ws_port, udp_listen_port) {
			    Ok(addr) => addr,
			    Err(err) => {
				let mut state = ui_state.lock().unwrap();
				state.server_status = format!("acquire failed: {}", err);
				continue;
			    }
			};

			{
			    let mut state = ui_state.lock().unwrap();
			    state.advertised_udp_peer = udp_peer_addr.clone();
			}

			if let Some(write) = write_opt.as_mut() {
			    let acquire = ClientRadioMessage::AcquireRadio {
				radio_id: rigflow_core::radio::RadioId(radio_id),
				center_freq_hz,
				target_freq_hz,
				audio_udp_peer: udp_peer_addr.clone(),
				waterfall_udp_peer: udp_peer_addr,
			    };

			    let text = serde_json::to_string(&acquire)?;
			    println!("CLIENT sending AcquireRadio: {}", text);

			    write
				.send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
				.await?;
			}
		    }

		    Some(ControlCommand::ReleaseRadio) => {
			if let Some(write) = write_opt.as_mut() {
			    let msg = ClientRadioMessage::ReleaseRadio;
			    let text = serde_json::to_string(&msg)?;
			    println!("CLIENT sending ReleaseRadio");

			    write
				.send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
				.await?;
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
			println!("CLIENT connecting to {}", ws_url);

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

				let server_udp_port = {
				    let state = ui_state.lock().unwrap();
				    state.rigflow_server_udp_port
				};

				let _ = media_cmd_tx.send(MediaCommand::RegisterUdp {
				    server_ip: server_ip.clone(),
				    server_udp_port,
				});

				if let Some(write) = write_opt.as_mut() {
				    let list_msg = ClientRadioMessage::ListRadios;
				    let text = serde_json::to_string(&list_msg)?;
				    println!("CLIENT sending: {}", text);
					write
					    .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
					    .await?;
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
			println!("CLIENT disconnecting");

			if let Some(mut write) = write_opt.take() {
			    let _ = write.close().await;
			}

			read_opt = None;

			let mut state = ui_state.lock().unwrap();
			state.server_connected = false;
			state.server_status = "no server".to_string();
			state.radio_acquired = false;
			state.selected_radio_id = None;
			state.available_radios.clear();
			connected_server_ip = None;
		    }

		    Some(ControlCommand::LegacyClientMessage(cmd)) => {
			println!("WEBSOCKET got LegacyClientMessage: {:?}", cmd);
			if let Some(write) = write_opt.as_mut() {
			    let text = serde_json::to_string(&cmd)?;
			    println!("WEBSOCKET sending text: {}", text);
			    write
				.send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
				.await?;
			}
		    }

		    None => break,
		}
	    }

	    msg = async {
		match read_opt.as_mut() {
		    Some(read) => read.next().await,
		    None => None,
		}
	    }, if read_opt.is_some() => {
		match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
			if let Ok(radio_msg) = serde_json::from_str::<ServerRadioMessage>(&text) {
                            println!("CLIENT got radio message: {:?}", radio_msg);

                            if let Some(outgoing) = apply_radio_server_message(radio_msg, &ui_state) {
				let text = serde_json::to_string(&outgoing)?;
				println!("CLIENT sending radio message: {}", text);
				if let Some(write) = write_opt.as_mut() {
				    write
					.send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
					.await?;
				}
                            }
			} else if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(&text) {
                            apply_server_message(server_msg, &ui_state);
			} else {
                            println!("CLIENT unknown message: {}", text);
			}
                    }
                    Some(Ok(_)) => {}
		    Some(Err(e)) => {
			println!("CLIENT websocket error: {}", e);

			write_opt = None;
			read_opt = None;
			connected_server_ip = None;

			let mut state = ui_state.lock().unwrap();
			state.server_connected = false;
			state.server_status = format!("connection error: {}", e);
			state.radio_acquired = false;

			continue;
		    }

		    None => {
			println!("CLIENT websocket closed");

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
	    state.available_radios = radios.clone();
	    
	    if radios.is_empty() {
		state.server_status = "connected, no radios available".to_string();
	    } else {
		state.server_status = format!("connected, {} radios available", radios.len());
	    }

	    // Optional: do NOT auto-acquire here if you want the UI list to drive selection.
	}

	ServerRadioMessage::RadioAcquired { radio_id, .. } => {
	    state.radio_acquired = true;
	    state.selected_radio_id = Some(radio_id.0.clone());
	    state.server_status = format!("radio acquired: {}", radio_id.0);
	}

	ServerRadioMessage::RadioReleased { .. } => {
	    state.radio_acquired = false;
	    state.server_status = "radio released".to_string();
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
        .map_err(|e| format!("failed to probe route to server {server_ip}:{server_port_for_route_probe}: {e}"))?;

    let local_ip = probe
        .local_addr()
        .map_err(|e| format!("failed to get local probe socket address: {e}"))?
        .ip();

    Ok(format!("{}:{}", local_ip, udp_listen_port))
}

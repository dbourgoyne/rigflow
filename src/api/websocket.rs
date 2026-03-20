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
    let mut broadcast_rx = state.tx.subscribe();

    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Ready).unwrap().into(),
        ))
        .await;

    let send_task = tokio::spawn(async move {
        while let Ok(msg) = broadcast_rx.recv().await {
            let text = match serde_json::to_string(&msg) {
                Ok(t) => t,
                Err(_) => continue,
            };

            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
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
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
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

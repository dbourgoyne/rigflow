//! WSJT-X UDP telemetry listener (port 2237).
//!
//! A dumb background thread: bind, receive datagrams, decode with
//! `rigflow_log::wsjtx`, and forward the `LoggedADIF` payload over an `mpsc`
//! channel. The UI thread drains it and inserts into whatever operator's store
//! is currently open (so FT8 QSOs auto-route to the active operator). The bind
//! is non-fatal — a taken port surfaces a status string, never a crash.

use std::net::UdpSocket;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use rigflow_log::wsjtx::{self, WsjtxMessage};

use crate::ui::state::UiState;

/// The WSJT-X bind address. Localhost is the common WSJT-X-on-same-host setup;
/// `0.0.0.0` also accepts a WSJT-X on another LAN host pointed here.
const WSJTX_UDP_ADDR: &str = "0.0.0.0:2237";

/// An event forwarded from the listener thread to the UI thread.
#[derive(Debug, Clone)]
pub enum LogEvent {
    /// A `LoggedADIF` datagram's ADIF text (header + one record).
    LoggedAdif(String),
}

/// Spawn the listener. Returns the receiver the UI thread drains each frame.
/// On bind failure the thread writes `wsjtx_listen_status` into `UiState` and
/// exits; the returned receiver simply never yields (its sender is dropped).
pub fn spawn_wsjtx_listener(state: Arc<Mutex<UiState>>) -> Receiver<LogEvent> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("wsjtx-udp".to_string())
        .spawn(move || run(state, tx));
    rx
}

fn set_status(state: &Arc<Mutex<UiState>>, msg: String) {
    if let Ok(mut s) = state.lock() {
        s.wsjtx_listen_status = msg;
    }
}

fn run(state: Arc<Mutex<UiState>>, tx: Sender<LogEvent>) {
    let socket = match UdpSocket::bind(WSJTX_UDP_ADDR) {
        Ok(s) => s,
        Err(e) => {
            set_status(
                &state,
                format!("WSJT-X UDP {WSJTX_UDP_ADDR} unavailable: {e}"),
            );
            return;
        }
    };
    set_status(&state, format!("Listening for WSJT-X on {WSJTX_UDP_ADDR}"));

    let mut buf = vec![0u8; 65536];
    loop {
        let n = match socket.recv_from(&mut buf) {
            Ok((n, _from)) => n,
            Err(e) => {
                // A transient recv error shouldn't kill the listener; a fatal
                // one (socket closed) will keep erroring — cap the churn by
                // reporting and continuing.
                set_status(&state, format!("WSJT-X recv error: {e}"));
                continue;
            }
        };
        if let Some(WsjtxMessage::LoggedAdif { adif, .. }) = wsjtx::decode(&buf[..n]) {
            // Receiver gone → the app is shutting down; stop the thread.
            if tx.send(LogEvent::LoggedAdif(adif)).is_err() {
                return;
            }
        }
    }
}

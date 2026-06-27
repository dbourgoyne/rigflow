//! # rigflow-client
//!
//! `rigflow-client` is the desktop UI for the rigflow SDR system.
//!
//! It connects to a `rigflow-server` instance over WebSocket for control,
//! receives audio and waterfall data over UDP, and provides an interactive
//! UI for tuning, demodulation, and visualization.
//!
//! ## Responsibilities
//!
//! - Connect to rigflow-server via WebSocket
//! - Acquire and control radios (frequency, mode, filters, etc.)
//! - Receive and play audio streams
//! - Render spectrum and waterfall displays
//! - Manage operator profiles and persistent settings
//!
//! ## Architecture
//!
//! The client is composed of three main subsystems:
//!
//! ### 1. UI (egui / eframe)
//!
//! - Immediate-mode UI built with egui
//! - Renders spectrum, waterfall, and control panels
//! - Sends control commands via channel to the networking layer
//!
//! ### 2. Control Plane (WebSocket)
//!
//! - Runs in a dedicated Tokio runtime
//! - Sends `ClientRadioMessage` commands to the server
//! - Receives `ServerRadioMessage` updates (radio list, runtime state, etc.)
//!
//! ### 3. Media Plane (UDP)
//!
//! - Audio and waterfall data are received via UDP
//! - Audio is buffered and played via CPAL
//! - Waterfall and spectrum data are rendered in real time
//!
//! ## Runtime Flow
//!
//! 1. Load persisted UI/operator state
//! 2. Start media runtime (audio + waterfall processing)
//! 3. Start WebSocket control task (async)
//! 4. Launch egui UI
//! 5. UI interacts with server through control channel
//!
//! ## Networking
//!
//! - WebSocket control:
//!
//!   ```text
//!   ws://<server-ip>:9000/ws
//!   ```
//!
//! - UDP media:
//!
//!   - Client registers its UDP address with the server
//!   - Server streams:
//!     - audio (i16 samples)
//!     - waterfall/spectrum data (FFT bins)
//!
//! ## Persistence
//!
//! The client stores per-operator settings, including:
//!
//! - last server connection
//! - demodulation preferences (bandwidth, pitch, etc.)
//! - waterfall display settings (zoom, normalization)
//! - bookmarks (frequency presets)
//!
//! Data is stored in a JSON file under the user config directory.
//!
//! ## Key Features
//!
//! - Click-to-tune spectrum display
//! - Zoomable spectrum and waterfall
//! - Multiple demod modes (WFM, NFM, AM, USB, LSB, CW)
//! - Adaptive or manual waterfall normalization
//! - Bookmark system with default auto-apply
//! - Low-latency UDP audio streaming
//!
//! ## Example Usage
//!
//! Start the server first (all sources are auto-discovered):
//!
//! ```bash
//! cargo run -p rigflow-server
//! ```
//!
//! Then run the client:
//!
//! ```bash
//! cargo run -p rigflow-client
//! ```
//!
//! In the UI:
//!
//! - Enter server IP
//! - Click Connect
//! - Select a radio
//! - Tune and operate
//!
//! ## Related Crates
//!
//! - `rigflow-server` — SDR backend and DSP processing
//! - `rigflow-core` — shared DSP, audio, and utilities
//! - `rigflow-protocol` — shared WebSocket protocol types

mod alsa_quiet;
mod audio_metrics;
mod audio_recorder;
mod client_runtime;
mod cw_decode;
mod cw_text;
mod digital_audio;
mod digital_rx;
mod digital_tx;
mod mic;
mod net;
mod persistence;
mod rigctl_server;
mod sidetone;
mod tci_server;
mod ui;
mod voice_keyer;
mod widgets;

use log::error;

use std::sync::{Arc, Mutex};

use eframe::NativeOptions;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::client_runtime::start_media_runtime;
use crate::net::control::ControlCommand;
use crate::net::websocket::websocket_control_task;
use crate::persistence::load_initial_ui_state;
use crate::ui::app::RigflowApp;
use crate::ui::state::UiState;

fn main() -> Result<(), eframe::Error> {
    // Initialize logging for both the UI process and background runtime tasks.
    // Default to quiet: our own crate at info, everything else at warn, so noisy
    // dependencies (winit, eframe, zbus/accesskit, ALSA/JACK device probing)
    // don't bury the useful lines. `RUST_LOG` overrides this for debugging.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,rigflow_client=info"),
    )
    .format_timestamp_millis()
    .init();

    // Mute libasound's harmless stderr diagnostics (e.g. find_matching_chmap).
    alsa_quiet::silence_alsa_errors();

    // Parse the command line up front so `--help` exits before any startup work.
    let window_size = parse_cli_or_exit();

    // Load persisted startup state first, then wrap it in shared UI state.
    let (initial_ui_state, persistence_store) = match load_initial_ui_state(None) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("failed to load persistent state: {err}");
            (
                UiState::default(),
                crate::persistence::PersistenceStore::new(
                    crate::persistence::resolve_config_dir(None)
                        .unwrap_or_else(|_| std::path::PathBuf::from(".")),
                ),
            )
        }
    };

    // Shared UI/application state used by the egui app and background tasks.
    let ui_state = Arc::new(Mutex::new(initial_ui_state));

    // Start the media runtime first so audio/waterfall infrastructure is ready
    // before the UI and WebSocket control plane begin interacting with it.
    let media_handles =
        start_media_runtime(Arc::clone(&ui_state)).expect("failed to start media runtime");

    // Outbound control channel from the UI into the WebSocket control task.
    let (ws_cmd_tx, ws_cmd_rx) = mpsc::unbounded_channel::<ControlCommand>();

    // Dedicated Tokio runtime for async networking/control tasks.
    let rt = Arc::new(Runtime::new().expect("failed to create tokio runtime"));

    {
        let ui_state_for_ws = Arc::clone(&ui_state);
        let media_cmd_tx_for_ws = media_handles.media_cmd_tx.clone();
        let audio_session_generation_for_ws = media_handles.audio_session_generation;

        rt.spawn(async move {
            if let Err(error) = websocket_control_task(
                ws_cmd_rx,
                ui_state_for_ws,
                media_cmd_tx_for_ws,
                audio_session_generation_for_ws,
            )
            .await
            {
                error!("WebSocket control task failed: {error}");
            }
        });
    }

    // CAT (Hamlib NET rigctl) server on 127.0.0.1:4532 for WSJT-X et al.  Reads
    // frequency/mode from UiState and issues control commands through the same
    // channel as the UI.
    {
        let ui_state_for_cat = Arc::clone(&ui_state);
        let cmd_tx_for_cat = ws_cmd_tx.clone();
        rt.spawn(async move {
            rigctl_server::RigctlServer::new(ui_state_for_cat, cmd_tx_for_cat)
                .run()
                .await;
        });
    }

    // TCI server on 127.0.0.1:40001 for TCI-capable digital apps (JTDX,
    // WSJT-X-Improved): carries CAT + PTT + RX/TX audio over one WebSocket, so
    // FT8 works with no virtual audio driver (no BlackHole) and no mic
    // permission.  Coexists with the rigctld CAT server above.
    {
        let ui_state_for_tci = Arc::clone(&ui_state);
        let cmd_tx_for_tci = ws_cmd_tx.clone();
        rt.spawn(async move {
            tci_server::TciServer::new(ui_state_for_tci, cmd_tx_for_tci)
                .run()
                .await;
        });
    }

    // Catch SIGINT (Ctrl-C) and SIGTERM (kill) so a terminal kill still releases
    // the radio and disconnects cleanly before the process exits, mirroring the
    // window-[X] path.  (SIGKILL is uncatchable; the server's heartbeat and the
    // rig's TX watchdog are the backstop for that.)
    {
        let sig_cmd_tx = ws_cmd_tx.clone();
        rt.spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                let mut term = signal(SignalKind::terminate()).ok();
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = async {
                        match term.as_mut() {
                            Some(t) => {
                                t.recv().await;
                            }
                            None => std::future::pending::<()>().await,
                        }
                    } => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }

            log::info!("shutdown signal received: releasing radio and disconnecting");
            let _ = sig_cmd_tx.send(ControlCommand::ReleaseRadio);
            let _ = sig_cmd_tx.send(ControlCommand::Disconnect);
            // Give the WebSocket task a moment to flush both before we exit.
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            std::process::exit(0);
        });
    }

    let options = NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size(window_size),
        ..NativeOptions::default()
    };

    eframe::run_native(
        "rigflow-client",
        options,
        Box::new(move |cc| {
            // Pin the dark theme: the UI is dark-by-design (black spectrum,
            // waterfall, and center panel), and many status-bar labels rely on
            // the theme's default text color. Without this, a macOS host set to
            // Light appearance renders that text dark-on-black (e.g. the VFO
            // frequency vanishes). Setting a preference also stops eframe from
            // following later OS appearance changes.
            cc.egui_ctx.set_theme(eframe::egui::ThemePreference::Dark);
            Ok(Box::new(RigflowApp::new(
                Arc::clone(&ui_state),
                ws_cmd_tx.clone(),
                media_handles.waterfall_buffer.clone(),
                media_handles.spectrum_db.clone(),
                media_handles.waterfall_buffer_b.clone(),
                media_handles.spectrum_db_b.clone(),
                persistence_store,
            )))
        }),
    )
}

/// Default initial window size (logical points): 1280×720, comfortable on
/// modern displays.
const DEFAULT_WINDOW_SIZE: [f32; 2] = [1280.0, 720.0];

/// Parse the command line, returning the initial window size in logical points.
///
/// Supported flags:
///   `-h`, `--help`                 print usage and exit 0
///   `-w`, `--window-size <WxH>`    initial window size, e.g. `1600x900`
///
/// On `--help` this prints and exits 0; on a bad argument it prints to stderr and
/// exits 2 (matching the server's CLI convention).
fn parse_cli_or_exit() -> [f32; 2] {
    let mut args = std::env::args().skip(1);
    let mut size = DEFAULT_WINDOW_SIZE;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "rigflow-client\n\n\
                     USAGE:\n    \
                     rigflow-client [OPTIONS]\n\n\
                     OPTIONS:\n    \
                     -w, --window-size <WxH>    Initial window size in pixels \
                     (default {}x{})\n    \
                     -h, --help                 Print this help and exit",
                    DEFAULT_WINDOW_SIZE[0] as u32, DEFAULT_WINDOW_SIZE[1] as u32,
                );
                std::process::exit(0);
            }
            "-w" | "--window-size" => {
                let value = args.next().unwrap_or_else(|| {
                    eprintln!("error: {arg} requires a value, e.g. 1280x720");
                    std::process::exit(2);
                });
                size = parse_window_size(&value).unwrap_or_else(|msg| {
                    eprintln!("error: {msg}");
                    std::process::exit(2);
                });
            }
            // macOS injects a `-psn_<...>` process-serial-number arg when launching
            // an .app bundle; ignore it rather than erroring.
            other if other.starts_with("-psn_") => {}
            other => {
                eprintln!("error: unrecognized argument '{other}' (try --help)");
                std::process::exit(2);
            }
        }
    }

    size
}

/// Parse a `<WIDTH>x<HEIGHT>` string (e.g. `1280x720`) into a window size.
/// Width and height must be positive integers.
fn parse_window_size(value: &str) -> Result<[f32; 2], String> {
    let (w, h) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("invalid window size '{value}'; expected <WIDTH>x<HEIGHT>"))?;
    let w: u32 = w
        .trim()
        .parse()
        .map_err(|_| format!("invalid width in '{value}'"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| format!("invalid height in '{value}'"))?;
    if w == 0 || h == 0 {
        return Err(format!("window size '{value}' must be positive"));
    }
    Ok([w as f32, h as f32])
}

#[cfg(test)]
mod cli_tests {
    use super::{DEFAULT_WINDOW_SIZE, parse_window_size};

    #[test]
    fn parses_valid_size() {
        assert_eq!(parse_window_size("1600x900").unwrap(), [1600.0, 900.0]);
        assert_eq!(parse_window_size("1280X720").unwrap(), [1280.0, 720.0]);
    }

    #[test]
    fn rejects_bad_sizes() {
        assert!(parse_window_size("1280").is_err());
        assert!(parse_window_size("0x720").is_err());
        assert!(parse_window_size("axb").is_err());
    }

    #[test]
    fn default_is_1280x720() {
        assert_eq!(DEFAULT_WINDOW_SIZE, [1280.0, 720.0]);
    }
}

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
mod ui;
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
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    // Mute libasound's harmless stderr diagnostics (e.g. find_matching_chmap).
    alsa_quiet::silence_alsa_errors();

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

    let options = NativeOptions::default();

    eframe::run_native(
        "rigflow-client",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(RigflowApp::new(
                Arc::clone(&ui_state),
                ws_cmd_tx.clone(),
                media_handles.waterfall_buffer.clone(),
                media_handles.spectrum_db.clone(),
                persistence_store,
            )))
        }),
    )
}

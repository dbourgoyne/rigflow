mod client_runtime;
mod net;
mod ui;
mod widgets;

use log::error;

use std::sync::{Arc, Mutex};

use eframe::NativeOptions;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::ui::state::UiState;
use crate::client_runtime::start_media_runtime;
use crate::ui::app::RigflowApp;
use crate::net::control::ControlCommand;
use crate::net::websocket::websocket_control_task;

fn main() -> Result<(), eframe::Error> {
    // Initialize logging for both the UI process and background runtime tasks.
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    // Shared UI/application state used by the egui app and background tasks.
    let ui_state = Arc::new(Mutex::new(UiState::default()));

    // Start the media runtime first so audio/waterfall infrastructure is ready
    // before the UI and WebSocket control plane begin interacting with it.
    let media_handles = start_media_runtime(Arc::clone(&ui_state))
        .expect("failed to start media runtime");

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
            )))
        }),
    )
}

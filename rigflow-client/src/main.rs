mod app;
mod client_runtime;
mod eframe_app;
mod input;
mod net;
mod render;
mod widgets;

use std::sync::{Arc, Mutex};

use eframe::NativeOptions;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::app::state::UiState;
use crate::client_runtime::start_media_runtime;
use crate::eframe_app::RigflowApp;
use crate::net::control::ControlCommand;
use crate::net::websocket::websocket_control_task;

fn main() -> Result<(), eframe::Error> {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    let ui_state = Arc::new(Mutex::new(UiState::default()));

    let media_handles = start_media_runtime(Arc::clone(&ui_state))
        .expect("failed to start media runtime");

    let (ws_cmd_tx, ws_cmd_rx) = mpsc::unbounded_channel::<ControlCommand>();

    let rt = Arc::new(Runtime::new().expect("failed to create tokio runtime"));

    {
        let ui_state_for_ws = Arc::clone(&ui_state);
        let media_cmd_tx_for_ws = media_handles.media_cmd_tx.clone();

        rt.spawn(async move {
            if let Err(e) = websocket_control_task(
                ws_cmd_rx,
                ui_state_for_ws,
                media_cmd_tx_for_ws,
            )
            .await
            {
                eprintln!("WebSocket control task failed: {e}");
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

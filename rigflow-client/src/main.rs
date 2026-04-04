mod app;
mod net;
mod input;
mod render;
mod widgets;
mod eframe_app;

use std::sync::{Arc, Mutex};

use eframe::NativeOptions;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::app::state::UiState;
use crate::eframe_app::RigflowApp;
use crate::net::control::ControlCommand;
use crate::net::websocket::websocket_control_task;


fn main() -> Result<(), eframe::Error> {
    env_logger::Builder::from_default_env()
        .format_timestamp_millis()
        .init();

    let ui_state = Arc::new(Mutex::new(UiState::default()));

    let (ws_cmd_tx, ws_cmd_rx) = mpsc::unbounded_channel::<ControlCommand>();

    let rt = Arc::new(Runtime::new().expect("failed to create tokio runtime"));

    {
        let ui_state_for_ws = Arc::clone(&ui_state);
        let rt_for_ws = Arc::clone(&rt);

        rt_for_ws.spawn(async move {
            if let Err(e) = websocket_control_task(ws_cmd_rx, ui_state_for_ws).await {
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
            )))
        }),
    )
}

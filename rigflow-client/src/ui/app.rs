use std::sync::{Arc, Mutex};

use eframe::egui;
use tokio::sync::mpsc;

use crate::net::control::ControlCommand;

use crate::persistence::PersistenceStore;

use crate::ui::state::UiState;

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
    pub persistence_store: PersistenceStore,
    pub waterfall_texture: Option<egui::TextureHandle>,
}

impl RigflowApp {
    pub fn new(
	state: Arc<Mutex<UiState>>,
	ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
	waterfall_buffer: Arc<Mutex<Vec<u32>>>,
	spectrum_db: Arc<Mutex<Vec<f32>>>,
	persistence_store: PersistenceStore,
    ) -> Self {
	Self {
            state,
            ws_cmd_tx,
            waterfall_buffer,
            spectrum_db,
            persistence_store,
            waterfall_texture: None,
	}
    }
}

impl eframe::App for RigflowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

	let snapshot = self.snapshot_state();
	let config_mode = !snapshot.server_connected;

	self.handle_keyboard_shortcuts(ctx);
	self.draw_left_panel(ctx, &snapshot, config_mode);
	self.draw_center_panel(ctx, &snapshot);
	self.draw_add_operator_dialog(ctx);
	self.draw_add_bookmark_dialog(ctx);

        ctx.request_repaint();
    }
}

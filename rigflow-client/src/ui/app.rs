use std::sync::{Arc, Mutex};

use eframe::egui;
use tokio::sync::mpsc;

use crate::net::control::ControlCommand;
use rigflow_protocol::radio_control::ClientRadioMessage;

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

    fn snapshot_state(&self) -> UiState {
        let state = self.state.lock().unwrap();
        state.clone()
    }

    pub(crate) fn send_radio_msg(&self, msg: ClientRadioMessage) {
        let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(msg));
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
        let mut center_delta_hz: f32 = 0.0;

        ctx.input(|input| {
            let step = if input.modifiers.shift {
                1_000_000.0
            } else {
                25_000.0
            };

            if input.key_pressed(egui::Key::ArrowUp) {
                center_delta_hz += step;
            }

            if input.key_pressed(egui::Key::ArrowDown) {
                center_delta_hz -= step;
            }
        });

        if center_delta_hz != 0.0 {
            let mut send_center: Option<u64> = None;

            if let Ok(mut state) = self.state.lock() {
                let limits = crate::ui::freq_limits::active_freq_limits(&state);
                let new_center = crate::ui::freq_limits::clamp_center(
                    state.center_freq_hz + center_delta_hz,
                    &limits,
                );
                state.center_freq_hz = new_center;

                if state.radio_acquired {
                    send_center = Some(new_center as u64);
                }
            }

            if let Some(hz) = send_center {
                let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                    rigflow_protocol::ClientRadioMessage::SetCenterFrequency {
                        center_freq_hz: hz as u64,
                    },
                ));
            }
        }

        let mut target_delta_hz: f32 = 0.0;

        ctx.input(|input| {
            let step = if input.modifiers.shift { 1_000.0 } else { 10.0 };

            if input.key_pressed(egui::Key::ArrowRight) {
                target_delta_hz += step;
            }

            if input.key_pressed(egui::Key::ArrowLeft) {
                target_delta_hz -= step;
            }
        });

        if target_delta_hz != 0.0 {
            let mut send_target: Option<u64> = None;

            if let Ok(mut state) = self.state.lock() {
                let limits = crate::ui::freq_limits::active_freq_limits(&state);
                let new_target = crate::ui::freq_limits::clamp_target(
                    state.target_freq_hz + target_delta_hz,
                    state.center_freq_hz,
                    state.input_sample_rate_hz,
                    &limits,
                );
                state.target_freq_hz = new_target;

                if state.radio_acquired {
                    send_target = Some(new_target as u64);
                }
            }

            if let Some(hz) = send_target {
                let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                    rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                        target_freq_hz: hz as u64,
                    },
                ));
            }
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
        self.draw_delete_operator_dialog(ctx);

        ctx.request_repaint();
    }
}

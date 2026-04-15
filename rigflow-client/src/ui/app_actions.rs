use super::app::RigflowApp;
use eframe::egui;

use crate::ControlCommand;
use crate::UiState;

impl RigflowApp {
    pub(crate) fn handle_keyboard_shortcuts(&mut self, ctx: &egui::Context) {
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
		let new_center = (state.center_freq_hz + center_delta_hz).max(0.0);
		state.center_freq_hz = new_center;

		if state.radio_acquired {
                    send_center = Some(new_center as u64);
		}
            }

            if let Some(hz) = send_center {
		let _ = self.ws_cmd_tx.send(
                    ControlCommand::LegacyClientMessage(
			rigflow_protocol::ClientMessage::SetCenterFrequency {
                            center_freq_hz: hz as f32,
			},
                    ),
		);
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
		let new_target = (state.target_freq_hz + target_delta_hz).max(0.0);
		state.target_freq_hz = new_target;

		if state.radio_acquired {
                    send_target = Some(new_target as u64);
		}
            }

            if let Some(hz) = send_target {
		let _ = self.ws_cmd_tx.send(
                    ControlCommand::LegacyClientMessage(
			rigflow_protocol::ClientMessage::SetFrequency {
                            target_freq_hz: hz as f32,
			},
                    ),
		);
            }
	}
    }

    pub(crate) fn snapshot_state(&self) -> UiState {
	let state = self.state.lock().unwrap();
	state.clone()
    }
}

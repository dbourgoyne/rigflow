use eframe::egui;

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::app::state::UiState;
use crate::net::control::ControlCommand;
use crate::spectrum_view::draw_spectrum_plot;

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
}

impl RigflowApp {
    pub fn new(
        state: Arc<Mutex<UiState>>,
        ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
        waterfall_buffer: Arc<Mutex<Vec<u32>>>,
        spectrum_db: Arc<Mutex<Vec<f32>>>,
    ) -> Self {
        Self {
            state,
            ws_cmd_tx,
            waterfall_buffer,
            spectrum_db,
        }
    }
}

impl eframe::App for RigflowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = {
            let state = self.state.lock().unwrap();
            state.clone()
        };

	egui::CentralPanel::default().show(ctx, |ui| {
	    let available = ui.available_size();
	    let spectrum_height = (available.y * 0.35).max(140.0);

	    ui.heading("Spectrum");
	    ui.separator();

	    ui.allocate_ui_with_layout(
		egui::vec2(available.x, spectrum_height),
		egui::Layout::top_down(egui::Align::Min),
		|ui| {
		    let spectrum_snapshot = {
			let guard = self.spectrum_db.lock().unwrap();
			guard.clone()
		    };

		    let spectrum_snapshot = {
			let guard = self.spectrum_db.lock().unwrap();
			guard.clone()
		    };

		    draw_spectrum_plot(
			ui,
			&spectrum_snapshot,
			-120.0,
			0.0,
			snapshot.center_freq_hz,
			snapshot.input_sample_rate_hz,
		    );
		},
	    );

	    ui.separator();
	    ui.label("Waterfall placeholder");
	});

	ctx.request_repaint(); 

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("rigflow");
                ui.separator();

                egui::CollapsingHeader::new("Rigflow Server")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label("rigflow server IP:");

                        let mut ip = snapshot.rigflow_server_ip.clone();
                        if ui.text_edit_singleline(&mut ip).changed() {
                            if let Ok(mut state) = self.state.lock() {
                                state.rigflow_server_ip = ip;
                            }
                        }

                        ui.add_space(8.0);

                        let button_text = if snapshot.server_connected {
                            "Disconnect"
                        } else {
                            "Connect"
                        };

                        if ui.button(button_text).clicked() {
                            let ip = snapshot.rigflow_server_ip.trim().to_string();

                            if snapshot.server_connected {
                                let _ = self.ws_cmd_tx.send(ControlCommand::Disconnect);
                            } else if ip.is_empty() {
                                if let Ok(mut state) = self.state.lock() {
                                    state.server_status =
                                        "connect failed: missing server IP".to_string();
                                }
                            } else {
                                let _ = self.ws_cmd_tx.send(ControlCommand::Connect {
                                    server_ip: ip,
                                });
                            }
                        }

                        ui.add_space(8.0);
                        ui.label("Status:");
                        ui.monospace(&snapshot.server_status);
                    });

                ui.separator();

		egui::CollapsingHeader::new("Radios")
		    .default_open(true)
		    .show(ui, |ui| {
			if snapshot.available_radios.is_empty() {
			    ui.label("no radios");
			} else {
			    let mut selected = snapshot.selected_radio_id.clone();

			    for radio in &snapshot.available_radios {
				let label = if radio.is_leased {
				    format!("{} (busy)", radio.display_name)
				} else {
				    radio.display_name.clone()
				};

				let is_selected = selected.as_deref() == Some(&radio.id.0);

				if ui.selectable_label(is_selected, label).clicked() {
				    selected = Some(radio.id.0.clone());
				}
			    }

			    if selected != snapshot.selected_radio_id {
				if let Ok(mut state) = self.state.lock() {
				    state.selected_radio_id = selected.clone();
				}
			    }

			    ui.add_space(8.0);

			    let can_acquire = selected.is_some() && !snapshot.radio_acquired;
			    let can_release = snapshot.radio_acquired;

			    ui.horizontal(|ui| {
				if ui
				    .add_enabled(can_acquire, egui::Button::new("Acquire"))
				    .clicked()
				{
				    if let Some(radio_id) = selected.clone() {
					let _ = self.ws_cmd_tx.send(ControlCommand::AcquireRadio { radio_id });
				    }
				}

				if ui
				    .add_enabled(can_release, egui::Button::new("Release"))
				    .clicked()
				{
				    let _ = self.ws_cmd_tx.send(ControlCommand::ReleaseRadio);
				}
			    });
			}
		    });
            });
    }
}

use eframe::egui;

use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::app::state::UiState;
use crate::net::control::ControlCommand;
use crate::spectrum_view::draw_spectrum_plot;
use crate::app::layout::{HEIGHT, WIDTH, WATERFALL_TOP, SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1, LEFT_GUTTER, RIGHT_GUTTER};

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
    pub waterfall_texture: Option<egui::TextureHandle>,
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
	    waterfall_texture: None,
        }
    }

    fn update_waterfall_texture(
	&mut self,
	ctx: &egui::Context,
	width: usize,
	height: usize,
	waterfall_top: usize,
	x0: usize,
	x1: usize,
    ) {
	let pixels = {
            let guard = self.waterfall_buffer.lock().unwrap();
            guard.clone()
	};

	if pixels.len() != width * height || waterfall_top >= height || x0 >= x1 || x1 > width {
            return;
	}

	let wf_height = height - waterfall_top;
	let wf_width = x1 - x0;

	let mut image = egui::ColorImage::new(
            [wf_width, wf_height],
            egui::Color32::BLACK,
	);

	for y in 0..wf_height {
            let src_row = waterfall_top + y;
            let src_start = src_row * width + x0;
            let src_end = src_row * width + x1;

            let dst_start = y * wf_width;
            let dst_end = dst_start + wf_width;

            for (dst, src) in image.pixels[dst_start..dst_end]
		.iter_mut()
		.zip(pixels[src_start..src_end].iter())
            {
		let rgb = *src;

		let r = ((rgb >> 16) & 0xff) as u8;
		let g = ((rgb >> 8) & 0xff) as u8;
		let b = (rgb & 0xff) as u8;

		*dst = egui::Color32::from_rgb(r, g, b);
            }
	}

	match &mut self.waterfall_texture {
            Some(tex) => {
		tex.set(image, egui::TextureOptions::NEAREST);
            }
            None => {
		let tex = ctx.load_texture(
                    "waterfall_texture",
                    image,
                    egui::TextureOptions::NEAREST,
		);
		self.waterfall_texture = Some(tex);
            }
	}
    }
}

impl eframe::App for RigflowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = {
            let state = self.state.lock().unwrap();
            state.clone()
        };

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

		if snapshot.radio_acquired {
		    egui::CollapsingHeader::new("Radio Control")
			.default_open(true)
			.show(ui, |ui| {
			    ui.label("Demod");

			    let mut selected_demod = snapshot.demod_mode.clone();

			    ui.horizontal(|ui| {
				ui.radio_value(&mut selected_demod, "wfm".to_string(), "wfm");
				ui.radio_value(&mut selected_demod, "nfm".to_string(), "nfm");
				ui.radio_value(&mut selected_demod, "lsb".to_string(), "lsb");
				ui.radio_value(&mut selected_demod, "usb".to_string(), "usb");
			    });

			    if selected_demod != snapshot.demod_mode {
				if let Ok(mut state) = self.state.lock() {
				    state.demod_mode = selected_demod.clone();
				    state.sideband = match selected_demod.as_str() {
					"lsb" => "lsb".to_string(),
					"usb" => "usb".to_string(),
					_ => state.sideband.clone(),
				    };
				}

				let _ = self.ws_cmd_tx.send(
				    crate::net::control::ControlCommand::LegacyClientMessage(
					rigflow_protocol::ClientMessage::SetDemodMode {
					    mode: selected_demod.clone(),
					},
				    ),
				);

				if selected_demod == "lsb" || selected_demod == "usb" {
				    let _ = self.ws_cmd_tx.send(
					crate::net::control::ControlCommand::LegacyClientMessage(
					    rigflow_protocol::ClientMessage::SetSideband {
						sideband: selected_demod,
					    },
					),
				    );
				}
			    }
			});
		}
            });

	
	egui::CentralPanel::default().show(ctx, |ui| {
	    egui::Frame::NONE
		.inner_margin(egui::Margin {
		    left: 12,
		    right: 12,
		    top: 4,
		    bottom: 4,
		})
		.show(ui, |ui| {
		    let spectrum_snapshot = {
			let guard = self.spectrum_db.lock().unwrap();
			guard.clone()
		    };

		    /*
		    println!(
			"demod_mode={:?} sideband={:?} target={} center={} sample_rate={}",
			snapshot.demod_mode,
			snapshot.sideband,
			snapshot.target_freq_hz,
			snapshot.center_freq_hz,
			snapshot.input_sample_rate_hz,
		);
		    */

		    if let Some(clicked_freq_hz) = draw_spectrum_plot(
			ui,
			egui::vec2(ui.available_width(), 220.0),
			&spectrum_snapshot,
			-120.0,
			0.0,
			snapshot.center_freq_hz,
			snapshot.target_freq_hz,
			snapshot.input_sample_rate_hz,
			&snapshot.demod_mode,
			&snapshot.sideband,
		    ) {
			println!("UI clicked spectrum at {}", clicked_freq_hz);
			let _ = self.ws_cmd_tx.send(
			    crate::net::control::ControlCommand::LegacyClientMessage(
				rigflow_protocol::ClientMessage::SetFrequency {
				    target_freq_hz: clicked_freq_hz,
				},
			    ),
			);
		    }
		    
		    ui.add_space(4.0);
		    ui.separator();
		    self.update_waterfall_texture(
			ctx,
			WIDTH,
			HEIGHT,
			WATERFALL_TOP,
			SPECTRUM_PLOT_X0,
			SPECTRUM_PLOT_X1,
		    );

		    if let Some(tex) = &self.waterfall_texture {
			let wf_height = (HEIGHT - WATERFALL_TOP) as f32;
			let image_width = ui.available_width();
//			let image_width = (ui.available_width() - LEFT_GUTTER - RIGHT_GUTTER);
//			let image_width = (ui.available_width() - LEFT_GUTTER - RIGHT_GUTTER).max(100.0);

			ui.horizontal(|ui| {
			    ui.add_space(LEFT_GUTTER);
			    ui.image((tex.id(), egui::vec2(image_width, wf_height)));
			    ui.add_space(RIGHT_GUTTER);
			});
		    }
		});

	    // Update immediately, don't wait for server response
	    // Makes UI feel snappier
	    /*
	    if let Some(clicked_freq_hz) = draw_spectrum_plot(...) {
		if let Ok(mut state) = self.state.lock() {
		    state.target_freq_hz = clicked_freq_hz;
		}

		let _ = self.ws_cmd_tx.send(
		    crate::net::control::ControlCommand::LegacyClientMessage(
			rigflow_protocol::ClientMessage::SetFrequency {
			    target_freq_hz: clicked_freq_hz,
			},
		    ),
		);
	}
	    */
	});

	ctx.request_repaint(); 

    }


}

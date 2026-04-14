use std::sync::{Arc, Mutex};

use log::warn;
use eframe::egui;
use tokio::sync::mpsc;

use crate::net::control::ControlCommand;

use crate::persistence::{
    apply_operator_settings_to_ui_state,
    apply_ui_state_to_operator_settings,
    normalize_operator_id,
    operator_file_path,
    BookmarkDisplaySettingsFile,
    BookmarkFile,
    PersistenceStore,
};

use crate::ui::{
    layout::{
        LEFT_GUTTER, RIGHT_GUTTER, WATERFALL_IMAGE_HEIGHT, WATERFALL_IMAGE_WIDTH,
    },
    om_bands::LicenseClass,
    spectrum_view::{
        draw_spectrum_plot, x_frac_to_frequency_hz, zoomed_visible_freq_range_hz,
        SpectrumInteraction,
    },
    state::UiState,
};
use rigflow_core::dsp::modes::{DemodMode, Sideband};

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

    fn save_selected_operator_license(&mut self) {
	let (operator_id, selected_license) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.selected_license)
	};

	if operator_id.trim().is_empty() {
            return;
	}

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load operator: {err}");
                    }
                    return;
		}
            };

	operator_settings.selected_license = selected_license;

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save operator license: {err}");
            }
	}
    }

    fn delete_operator(&mut self, operator_id: &str) {
	use std::fs;

	let operator_id = match normalize_operator_id(operator_id) {
            Ok(id) => id,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("invalid operator id: {err}");
		}
		return;
            }
	};

	let path = operator_file_path(self.persistence_store.config_dir(), &operator_id);

	if let Err(err) = fs::remove_file(&path) {
            if err.kind() != std::io::ErrorKind::NotFound {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to delete operator file: {err}");
		}
		return;
            }
	}

	let mut app_state = match self.persistence_store.load_app_state() {
            Ok(app_state) => app_state,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to load app state: {err}");
		}
		return;
            }
	};

	app_state.known_operator_ids.retain(|id| id != &operator_id);
	let next_operator = app_state.known_operator_ids.first().cloned();
	app_state.last_operator_id = next_operator.clone();

	if let Err(err) = self.persistence_store.save_app_state(&app_state) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save app state: {err}");
            }
            return;
	}

	if let Ok(mut state) = self.state.lock() {
            state.show_delete_operator_dialog = false;
            state.pending_delete_operator_id = None;
	}

	if let Some(next_operator_id) = next_operator {
            match self.persistence_store.load_or_create_operator_settings(&next_operator_id) {
		Ok(operator_settings) => {
                    if let Ok(mut state) = self.state.lock() {
			apply_operator_settings_to_ui_state(
                            &mut state,
                            &operator_settings,
                            &app_state,
			);
			state.persistence_status.clear();
                    }
		}
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load replacement operator: {err}");
                    }
		}
            }
	} else if let Ok(mut state) = self.state.lock() {
            let mut new_state = UiState::default();
            new_state.known_operator_ids = Vec::new();
            new_state.persistence_status.clear();
            *state = new_state;
	}
    }
    
    fn delete_selected_bookmark(&mut self) {
	let selected_id = {
            let state = self.state.lock().unwrap();
            state.selected_bookmark_id.clone()
	};

	let Some(selected_id) = selected_id else {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status = "no bookmark selected".to_string();
            }
            return;
	};

	if let Ok(mut state) = self.state.lock() {
            let before_len = state.bookmarks.len();
            state.bookmarks.retain(|bookmark| bookmark.id != selected_id);

            if state.bookmarks.len() == before_len {
		state.bookmark_status = "bookmark not found".to_string();
		return;
            }

            if state
		.default_bookmark_id
		.as_ref()
		.map(|id| id == &selected_id)
		.unwrap_or(false)
            {
		state.default_bookmark_id = None;
            }

            state.selected_bookmark_id = None;
            state.bookmark_status.clear();
	}

	self.save_bookmarks_to_current_operator();
    }

    fn set_default_bookmark(&mut self, bookmark_id: &str) {
	if let Ok(mut state) = self.state.lock() {
            state.default_bookmark_id = Some(bookmark_id.to_string());
            state.bookmark_status.clear();
	}

	self.save_bookmarks_to_current_operator();
    }

    fn apply_bookmark(&mut self, bookmark_id: &str) {
	let bookmark = {
            let state = self.state.lock().unwrap();
            state
		.bookmarks
		.iter()
		.find(|b| b.id == bookmark_id)
		.cloned()
	};

	let Some(bookmark) = bookmark else {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status = "bookmark not found".to_string();
            }
            return;
	};

	{

	    let center_freq_hz = bookmark.frequency_hz;

	    {
		let mut state = self.state.lock().unwrap();

		state.center_freq_hz = center_freq_hz;
		state.target_freq_hz = bookmark.frequency_hz;
		state.demod_mode = bookmark.demod_mode;

		if let Some(sideband) = bookmark.sideband {
		    state.sideband = sideband;
		}

		if let Some(display) = &bookmark.display {
		    if let Some(zoom) = display.zoom {
			state.display_zoom = zoom;
		    }
		    if let Some(adaptive) = display.adaptive_waterfall_normalization {
			state.adaptive_waterfall_normalization = adaptive;
		    }
		    if let Some(top_db) = display.waterfall_top_db {
			state.display_top_db = top_db;
		    }
		    if let Some(range_db) = display.waterfall_range_db {
			state.display_range_db = range_db;
		    }
		}

		state.selected_bookmark_id = Some(bookmark.id.clone());
		state.bookmark_status.clear();
	    }

	    let _ = self.ws_cmd_tx.send(
		ControlCommand::LegacyClientMessage(
		    rigflow_protocol::ClientMessage::SetCenterFrequency {
			center_freq_hz,
		    },
		),
	    );

	    let _ = self.ws_cmd_tx.send(
		ControlCommand::LegacyClientMessage(
		    rigflow_protocol::ClientMessage::SetFrequency {
			target_freq_hz: bookmark.frequency_hz,
		    },
		),
	    );

	    let _ = self.ws_cmd_tx.send(
		ControlCommand::LegacyClientMessage(
		    rigflow_protocol::ClientMessage::SetDemodMode {
			mode: bookmark.demod_mode,
		    },
		),
	    );

	    if let Some(sideband) = bookmark.sideband {
		let _ = self.ws_cmd_tx.send(
		    ControlCommand::LegacyClientMessage(
			rigflow_protocol::ClientMessage::SetSideband { sideband },
		    ),
		);
	    }
	}
    }

    fn save_current_as_bookmark(&mut self) {
	let (
            name,
            target_freq_hz,
            demod_mode,
            sideband,
            zoom,
            adaptive_waterfall_normalization,
            display_top_db,
            display_range_db,
            existing_ids,
	) = {
            let state = self.state.lock().unwrap();

            (
		state.pending_bookmark_name.trim().to_string(),
		state.target_freq_hz,
		state.demod_mode,
		state.sideband,
		state.display_zoom,
		state.adaptive_waterfall_normalization,
		state.display_top_db,
		state.display_range_db,
		state
                    .bookmarks
                    .iter()
                    .map(|b| b.id.clone())
                    .collect::<Vec<_>>(),
            )
	};

	if name.is_empty() {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status = "bookmark name cannot be empty".to_string();
            }
            return;
	}

	let mut bookmark_id = Self::make_bookmark_id(&name);
	if existing_ids.iter().any(|id| id == &bookmark_id) {
            let mut suffix = 2;
            while existing_ids.iter().any(|id| id == &format!("{bookmark_id}-{suffix}")) {
		suffix += 1;
            }
            bookmark_id = format!("{bookmark_id}-{suffix}");
	}

	let bookmark = BookmarkFile {
            id: bookmark_id.clone(),
            name,
            frequency_hz: target_freq_hz,
            demod_mode,
            sideband: Some(sideband),
            display: Some(BookmarkDisplaySettingsFile {
		zoom: Some(zoom),
		adaptive_waterfall_normalization: Some(
                    adaptive_waterfall_normalization,
		),
		waterfall_top_db: Some(display_top_db),
		waterfall_range_db: Some(display_range_db),
            }),
            notes: None,
	};

	if let Ok(mut state) = self.state.lock() {
            state.bookmarks.push(bookmark);
            state.selected_bookmark_id = Some(bookmark_id);
            state.show_add_bookmark_dialog = false;
            state.pending_bookmark_name.clear();
            state.bookmark_status.clear();
	}

	self.save_bookmarks_to_current_operator();
    }

    fn make_bookmark_id(name: &str) -> String {
	let mut id = String::new();

	for ch in name.trim().chars() {
            if ch.is_ascii_alphanumeric() {
		id.push(ch.to_ascii_lowercase());
            } else if ch == ' ' || ch == '-' || ch == '_' {
		if !id.ends_with('-') {
                    id.push('-');
		}
            }
	}

	let id = id.trim_matches('-').to_string();

	if id.is_empty() {
            "bookmark".to_string()
	} else {
            id
	}
    }

    fn save_bookmarks_to_current_operator(&mut self) {
	let operator_id = {
            let state = self.state.lock().unwrap();
            state.operator_id.clone()
	};

	if operator_id.trim().is_empty() {
            return;
	}

	let operator_id = match normalize_operator_id(&operator_id) {
            Ok(id) => id,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.bookmark_status = format!("invalid operator id: {err}");
		}
		return;
            }
	};

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.bookmark_status =
                            format!("failed to load operator settings: {err}");
                    }
                    return;
		}
            };

	{
            let state = self.state.lock().unwrap();
            apply_ui_state_to_operator_settings(&state, &mut operator_settings);
	}

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status =
                    format!("failed to save operator settings: {err}");
            }
	}
    }

    fn save_server_ip(&mut self) {
	let (operator_id, server_ip) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.rigflow_server_ip.clone())
	};

	if operator_id.trim().is_empty() {
            return;
	}

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load operator: {err}");
                    }
                    return;
		}
            };

	operator_settings.server_ip = server_ip;

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save server IP: {err}");
            }
	}
    }

    fn save_pending_operator(&mut self) {
	let (raw_operator_id, selected_license) = {
            let state = self.state.lock().unwrap();
            (
		state.pending_operator_id.clone(),
		state.pending_operator_license,
            )
	};

	let operator_id = match normalize_operator_id(&raw_operator_id) {
            Ok(id) => id,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("invalid operator id: {err}");
		}
		return;
            }
	};

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load/create operator: {err}");
                    }
                    return;
		}
            };

	operator_settings.selected_license = selected_license;

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save operator settings: {err}");
            }
            return;
	}

	let app_state = match self.persistence_store.upsert_known_operator(&operator_id) {
            Ok(app_state) => app_state,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to update known operators: {err}");
		}
		return;
            }
	};

	if let Ok(mut state) = self.state.lock() {
            apply_operator_settings_to_ui_state(
		&mut state,
		&operator_settings,
		&app_state,
            );

            state.show_add_operator_dialog = false;
            state.pending_operator_id.clear();
            state.pending_operator_license = None;
            state.persistence_status.clear();
	}
    }
    
    fn update_waterfall_texture(
        &mut self,
        ctx: &egui::Context,
        wf_width: usize,
        wf_height: usize,
    ) {
        let pixels = {
            let guard = self.waterfall_buffer.lock().unwrap();
            guard.clone()
        };

        if pixels.len() != wf_width * wf_height {
            warn!(
                "waterfall texture size mismatch: pixels={} expected={}",
                pixels.len(),
                wf_width * wf_height
            );
            return;
        }

        let mut image =
            egui::ColorImage::new([wf_width, wf_height], egui::Color32::BLACK);

        for (dst, src) in image.pixels.iter_mut().zip(pixels.iter()) {
            let rgb = *src;
            let r = ((rgb >> 16) & 0xff) as u8;
            let g = ((rgb >> 8) & 0xff) as u8;
            let b = (rgb & 0xff) as u8;
            *dst = egui::Color32::from_rgb(r, g, b);
        }

        match &mut self.waterfall_texture {
            Some(texture) => {
                texture.set(image, egui::TextureOptions::NEAREST);
            }
            None => {
                let texture = ctx.load_texture(
                    "waterfall_texture",
                    image,
                    egui::TextureOptions::NEAREST,
                );
                self.waterfall_texture = Some(texture);
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

        let mut center_delta_hz: f32 = 0.0;
	let config_mode = !snapshot.server_connected;

        ctx.input(|input| {
            let step = if input.modifiers.shift { 1_000_000.0 } else { 25_000.0 };

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

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("rigflow");
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {

			//-------- Radio operator ----------
			ui.collapsing("Radio Operator", |ui| {
			    if !config_mode {
				ui.label("Disconnect from the server to change operator settings.");
				ui.add_space(6.0);
			    }

			    ui.add_enabled_ui(config_mode, |ui| {
				ui.label("Current operator:");

				let mut selected_operator = if snapshot.operator_id.trim().is_empty() {
				    None
				} else {
				    Some(snapshot.operator_id.clone())
				};

				egui::ComboBox::from_id_salt("operator_combo")
				    .selected_text(
					selected_operator
					    .clone()
					    .unwrap_or_else(|| "none".to_string()),
				    )
				    .show_ui(ui, |ui| {
					for operator_id in &snapshot.known_operator_ids {
					    ui.selectable_value(
						&mut selected_operator,
						Some(operator_id.clone()),
						operator_id,
					    );
					}
				    });

				if selected_operator != Some(snapshot.operator_id.clone()) {
				    if let Some(operator_id) = selected_operator {
					match self.persistence_store.load_or_create_operator_settings(&operator_id) {
					    Ok(operator_settings) => {
						match self.persistence_store.load_app_state() {
						    Ok(mut app_state) => {
							app_state.last_operator_id = Some(operator_id.clone());

							if let Err(err) = self.persistence_store.save_app_state(&app_state) {
							    if let Ok(mut state) = self.state.lock() {
								state.persistence_status =
								    format!("failed to save app state: {err}");
							    }
							}

							if let Ok(mut state) = self.state.lock() {
							    apply_operator_settings_to_ui_state(
								&mut state,
								&operator_settings,
								&app_state,
							    );
							    state.persistence_status.clear();
							}
						    }
						    Err(err) => {
							if let Ok(mut state) = self.state.lock() {
							    state.persistence_status =
								format!("failed to load app state: {err}");
							}
						    }
						}
					    }
					    Err(err) => {
						if let Ok(mut state) = self.state.lock() {
						    state.persistence_status =
							format!("failed to load operator: {err}");
						}
					    }
					}
				    }
				}

				ui.add_space(8.0);

				ui.horizontal(|ui| {
				    if ui.button("Add Operator").clicked() {
					if let Ok(mut state) = self.state.lock() {
					    state.show_add_operator_dialog = true;
					    state.pending_operator_id.clear();
					    state.pending_operator_license = None;
					    state.persistence_status.clear();
					}
				    }

				    if ui
					.add_enabled(
					    !snapshot.operator_id.trim().is_empty(),
					    egui::Button::new("Delete Operator"),
					)
					.clicked()
				    {
					if let Ok(mut state) = self.state.lock() {
					    state.show_delete_operator_dialog = true;
					    state.pending_delete_operator_id = Some(state.operator_id.clone());
					    state.persistence_status.clear();
					}
				    }
				});

				ui.add_space(8.0);
				ui.label("License:");

				let mut selected_license = snapshot.selected_license;

				ui.radio_value(
				    &mut selected_license,
				    Some(LicenseClass::AmateurExtra),
				    "Amateur Extra",
				);
				ui.radio_value(
				    &mut selected_license,
				    Some(LicenseClass::Advanced),
				    "Advanced",
				);
				ui.radio_value(
				    &mut selected_license,
				    Some(LicenseClass::General),
				    "General",
				);
				ui.radio_value(
				    &mut selected_license,
				    Some(LicenseClass::Technician),
				    "Technician",
				);
				ui.radio_value(&mut selected_license, None, "None");

				if selected_license != snapshot.selected_license {
				    if let Ok(mut state) = self.state.lock() {
					state.selected_license = selected_license;
				    }
				    self.save_selected_operator_license();
				}
			    });

			    if !snapshot.persistence_status.is_empty() {
				ui.add_space(6.0);
				ui.colored_label(
				    egui::Color32::YELLOW,
				    &snapshot.persistence_status,
				);
			    }
			});

			//-------- Rigflow Server ----------
                        egui::CollapsingHeader::new("Rigflow Server")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label("rigflow server IP:");

				let mut ip = snapshot.rigflow_server_ip.clone();
				ui.add_enabled_ui(config_mode, |ui| {
				    if ui.text_edit_singleline(&mut ip).changed() {
					if let Ok(mut state) = self.state.lock() {
					    state.rigflow_server_ip = ip.clone();
					}

					self.save_server_ip();
				    }
				});
				
				ui.add_space(8.0);

				let button_text = if snapshot.server_connected {
				    "Disconnect"
				} else {
				    "Connect"
				};

                                if ui.button(button_text).clicked() {
                                    let ip =
                                        snapshot.rigflow_server_ip.trim().to_string();

                                    if snapshot.server_connected {
                                        let _ = self.ws_cmd_tx.send(
                                            ControlCommand::Disconnect,
                                        );
                                    } else if ip.is_empty() {
                                        if let Ok(mut state) = self.state.lock() {
                                            state.server_status =
                                                "connect failed: missing server IP"
                                                    .to_string();
                                        }
                                    } else {
                                        let _ = self.ws_cmd_tx.send(
                                            ControlCommand::Connect {
                                                server_ip: ip,
                                            },
                                        );
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
                                    let mut selected =
                                        snapshot.selected_radio_id.clone();

                                    for radio in &snapshot.available_radios {
                                        let label = if radio.is_leased {
                                            format!("{} (busy)", radio.display_name)
                                        } else {
                                            radio.display_name.clone()
                                        };

                                        let is_selected =
                                            selected.as_deref() == Some(&radio.id.0);

                                        if ui
                                            .selectable_label(is_selected, label)
                                            .clicked()
                                        {
                                            selected = Some(radio.id.0.clone());
                                        }
                                    }

                                    if selected != snapshot.selected_radio_id {
                                        if let Ok(mut state) = self.state.lock() {
                                            state.selected_radio_id = selected.clone();
                                        }
                                    }

                                    ui.add_space(8.0);

                                    let can_acquire =
                                        selected.is_some() && !snapshot.radio_acquired;
                                    let can_release = snapshot.radio_acquired;

                                    ui.horizontal(|ui| {
                                        if ui
                                            .add_enabled(
                                                can_acquire,
                                                egui::Button::new("Acquire"),
                                            )
                                            .clicked()
                                        {
                                            if let Some(radio_id) = selected.clone() {
                                                let _ = self.ws_cmd_tx.send(
                                                    ControlCommand::AcquireRadio {
                                                        radio_id,
                                                    },
                                                );
                                            }
                                        }

                                        if ui
                                            .add_enabled(
                                                can_release,
                                                egui::Button::new("Release"),
                                            )
                                            .clicked()
                                        {
                                            let _ = self.ws_cmd_tx.send(
                                                ControlCommand::ReleaseRadio,
                                            );
                                        }
                                    });
                                }
                            });

                        if snapshot.radio_acquired {
                            egui::CollapsingHeader::new("Radio Control")
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.label("Demod");

                                    let mut selected_demod =
                                        snapshot.demod_mode.clone();

                                    ui.horizontal(|ui| {
                                        ui.radio_value(
                                            &mut selected_demod,
                                            DemodMode::Wfm,
                                            "wfm",
                                        );
                                        ui.radio_value(
                                            &mut selected_demod,
                                            DemodMode::Nfm,
                                            "nfm",
                                        );
                                        ui.radio_value(
                                            &mut selected_demod,
                                            DemodMode::Lsb,
                                            "lsb",
                                        );
                                        ui.radio_value(
                                            &mut selected_demod,
                                            DemodMode::Usb,
                                            "usb",
                                        );
                                    });

                                    if selected_demod != snapshot.demod_mode {
                                        if let Ok(mut state) = self.state.lock() {
                                            state.demod_mode =
                                                selected_demod.clone();
                                            state.sideband = match selected_demod {
                                                DemodMode::Lsb => Sideband::Lsb,
                                                DemodMode::Usb => Sideband::Usb,
                                                _ => state.sideband,
                                            };
                                        }

                                        let _ = self.ws_cmd_tx.send(
                                            ControlCommand::LegacyClientMessage(
                                                rigflow_protocol::ClientMessage::SetDemodMode {
                                                    mode: selected_demod,
                                                },
                                            ),
                                        );

                                        if selected_demod == DemodMode::Lsb
                                            || selected_demod == DemodMode::Usb
                                        {
                                            let _ = self.ws_cmd_tx.send(
                                                ControlCommand::LegacyClientMessage(
                                                    rigflow_protocol::ClientMessage::SetSideband {
                                                        sideband: match selected_demod {
                                                            DemodMode::Lsb => Sideband::Lsb,
                                                            DemodMode::Usb => Sideband::Usb,
                                                            _ => unreachable!(
                                                                "sideband only sent for USB/LSB"
                                                            ),
                                                        },
                                                    },
                                                ),
                                            );
                                        }
                                    }
                                });
                        }

                        ui.collapsing("Waterfall Control", |ui| {
                            if let Ok(mut state) = self.state.lock() {
                                ui.add(
                                    egui::Slider::new(
                                        &mut state.display_zoom,
                                        1.0..=4.0,
                                    )
                                    .text("Zoom"),
                                );

                                ui.checkbox(
                                    &mut state.adaptive_waterfall_normalization,
                                    "Adaptive normalization",
                                );

                                let manual_enabled =
                                    !state.adaptive_waterfall_normalization;

                                ui.add_enabled_ui(manual_enabled, |ui| {
                                    ui.add(
                                        egui::Slider::new(
                                            &mut state.display_top_db,
                                            -120.0..=20.0,
                                        )
                                        .text("Top dB"),
                                    );

                                    ui.add(
                                        egui::Slider::new(
                                            &mut state.display_range_db,
                                            10.0..=120.0,
                                        )
                                        .text("Range dB"),
                                    );
                                });
                            } else {
                                ui.label("Waterfall controls unavailable");
                            }
                        });
			

			ui.separator();

			ui.collapsing("Bookmarks", |ui| {
			    let snapshot = {
				let state = self.state.lock().unwrap();
				state.clone()
			    };

			    if snapshot.bookmarks.is_empty() {
				ui.label("no bookmarks");
			    } else {
				for bookmark in &snapshot.bookmarks {
				    let selected = snapshot
					.selected_bookmark_id
					.as_ref()
					.map(|id| id == &bookmark.id)
					.unwrap_or(false);

				    let mut label = bookmark.name.clone();
				    if snapshot
					.default_bookmark_id
					.as_ref()
					.map(|id| id == &bookmark.id)
					.unwrap_or(false)
				    {
					label.push_str("  [default]");
				    }

				    if ui.selectable_label(selected, label).clicked() {
					if let Ok(mut state) = self.state.lock() {
					    state.selected_bookmark_id = Some(bookmark.id.clone());
					}
				    }
				}

				ui.add_space(8.0);

				ui.horizontal(|ui| {
				    let selected_id = snapshot.selected_bookmark_id.clone();

				    if ui
					.add_enabled(selected_id.is_some(), egui::Button::new("Apply"))
					.clicked()
				    {
					if let Some(bookmark_id) = selected_id.clone() {
					    self.apply_bookmark(&bookmark_id);
					}
				    }

				    if ui
					.add_enabled(selected_id.is_some(), egui::Button::new("Set Default"))
					.clicked()
				    {
					if let Some(bookmark_id) = selected_id.clone() {
					    self.set_default_bookmark(&bookmark_id);
					}
				    }

				    if ui
					.add_enabled(selected_id.is_some(), egui::Button::new("Delete"))
					.clicked()
				    {
					self.delete_selected_bookmark();
				    }
				});
			    }

			    ui.add_space(8.0);

			    if ui.button("Save Current as Bookmark").clicked() {
				if let Ok(mut state) = self.state.lock() {
				    state.show_add_bookmark_dialog = true;
				    state.pending_bookmark_name.clear();
				    state.bookmark_status.clear();
				}
			    }

			    if !snapshot.bookmark_status.is_empty() {
				ui.add_space(6.0);
				ui.colored_label(egui::Color32::YELLOW, &snapshot.bookmark_status);
			    }

			    let auto_apply_changed = if let Ok(mut state) = self.state.lock() {
				ui.checkbox(
				    &mut state.auto_apply_default_bookmark_on_acquire,
				    "Auto-apply default on radio acquire",
				)
				    .changed()
			    } else {
				false
			    };

			    if auto_apply_changed {
				self.save_bookmarks_to_current_operator();
			    }

			});
		    });
		
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::BLACK)
                .inner_margin(egui::Margin {
                    left: 12,
                    right: 12,
                    top: 4,
                    bottom: 4,
                })
                .show(ui, |ui| {
                    let lo_strip_height = 34.0;
                    let spectrum_height = 220.0;
                    let gap = 6.0;
                    let waterfall_height = (ui.available_height()
                        - lo_strip_height
                        - spectrum_height
                        - gap
                        - gap
                        - 2.0)
                        .max(120.0);

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), lo_strip_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let state_snapshot = {
                                let state = self.state.lock().unwrap();
                                state.clone()
                            };

                            let strip_rect = ui.max_rect();
                            let lo_y = strip_rect.top() + 2.0;

                            let lo_pos =
                                egui::Pos2::new(strip_rect.left() + 12.0, lo_y);
                            let lo_offset_pos =
                                egui::Pos2::new(strip_rect.right() - 12.0, lo_y);

                            let mut new_center_freq_hz = None;
                            let mut new_target_freq_hz = None;

                            if let Some(new_center_hz) =
                                crate::widgets::lo_frequency_widget::draw_lo_widget(
                                    ui,
                                    lo_pos,
                                    state_snapshot.center_freq_hz.max(0.0) as u64,
                                )
                            {
                                new_center_freq_hz = Some(new_center_hz as f32);
                            }

                            let lo_offset_hz = (state_snapshot.target_freq_hz
                                - state_snapshot.center_freq_hz)
                                .round() as i64;

                            if let Some(new_offset_hz) =
                                crate::widgets::lo_frequency_widget::draw_lo_offset_widget(
                                    ui,
                                    lo_offset_pos,
                                    lo_offset_hz,
                                )
                            {
                                let new_target = (state_snapshot.center_freq_hz.round()
                                    as i64
                                    + new_offset_hz)
                                    .max(0) as f32;
                                new_target_freq_hz = Some(new_target);
                            }

                            if let Some(new_center_hz) = new_center_freq_hz {
                                if let Ok(mut state) = self.state.lock() {
                                    state.center_freq_hz = new_center_hz;
                                }

                                let _ = self.ws_cmd_tx.send(
                                    ControlCommand::LegacyClientMessage(
                                        rigflow_protocol::ClientMessage::SetCenterFrequency {
                                            center_freq_hz: new_center_hz,
                                        },
                                    ),
                                );
                            }

                            if let Some(new_target_hz) = new_target_freq_hz {
                                if let Ok(mut state) = self.state.lock() {
                                    state.target_freq_hz = new_target_hz;
                                }

                                let _ = self.ws_cmd_tx.send(
                                    ControlCommand::LegacyClientMessage(
                                        rigflow_protocol::ClientMessage::SetFrequency {
                                            target_freq_hz: new_target_hz,
                                        },
                                    ),
                                );
                            }
                        },
                    );

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), spectrum_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            let spectrum_snapshot = {
                                let guard = self.spectrum_db.lock().unwrap();
                                guard.clone()
                            };

                            let state_snapshot = {
                                let state = self.state.lock().unwrap();
                                state.clone()
                            };

                            let interaction: SpectrumInteraction =
                                draw_spectrum_plot(
                                    ui,
                                    egui::vec2(ui.available_width(), spectrum_height),
                                    &spectrum_snapshot,
                                    -120.0,
                                    0.0,
                                    &state_snapshot,
                                );

                            if let Some(clicked_freq_hz) =
                                interaction.clicked_target_freq_hz
                            {
                                let _ = self.ws_cmd_tx.send(
                                    ControlCommand::LegacyClientMessage(
                                        rigflow_protocol::ClientMessage::SetFrequency {
                                            target_freq_hz: clicked_freq_hz,
                                        },
                                    ),
                                );
                            }
                        },
                    );

                    ui.add_space(gap);
                    ui.separator();
                    ui.add_space(gap);

                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), waterfall_height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            self.update_waterfall_texture(
                                ctx,
                                WATERFALL_IMAGE_WIDTH,
                                WATERFALL_IMAGE_HEIGHT,
                            );

                            if let Some(texture) = &self.waterfall_texture {
                                let image_width = (ui.available_width()
                                    - LEFT_GUTTER
                                    - RIGHT_GUTTER)
                                    .max(100.0);

                                let mut clicked_freq_hz = None;

                                ui.horizontal(|ui| {
                                    ui.add_space(LEFT_GUTTER);

                                    let image = egui::Image::new((
                                        texture.id(),
                                        egui::vec2(image_width, waterfall_height),
                                    ))
                                    .sense(egui::Sense::click());

                                    let response = ui.add(image);

                                    if response.clicked()
                                        && snapshot.input_sample_rate_hz > 0.0
                                    {
                                        if let Some(pointer_pos) =
                                            response.interact_pointer_pos()
                                        {
                                            let frac = ((pointer_pos.x
                                                - response.rect.left())
                                                / response.rect.width())
                                                .clamp(0.0, 1.0);

                                            let state_snapshot = {
                                                let state = self.state.lock().unwrap();
                                                state.clone()
                                            };

                                            let spectrum_len = {
                                                let spectrum =
                                                    self.spectrum_db.lock().unwrap();
                                                spectrum.len()
                                            };

                                            if let Some((left_hz, right_hz)) =
                                                zoomed_visible_freq_range_hz(
                                                    spectrum_len,
                                                    &state_snapshot,
                                                )
                                            {
                                                clicked_freq_hz =
                                                    Some(left_hz + frac * (right_hz - left_hz));
                                            } else {
                                                clicked_freq_hz = Some(
                                                    x_frac_to_frequency_hz(
                                                        frac,
                                                        &state_snapshot,
                                                    ),
                                                );
                                            }
                                        }
                                    }
                                });

                                if let Some(clicked_freq_hz) = clicked_freq_hz {
                                    if !snapshot.radio_acquired {
                                        if let Ok(mut state) = self.state.lock() {
                                            state.server_status =
                                                "cannot tune: no radio acquired"
                                                    .to_string();
                                        }
                                    } else {
                                        if let Ok(mut state) = self.state.lock() {
                                            state.target_freq_hz = clicked_freq_hz;
                                        }

                                        let _ = self.ws_cmd_tx.send(
                                            ControlCommand::LegacyClientMessage(
                                                rigflow_protocol::ClientMessage::SetFrequency {
                                                    target_freq_hz: clicked_freq_hz,
                                                },
                                            ),
                                        );
                                    }
                                }
                            }
                        },
                    );
                });
        });

	let show_add_operator_dialog = {
	    let state = self.state.lock().unwrap();
	    state.show_add_operator_dialog
	};

	if show_add_operator_dialog {
	    egui::Window::new("Add Operator")
		.collapsible(false)
		.resizable(false)
		.show(ctx, |ui| {
		    let mut save_requested = false;
		    let mut cancel_requested = false;

		    if let Ok(mut state) = self.state.lock() {
			ui.label("Operator ID / Call Sign:");
			ui.text_edit_singleline(&mut state.pending_operator_id);

			ui.add_space(8.0);
			ui.label("License:");

			use crate::ui::om_bands::LicenseClass;

			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::AmateurExtra),
			    "Amateur Extra",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::Advanced),
			    "Advanced",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::General),
			    "General",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::Technician),
			    "Technician",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    None,
			    "None",
			);

			if !state.persistence_status.is_empty() {
			    ui.add_space(8.0);
			    ui.colored_label(
				egui::Color32::YELLOW,
				&state.persistence_status,
			    );
			}

			ui.add_space(10.0);

			ui.horizontal(|ui| {
			    if ui.button("Cancel").clicked() {
				cancel_requested = true;
			    }

			    if ui.button("Save").clicked() {
				save_requested = true;
			    }
			});
		    }

		    if cancel_requested {
			if let Ok(mut state) = self.state.lock() {
			    state.show_add_operator_dialog = false;
			    state.pending_operator_id.clear();
			    state.pending_operator_license = None;
			    state.persistence_status.clear();
			}
		    }

		    if save_requested {
			self.save_pending_operator();
		    }
		});
	}

	let show_add_bookmark_dialog = {
	    let state = self.state.lock().unwrap();
	    state.show_add_bookmark_dialog
	};

	if show_add_bookmark_dialog {
	    egui::Window::new("Save Current as Bookmark")
		.collapsible(false)
		.resizable(false)
		.show(ctx, |ui| {
		    let mut save_requested = false;
		    let mut cancel_requested = false;

		    if let Ok(mut state) = self.state.lock() {
			ui.label("Bookmark name:");
			ui.text_edit_singleline(&mut state.pending_bookmark_name);

			if !state.bookmark_status.is_empty() {
			    ui.add_space(8.0);
			    ui.colored_label(
				egui::Color32::YELLOW,
				&state.bookmark_status,
			    );
			}

			ui.add_space(10.0);

			ui.horizontal(|ui| {
			    if ui.button("Cancel").clicked() {
				cancel_requested = true;
			    }

			    if ui.button("Save").clicked() {
				save_requested = true;
			    }
			});
		    }

		    if cancel_requested {
			if let Ok(mut state) = self.state.lock() {
			    state.show_add_bookmark_dialog = false;
			    state.pending_bookmark_name.clear();
			    state.bookmark_status.clear();
			}
		    }

		    if save_requested {
			self.save_current_as_bookmark();
		    }
		});
	}

	let default_bookmark_to_apply = {
	    let mut state = self.state.lock().unwrap();

	    if state.pending_apply_default_bookmark {
		state.pending_apply_default_bookmark = false;
		state.default_bookmark_id.clone()
	    } else {
		None
	    }
	};

	if let Some(bookmark_id) = default_bookmark_to_apply {
	    self.apply_bookmark(&bookmark_id);
	}

	let delete_target = {
	    let state = self.state.lock().unwrap();
	    if state.show_delete_operator_dialog {
		state.pending_delete_operator_id.clone()
	    } else {
		None
	    }
	};

	if let Some(operator_id) = delete_target {
	    egui::Window::new("Delete Operator")
		.collapsible(false)
		.resizable(false)
		.show(ctx, |ui| {
		    ui.label(format!("Delete operator \"{}\"?", operator_id));
		    ui.add_space(6.0);
		    ui.colored_label(
			egui::Color32::YELLOW,
			"All operator settings, including bookmarks, will be lost.",
		    );

		    ui.add_space(10.0);

		    let mut cancel_requested = false;
		    let mut delete_requested = false;

		    ui.horizontal(|ui| {
			if ui.button("Cancel").clicked() {
			    cancel_requested = true;
			}

			if ui.button("Delete").clicked() {
			    delete_requested = true;
			}
		    });

		    if cancel_requested {
			if let Ok(mut state) = self.state.lock() {
			    state.show_delete_operator_dialog = false;
			    state.pending_delete_operator_id = None;
			    state.persistence_status.clear();
			}
		    }

		    if delete_requested {
			self.delete_operator(&operator_id);
		    }
		});
	}

        ctx.request_repaint();
    }
}

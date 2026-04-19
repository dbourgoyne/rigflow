use std::time::{Duration, Instant};
use super::app::RigflowApp;
use eframe::egui;

use crate::UiState;
use crate::ui::om_bands::LicenseClass;
use crate::persistence::apply_operator_settings_to_ui_state;
use crate::ControlCommand;
use rigflow_core::dsp::modes::{DemodMode, Sideband, filter_bandwidth_limits, clamp_filter_bandwidth};
use crate::ui::utils::should_send_debounced;

impl RigflowApp {

    
    pub(crate) fn draw_left_panel(
	&mut self,
	ctx: &egui::Context,
	snapshot: &UiState,
	config_mode: bool,
    ) {

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("rigflow");
                ui.separator();

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
			self.draw_operator_panel(ui, snapshot, config_mode);
                        ui.separator();
			self.draw_server_panel(ui, snapshot, config_mode);
			ui.separator();
			self.draw_radios_panel(ui, snapshot);
			self.draw_radio_control_panel(ui, snapshot);
			ui.separator();
			self.draw_waterfall_control_panel(ui);
			ui.separator();
			self.draw_bookmarks_panel(ui);
			ui.separator();
		    });
            });
    }


    pub(crate) fn draw_waterfall_control_panel(
	&mut self,
	ui: &mut egui::Ui,
    ) {
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
    }

    pub(crate) fn draw_radio_control_panel(
	&mut self,
	ui: &mut egui::Ui,
	snapshot: &UiState,
    ) {    
        if snapshot.radio_acquired {
            egui::CollapsingHeader::new("Radio Control")
                .default_open(true)
                .show(ui, |ui| {

		    // ----------- Filter Bandwidth Slider ---------------
		    if let Ok(mut state) = self.state.lock() {
			let bw_limits = filter_bandwidth_limits(snapshot.demod_mode);

			// Apply default once on first mode entry.
			if state.last_demod_mode_for_bw != Some(snapshot.demod_mode) {
			    state.filter_bandwidth_hz = bw_limits.default_hz;
			    state.last_demod_mode_for_bw = Some(snapshot.demod_mode);
			    state.filter_bw_debounce.last_sent_value = bw_limits.default_hz;
			    state.filter_bw_debounce.last_send_time = std::time::Instant::now();

			    let _ = self.ws_cmd_tx.send(
				ControlCommand::LegacyClientMessage(
				    rigflow_protocol::ClientMessage::SetFilterBandwidth {
					bandwidth_hz: bw_limits.default_hz,
				    },
				),
			    );
			}

			// Keep local value in valid range.
			state.filter_bandwidth_hz = clamp_filter_bandwidth(
			    snapshot.demod_mode,
			    state.filter_bandwidth_hz
			);

			let response = ui.add(
			    egui::Slider::new(
				&mut state.filter_bandwidth_hz,
				bw_limits.min_hz..=bw_limits.max_hz,
			    )
				.text("Filter Bandwidth (Hz)")
			);

			let now = Instant::now();

			// Throttled live updates while dragging.
			if response.changed() {

			    if let Some(send_hz) = should_send_debounced(
				now,
				state.filter_bandwidth_hz,
				&mut state.filter_bw_debounce,
				10.0,
				Duration::from_millis(75),
			    ) {

				let _ = self.ws_cmd_tx.send(
				    ControlCommand::LegacyClientMessage(
					rigflow_protocol::ClientMessage::SetFilterBandwidth {
					    bandwidth_hz: send_hz,
					},
				    ),
				);
			    }
			}
			// Always send the final exact value when drag ends.
			if response.drag_stopped() {

			    let final_hz = state
				.filter_bandwidth_hz
				.round()
				.clamp(bw_limits.min_hz, bw_limits.max_hz);

			    if (final_hz - state.last_filter_bw_sent_hz).abs() >= 1.0 {
				state.last_filter_bw_sent_hz = final_hz;
				state.last_filter_bw_send_time = now;

				let _ = self.ws_cmd_tx.send(
				    ControlCommand::LegacyClientMessage(
					rigflow_protocol::ClientMessage::SetFilterBandwidth {
					    bandwidth_hz: final_hz,
					},
				    ),
				);
			    }
			}
		    }



		    //----------- SSB and CW Pitch Hz Slider -----------------
		    match snapshot.demod_mode {
			DemodMode::Usb | DemodMode::Lsb => {
			    if let Ok(mut state) = self.state.lock() {
				let response = ui.add(
				    egui::Slider::new(&mut state.ssb_pitch_hz, -1500.0..=1500.0)
					.text("SSB Pitch (Hz)")
				);

				if response.changed() {
				    self.ws_cmd_tx.send(
				    	ControlCommand::LegacyClientMessage(
					    rigflow_protocol::ClientMessage::SetPitch {
						pitch_hz: state.ssb_pitch_hz,
					    }
					)
				    ).ok();
				}
			    }
			}

			DemodMode::Cw => {
			    if let Ok(mut state) = self.state.lock() {
				let response = ui.add(
				    egui::Slider::new(&mut state.cw_pitch_hz, 300.0..=1200.0)
					.text("CW Pitch (Hz)")
				);

				if response.changed() {
				    self.ws_cmd_tx.send(
					ControlCommand::LegacyClientMessage(
					    rigflow_protocol::ClientMessage::SetPitch {
						pitch_hz: state.cw_pitch_hz,
					    }
					)
				    ).ok();
				}
			    }
			}

			_ => {}
		    }
		    
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
                            DemodMode::Am,
                            "am",
                        );
			ui.radio_value(
                            &mut selected_demod,
                            DemodMode::Cw,
                            "cw",
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
    }

    pub(crate) fn draw_radios_panel(
	&mut self,
	ui: &mut egui::Ui,
	snapshot: &UiState,
    ) {
	
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
    }

    pub(crate) fn draw_server_panel(
	&mut self,
	ui: &mut egui::Ui,
	snapshot: &UiState,
	config_mode:bool,
    ) {
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
    }

    pub(crate) fn draw_operator_panel(
	&mut self,
	ui: &mut egui::Ui,
	snapshot: &UiState,
	config_mode: bool,
    ) {
	
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
    }

    pub(crate) fn draw_bookmarks_panel(
	&mut self,
	ui: &mut egui::Ui,
    ) {
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

		ui.label("Notes:");

		let mut edited_notes = snapshot
		    .selected_bookmark_id
		    .as_ref()
		    .and_then(|selected_id| {
			snapshot
			    .bookmarks
			    .iter()
			    .find(|b| &b.id == selected_id)
			    .and_then(|b| b.notes.clone())
		    })
		    .unwrap_or_default();

		let notes_changed = ui
		    .add_enabled(
			snapshot.selected_bookmark_id.is_some(),
			egui::TextEdit::multiline(&mut edited_notes)
			    .desired_rows(4)
			    .desired_width(f32::INFINITY),
		    )
		    .changed();

		if notes_changed {
		    self.update_selected_bookmark_notes(edited_notes);
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
		    state.pending_bookmark_notes.clear();
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
    }
}

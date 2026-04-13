use std::sync::{Arc, Mutex};

use eframe::egui;
use tokio::sync::mpsc;

use crate::app::layout::{
    LEFT_GUTTER, RIGHT_GUTTER, WATERFALL_IMAGE_HEIGHT, WATERFALL_IMAGE_WIDTH,
};
use crate::app::om_bands::LicenseClass;
use crate::app::state::UiState;
use crate::net::control::ControlCommand;
use crate::ui::spectrum_view::{
    draw_spectrum_plot, x_frac_to_frequency_hz, SpectrumInteraction,
};

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    pub waterfall_buffer: Arc<Mutex<Vec<u32>>>,
    pub spectrum_db: Arc<Mutex<Vec<f32>>>,
    pub waterfall_texture: Option<egui::TextureHandle>,
}
use rigflow_core::dsp::modes::{DemodMode, Sideband};

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

    /// Upload the current waterfall pixel buffer into an egui texture.
    ///
    /// The media thread maintains the CPU-side pixel buffer as packed RGB
    /// values. Each frame, the UI converts that into an egui `ColorImage`
    /// and either updates the existing texture or creates it on first use.
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
            println!(
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
        // Snapshot shared UI state once at the start of the frame so most UI
        // rendering can read without repeatedly holding the mutex.
        let snapshot = {
            let state = self.state.lock().unwrap();
            state.clone()
        };

        // --- Keyboard center-frequency tuning --------------------------------
        let mut center_delta_hz: f32 = 0.0;

        ctx.input(|input| {
            let step = if input.modifiers.shift {
                1_000_000.0 // large step (1 MHz)
            } else {
                25_000.0 // small step (25 kHz)
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

        // --- Keyboard target-frequency tuning --------------------------------
        let mut target_delta_hz: f32 = 0.0;

        ctx.input(|input| {
            let step = if input.modifiers.shift {
                1_000.0 // large step (1 kHz)
            } else {
                10.0 // small step (10 Hz)
            };

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
                        // --- Radio Operator menu ------------------------------
                        ui.collapsing("Radio Operator", |ui| {
                            let mut selected = snapshot.selected_license;

                            ui.radio_value(
                                &mut selected,
                                Some(LicenseClass::AmateurExtra),
                                "Amateur Extra",
                            );
                            ui.radio_value(
                                &mut selected,
                                Some(LicenseClass::Advanced),
                                "Advanced",
                            );
                            ui.radio_value(
                                &mut selected,
                                Some(LicenseClass::General),
                                "General",
                            );
                            ui.radio_value(
                                &mut selected,
                                Some(LicenseClass::Technician),
                                "Technician",
                            );
                            ui.radio_value(&mut selected, None, "None");

                            if selected != snapshot.selected_license {
                                if let Ok(mut state) = self.state.lock() {
                                    state.selected_license = selected;
                                }
                            }
                        });

                        // --- Rigflow Server menu ------------------------------
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

                        // --- Radios menu --------------------------------------
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

                        // --- Radio Control menu -------------------------------
                        //
                        // This section only appears after a radio has been
                        // successfully acquired. The existing structure here
                        // is intentionally preserved because it is behavior-
                        // sensitive and directly tied to working UI flow.
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
							    _ => unreachable!("sideband only sent for USB/LSB"),
							},
                                                    },
                                                ),
                                            );
                                        }
                                    }
                                });
                        }
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

                    // --- LO frequency widget strip --------------------------
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

                    // --- Spectrum region ------------------------------------
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

                    // --- Waterfall region -----------------------------------
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

                                            clicked_freq_hz = Some(
                                                x_frac_to_frequency_hz(
                                                    frac,
                                                    &state_snapshot,
                                                ),
                                            );
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

        // Real-time UI: continuously repaint so spectrum/waterfall remain live.
        ctx.request_repaint();
    }
}

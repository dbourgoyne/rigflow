use log::warn;
use super::app::RigflowApp;
use eframe::egui;
use crate::UiState;
use crate::ControlCommand;
use crate::ui::spectrum_view::{
    SpectrumInteraction,
    draw_spectrum_plot,
    zoomed_visible_freq_range_hz,
    x_frac_to_frequency_hz,
};
use crate::ui::layout::{
    WATERFALL_IMAGE_WIDTH,
    WATERFALL_IMAGE_HEIGHT,
    LEFT_GUTTER,
    RIGHT_GUTTER,
};

impl RigflowApp {

    pub(crate) fn draw_center_panel(
        &mut self,
        ctx: &egui::Context,
        snapshot: &UiState,
    ) {
	
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
                                    ControlCommand::RadioMessage(
                                        rigflow_protocol::ClientRadioMessage::SetCenterFrequency {
                                            center_freq_hz: new_center_hz as u64,
                                        },
                                    ),
                                );
                            }

                            if let Some(new_target_hz) = new_target_freq_hz {
                                if let Ok(mut state) = self.state.lock() {
                                    state.target_freq_hz = new_target_hz;
                                }

                                let _ = self.ws_cmd_tx.send(
                                    ControlCommand::RadioMessage(
                                        rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                                            target_freq_hz: new_target_hz as u64,
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

			    if let Some(bookmark_id) = interaction.clicked_bookmark_id {
				self.apply_bookmark(&bookmark_id);
			    } else if let Some(clicked_freq_hz) = interaction.clicked_target_freq_hz {
				let _ = self.ws_cmd_tx.send(
				    ControlCommand::RadioMessage(
					rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
					    target_freq_hz: clicked_freq_hz as u64,
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
                                            ControlCommand::RadioMessage(
                                                rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                                                    target_freq_hz: clicked_freq_hz as u64,
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


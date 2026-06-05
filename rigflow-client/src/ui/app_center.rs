use super::app::RigflowApp;
use crate::ui::layout::{LEFT_GUTTER, RIGHT_GUTTER, WATERFALL_IMAGE_HEIGHT, WATERFALL_IMAGE_WIDTH};
use crate::ui::spectrum_view::{
    draw_spectrum_plot, x_frac_to_frequency_hz, zoomed_visible_freq_range_hz, SpectrumInteraction,
};
use crate::ControlCommand;
use crate::UiState;
use eframe::egui;
use log::warn;

impl RigflowApp {
    pub(crate) fn draw_center_panel(&mut self, ctx: &egui::Context, snapshot: &UiState) {
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
                    // Top status bar: live operating telemetry (frequency, mode,
                    // S-meter, dBm, TX/RX, SWR, REC).  Allocated first so it
                    // consumes height before the spectrum/waterfall are sized.
                    let status_bar_height = 30.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), status_bar_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            self.draw_status_bar(ui, snapshot);
                        },
                    );

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
                                let limits =
                                    crate::ui::freq_limits::active_freq_limits(&state_snapshot);
                                new_center_freq_hz = Some(crate::ui::freq_limits::clamp_center(
                                    new_center_hz as f32,
                                    &limits,
                                ));
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
                                let raw_target = (state_snapshot.center_freq_hz.round() as i64
                                    + new_offset_hz)
                                    .max(0) as f32;
                                let limits =
                                    crate::ui::freq_limits::active_freq_limits(&state_snapshot);
                                new_target_freq_hz = Some(crate::ui::freq_limits::clamp_target(
                                    raw_target,
                                    state_snapshot.center_freq_hz,
                                    state_snapshot.input_sample_rate_hz,
                                    &limits,
                                ));
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

                            let (spectrum_db_min, spectrum_db_max) =
                                if state_snapshot.adaptive_waterfall_normalization {
                                    let top = state_snapshot.adaptive_top_db_estimate + 3.0;
                                    (top - state_snapshot.adaptive_range_db_estimate, top)
                                } else {
                                    let top = state_snapshot.manual_waterfall_top_db;
                                    (top - state_snapshot.manual_waterfall_range_db, top)
                                };

                            let interaction: SpectrumInteraction =
                                draw_spectrum_plot(
                                    ui,
                                    egui::vec2(ui.available_width(), spectrum_height),
                                    &spectrum_snapshot,
                                    spectrum_db_min,
                                    spectrum_db_max,
                                    &state_snapshot,
                                );

			    if let Some(bookmark_id) = interaction.clicked_bookmark_id {
				self.apply_bookmark(&bookmark_id);
			    } else if let Some(clicked_freq_hz) = interaction.clicked_target_freq_hz {
				let limits =
				    crate::ui::freq_limits::active_freq_limits(snapshot);
				let clamped = crate::ui::freq_limits::clamp_target(
				    clicked_freq_hz,
				    snapshot.center_freq_hz,
				    snapshot.input_sample_rate_hz,
				    &limits,
				);
				let _ = self.ws_cmd_tx.send(
				    ControlCommand::RadioMessage(
					rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
					    target_freq_hz: clamped as u64,
					},
				    ),
				);
			    }

				    // Mouse-wheel fine tuning over the spectrum: +/-50 Hz per
				    // notch, through the same clamp/tune path as keys and click-to-
				    // tune (server-side validation preserved).  Local target is
				    // updated so rapid scrolls accumulate before the server echo.
				    if interaction.scroll_target_delta_hz != 0.0
					&& snapshot.radio_acquired
				    {
					let limits =
					    crate::ui::freq_limits::active_freq_limits(snapshot);
					let new_target = crate::ui::freq_limits::clamp_target(
					    snapshot.target_freq_hz + interaction.scroll_target_delta_hz,
					    snapshot.center_freq_hz,
					    snapshot.input_sample_rate_hz,
					    &limits,
					);
					if let Ok(mut state) = self.state.lock() {
					    state.target_freq_hz = new_target;
					}
					let _ = self.ws_cmd_tx.send(
					    ControlCommand::RadioMessage(
						rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
						    target_freq_hz: new_target as u64,
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
                                let mut wheel_delta_hz = 0.0_f32;

                                ui.horizontal(|ui| {
                                    ui.add_space(LEFT_GUTTER);

                                    let image = egui::Image::new((
                                        texture.id(),
                                        egui::vec2(image_width, waterfall_height),
                                    ))
					.sense(egui::Sense::click());

                                    let response = ui.add(image);

                                    // Mouse-wheel fine tuning over the waterfall:
                                    // ±50 Hz per notch (same step/path as the
                                    // spectrum), only while hovering the image.
                                    if response.hovered() {
                                        let scroll_y =
                                            ui.ctx().input(|i| i.raw_scroll_delta.y);
                                        if scroll_y > 0.0 {
                                            wheel_delta_hz =
                                                crate::ui::spectrum_view::WHEEL_TUNE_STEP_HZ;
                                        } else if scroll_y < 0.0 {
                                            wheel_delta_hz =
                                                -crate::ui::spectrum_view::WHEEL_TUNE_STEP_HZ;
                                        }
                                    }

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
                                        let limits =
                                            crate::ui::freq_limits::active_freq_limits(snapshot);
                                        let clamped = crate::ui::freq_limits::clamp_target(
                                            clicked_freq_hz,
                                            snapshot.center_freq_hz,
                                            snapshot.input_sample_rate_hz,
                                            &limits,
                                        );
                                        if let Ok(mut state) = self.state.lock() {
                                            state.target_freq_hz = clamped;
                                        }
                                        let _ = self.ws_cmd_tx.send(
                                            ControlCommand::RadioMessage(
                                                rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                                                    target_freq_hz: clamped as u64,
                                                },
                                            ),
                                        );
                                    }
                                }

                                // Apply waterfall mouse-wheel fine tuning through
                                // the same clamp/tune path (server-side validation
                                // preserved); local target updated so rapid
                                // scrolls accumulate.
                                if wheel_delta_hz != 0.0 && snapshot.radio_acquired {
                                    let limits =
                                        crate::ui::freq_limits::active_freq_limits(snapshot);
                                    let new_target = crate::ui::freq_limits::clamp_target(
                                        snapshot.target_freq_hz + wheel_delta_hz,
                                        snapshot.center_freq_hz,
                                        snapshot.input_sample_rate_hz,
                                        &limits,
                                    );
                                    if let Ok(mut state) = self.state.lock() {
                                        state.target_freq_hz = new_target;
                                    }
                                    let _ = self.ws_cmd_tx.send(
                                        ControlCommand::RadioMessage(
                                            rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                                                target_freq_hz: new_target as u64,
                                            },
                                        ),
                                    );
                                }
                            }
                        },
                    );
                });
        });
    }

    /// Top status bar: compact, single-row live operating telemetry.  Reads
    /// only from the UI snapshot (no protocol changes); optional fields (SWR,
    /// REC) are omitted when unavailable.  Structured as a left-to-right row so
    /// future items (TX power, ALC, network status, …) just append more cells.
    fn draw_status_bar(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        use crate::ui::panels::s_meter_label;

        if !snapshot.radio_acquired {
            ui.label(egui::RichText::new("No radio acquired").weak());
            return;
        }

        // Frequency (operating / target) — prominent.
        ui.label(
            egui::RichText::new(format_freq_dotted(snapshot.target_freq_hz.max(0.0) as u64))
                .size(18.0)
                .strong(),
        );
        // Mode.
        ui.label(egui::RichText::new(mode_label(snapshot.demod_mode)).strong());

        ui.separator();

        // S-meter — the most prominent item (largest text, coloured).
        ui.label(
            egui::RichText::new(s_meter_label(snapshot.signal_dbm))
                .size(20.0)
                .strong()
                .color(egui::Color32::from_rgb(120, 230, 120)),
        );
        ui.label(egui::RichText::new(format!("{:.0} dBm", snapshot.signal_dbm)).size(15.0));

        ui.separator();

        // TX / RX state.
        let transmitting = snapshot.ssb_ptt_down
            || snapshot.cw_key_down
            || snapshot.tx_tone_running
            || snapshot.tx_tune_running;
        if transmitting {
            ui.label(
                egui::RichText::new("TX")
                    .strong()
                    .color(egui::Color32::from_rgb(235, 80, 80)),
            );
        } else {
            ui.label(egui::RichText::new("RX").weak());
        }

        // SWR — shown only when the source reports it.
        if let Some(swr) = snapshot.source_status.swr {
            ui.separator();
            ui.label(format!("SWR {swr:.1}"));
        }

        // Recording — shown only while a recording is active.
        if snapshot.iq_recording_status.recording {
            ui.separator();
            ui.label(
                egui::RichText::new("REC")
                    .strong()
                    .color(egui::Color32::from_rgb(235, 90, 90)),
            );
        }
    }

    fn update_waterfall_texture(&mut self, ctx: &egui::Context, wf_width: usize, wf_height: usize) {
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

        let mut image = egui::ColorImage::new([wf_width, wf_height], egui::Color32::BLACK);

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
                let texture =
                    ctx.load_texture("waterfall_texture", image, egui::TextureOptions::NEAREST);
                self.waterfall_texture = Some(texture);
            }
        }
    }
}

/// Format a frequency in Hz with `.` thousands separators, e.g.
/// `14074000 → "14.074.000"`.
fn format_freq_dotted(hz: u64) -> String {
    let digits = hz.to_string();
    let n = digits.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, c) in digits.chars().enumerate() {
        if i != 0 && (n - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// Uppercase mode label for the status bar.
fn mode_label(mode: rigflow_core::dsp::modes::DemodMode) -> &'static str {
    use rigflow_core::dsp::modes::DemodMode;
    match mode {
        DemodMode::Wfm => "WFM",
        DemodMode::Nfm => "NFM",
        DemodMode::Usb => "USB",
        DemodMode::Lsb => "LSB",
        DemodMode::Am => "AM",
        DemodMode::Cwu => "CWU",
        DemodMode::Cwl => "CWL",
    }
}

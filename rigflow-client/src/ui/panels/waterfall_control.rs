use std::time::{Duration, Instant};

use crate::ui::app::RigflowApp;
use crate::ui::utils::should_send_debounced;
use eframe::egui;
use egui::RichText;
use rigflow_core::radio::vfo::VfoSelect;
use rigflow_protocol::radio_control::ClientRadioMessage;

/// Default waterfall frame rate (Hz). Must match `UiState::default` and the
/// persistence serde default (`WaterfallDisplayPreferencesFile`).
const DEF_WATERFALL_RATE_HZ: f32 = 20.0;

impl RigflowApp {
    pub(crate) fn draw_waterfall_control_panel(&mut self, ui: &mut egui::Ui) {
        let mut save_waterfall_prefs = false;
        // Waterfall rate to push to the server this frame, if any (set inside the
        // panel closure; sent after it so we don't hold the state lock across the send).
        let mut send_waterfall_rate: Option<f32> = None;
        // Which VFO the controls edit this frame (drives the rate message + persist).
        let mut active_b = false;
        let now = Instant::now();

        ui.collapsing(super::panel_header("Waterfall Control"), |ui| {
            if let Ok(mut state) = self.state.lock() {
                // Under dual-watch the controls follow the "Active VFO" selector;
                // each VFO keeps its own zoom / normalization / rate.  VFO B is
                // session-only, so it never sets `save_waterfall_prefs`.
                active_b =
                    state.dual_watch_enabled && matches!(state.active_control_vfo, VfoSelect::B);
                let persist = !active_b;
                if active_b {
                    ui.label(RichText::new("Editing VFO B").size(11.0));
                }

                // ── Zoom ──
                let mut zoom = if active_b {
                    state.vfo_b_display_zoom
                } else {
                    state.display_zoom
                };
                let mut zoom_response =
                    ui.add(egui::Slider::new(&mut zoom, 1.0..=4.0).text("Zoom"));
                let zoom_scrolled =
                    super::slider_scroll(ui, &mut zoom_response, &mut zoom, 1.0, 4.0, 0.1);
                if active_b {
                    state.vfo_b_display_zoom = zoom;
                } else {
                    state.display_zoom = zoom;
                }
                if (zoom_response.drag_stopped() || zoom_scrolled) && persist {
                    save_waterfall_prefs = true;
                }

                // ── Adaptive normalization ──
                let mut adaptive = if active_b {
                    state.vfo_b_adaptive_waterfall_normalization
                } else {
                    state.adaptive_waterfall_normalization
                };
                let adaptive_changed = ui
                    .checkbox(&mut adaptive, "Adaptive normalization")
                    .changed();
                if active_b {
                    state.vfo_b_adaptive_waterfall_normalization = adaptive;
                } else {
                    state.adaptive_waterfall_normalization = adaptive;
                }
                if adaptive_changed && persist {
                    save_waterfall_prefs = true;
                }

                // ── Manual Top / Range dB ──
                let manual_enabled = !adaptive;
                ui.add_enabled_ui(manual_enabled, |ui| {
                    let mut top = if active_b {
                        state.vfo_b_manual_waterfall_top_db
                    } else {
                        state.manual_waterfall_top_db
                    };
                    let mut top_response =
                        ui.add(egui::Slider::new(&mut top, -120.0..=20.0).text("Top dB"));
                    let top_scrolled =
                        super::slider_scroll(ui, &mut top_response, &mut top, -120.0, 20.0, 1.0);
                    if active_b {
                        state.vfo_b_manual_waterfall_top_db = top;
                    } else {
                        state.manual_waterfall_top_db = top;
                    }

                    let mut range = if active_b {
                        state.vfo_b_manual_waterfall_range_db
                    } else {
                        state.manual_waterfall_range_db
                    };
                    let mut range_response =
                        ui.add(egui::Slider::new(&mut range, 10.0..=120.0).text("Range dB"));
                    let range_scrolled =
                        super::slider_scroll(ui, &mut range_response, &mut range, 10.0, 120.0, 1.0);
                    if active_b {
                        state.vfo_b_manual_waterfall_range_db = range;
                    } else {
                        state.manual_waterfall_range_db = range;
                    }

                    if manual_enabled
                        && persist
                        && (top_response.drag_stopped()
                            || range_response.drag_stopped()
                            || top_scrolled
                            || range_scrolled)
                    {
                        save_waterfall_prefs = true;
                    }
                });

                // ── Smoothing (temporal EMA on the FFT rows) ──
                // Reduces noise-floor scintillation.  0 = raw / off.
                let mut smoothing = if active_b {
                    state.vfo_b_waterfall_smoothing
                } else {
                    state.waterfall_smoothing
                };
                let mut smooth_response = ui.add(
                    egui::Slider::new(&mut smoothing, 0.0..=1.0)
                        .fixed_decimals(2)
                        .text("Smoothing"),
                );
                let smooth_scrolled =
                    super::slider_scroll(ui, &mut smooth_response, &mut smoothing, 0.0, 1.0, 0.05);
                if active_b {
                    state.vfo_b_waterfall_smoothing = smoothing;
                } else {
                    state.waterfall_smoothing = smoothing;
                }
                if (smooth_response.drag_stopped() || smooth_scrolled) && persist {
                    save_waterfall_prefs = true;
                }

                // ── Waterfall frame rate (sent to the server; 0 = off) ──
                let mut rate = if active_b {
                    state.vfo_b_waterfall_frame_rate_hz
                } else {
                    state.waterfall_frame_rate_hz
                };
                let mut rate_response = ui.add(
                    egui::Slider::new(&mut rate, 0.0..=30.0).text("Waterfall rate (Hz, 0 = off)"),
                );
                let rate_scrolled =
                    super::slider_scroll(ui, &mut rate_response, &mut rate, 0.0, 30.0, 1.0);
                if active_b {
                    state.vfo_b_waterfall_frame_rate_hz = rate;
                } else {
                    state.waterfall_frame_rate_hz = rate;
                }

                if rate_response.changed() {
                    if let Some(hz) = should_send_debounced(
                        now,
                        rate,
                        &mut state.waterfall_rate_debounce,
                        1.0,
                        Duration::from_millis(75),
                    ) {
                        send_waterfall_rate = Some(hz);
                    }
                }

                if rate_response.drag_stopped() || rate_scrolled {
                    let final_hz = rate.round().clamp(0.0, 30.0);
                    if active_b {
                        state.vfo_b_waterfall_frame_rate_hz = final_hz;
                    } else {
                        state.waterfall_frame_rate_hz = final_hz;
                    }
                    state.waterfall_rate_debounce.last_sent_value = final_hz;
                    state.waterfall_rate_debounce.last_send_time = now;
                    send_waterfall_rate = Some(final_hz);
                    if persist {
                        save_waterfall_prefs = true;
                    }
                }

                // Restore Default — greys out when already at the default.
                if ui
                    .add_enabled(
                        rate != DEF_WATERFALL_RATE_HZ,
                        egui::Button::new(RichText::new("Restore Default").size(8.0)),
                    )
                    .clicked()
                {
                    if active_b {
                        state.vfo_b_waterfall_frame_rate_hz = DEF_WATERFALL_RATE_HZ;
                    } else {
                        state.waterfall_frame_rate_hz = DEF_WATERFALL_RATE_HZ;
                    }
                    state.waterfall_rate_debounce.last_sent_value = DEF_WATERFALL_RATE_HZ;
                    state.waterfall_rate_debounce.last_send_time = now;
                    send_waterfall_rate = Some(DEF_WATERFALL_RATE_HZ);
                    if persist {
                        save_waterfall_prefs = true;
                    }
                }
            } else {
                ui.label("Waterfall controls unavailable");
            }
        });

        // Push the restored rate to the server on acquire (VFO A only). Done outside
        // the collapsing closure so it runs even when the panel is collapsed; the
        // server starts at its own default and won't otherwise hear our preference.
        let mut send_a_pending: Option<f32> = None;
        if let Ok(mut state) = self.state.lock() {
            if state.pending_apply_waterfall_rate {
                state.pending_apply_waterfall_rate = false;
                send_a_pending = Some(state.waterfall_frame_rate_hz);
            }
        }
        if let Some(rate_hz) = send_a_pending {
            self.send_radio_msg(ClientRadioMessage::SetWaterfallFrameRate { rate_hz });
        }

        // The slider's rate goes to whichever VFO is being edited.
        if let Some(rate_hz) = send_waterfall_rate {
            if active_b {
                self.send_radio_msg(ClientRadioMessage::SetVfoBWaterfallFrameRate { rate_hz });
            } else {
                self.send_radio_msg(ClientRadioMessage::SetWaterfallFrameRate { rate_hz });
            }
        }

        // Only VFO A's waterfall display persists (per-radio + operator defaults);
        // VFO B's is session-only.
        if save_waterfall_prefs {
            self.save_waterfall_display_preferences_to_current_operator();
        }
    }
}

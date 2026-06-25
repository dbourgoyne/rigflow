use std::time::{Duration, Instant};

use crate::ui::app::RigflowApp;
use crate::ui::utils::should_send_debounced;
use eframe::egui;
use egui::RichText;
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
        let now = Instant::now();

        ui.collapsing(super::panel_header("Waterfall Control"), |ui| {
            if let Ok(mut state) = self.state.lock() {
                let mut zoom_response =
                    ui.add(egui::Slider::new(&mut state.display_zoom, 1.0..=4.0).text("Zoom"));
                let zoom_scrolled = super::slider_scroll(
                    ui,
                    &mut zoom_response,
                    &mut state.display_zoom,
                    1.0,
                    4.0,
                    0.1,
                );

                if zoom_response.drag_stopped() || zoom_scrolled {
                    save_waterfall_prefs = true;
                }

                let adaptive_changed = ui
                    .checkbox(
                        &mut state.adaptive_waterfall_normalization,
                        "Adaptive normalization",
                    )
                    .changed();

                if adaptive_changed {
                    save_waterfall_prefs = true;
                }

                let manual_enabled = !state.adaptive_waterfall_normalization;

                ui.add_enabled_ui(manual_enabled, |ui| {
                    let mut top_response = ui.add(
                        egui::Slider::new(&mut state.manual_waterfall_top_db, -120.0..=20.0)
                            .text("Top dB"),
                    );
                    let top_scrolled = super::slider_scroll(
                        ui,
                        &mut top_response,
                        &mut state.manual_waterfall_top_db,
                        -120.0,
                        20.0,
                        1.0,
                    );

                    let mut range_response = ui.add(
                        egui::Slider::new(&mut state.manual_waterfall_range_db, 10.0..=120.0)
                            .text("Range dB"),
                    );
                    let range_scrolled = super::slider_scroll(
                        ui,
                        &mut range_response,
                        &mut state.manual_waterfall_range_db,
                        10.0,
                        120.0,
                        1.0,
                    );

                    if manual_enabled
                        && (top_response.drag_stopped()
                            || range_response.drag_stopped()
                            || top_scrolled
                            || range_scrolled)
                    {
                        save_waterfall_prefs = true;
                    }
                });

                // Waterfall frame rate (sent to the server; 0 = off). Range matches the
                // server clamp (0–30 Hz). Debounced during drag, final value on release.
                let mut rate_response = ui.add(
                    egui::Slider::new(&mut state.waterfall_frame_rate_hz, 0.0..=30.0)
                        .text("Waterfall rate (Hz, 0 = off)"),
                );
                let rate_scrolled = super::slider_scroll(
                    ui,
                    &mut rate_response,
                    &mut state.waterfall_frame_rate_hz,
                    0.0,
                    30.0,
                    1.0,
                );

                if rate_response.changed() {
                    if let Some(hz) = should_send_debounced(
                        now,
                        state.waterfall_frame_rate_hz,
                        &mut state.waterfall_rate_debounce,
                        1.0,
                        Duration::from_millis(75),
                    ) {
                        send_waterfall_rate = Some(hz);
                    }
                }

                if rate_response.drag_stopped() || rate_scrolled {
                    let final_hz = state.waterfall_frame_rate_hz.round().clamp(0.0, 30.0);
                    state.waterfall_frame_rate_hz = final_hz;
                    state.waterfall_rate_debounce.last_sent_value = final_hz;
                    state.waterfall_rate_debounce.last_send_time = now;
                    send_waterfall_rate = Some(final_hz);
                    save_waterfall_prefs = true;
                }

                // Restore Default — greys out when already at the default.
                if ui
                    .add_enabled(
                        state.waterfall_frame_rate_hz != DEF_WATERFALL_RATE_HZ,
                        egui::Button::new(RichText::new("Restore Default").size(8.0)),
                    )
                    .clicked()
                {
                    state.waterfall_frame_rate_hz = DEF_WATERFALL_RATE_HZ;
                    state.waterfall_rate_debounce.last_sent_value = DEF_WATERFALL_RATE_HZ;
                    state.waterfall_rate_debounce.last_send_time = now;
                    send_waterfall_rate = Some(DEF_WATERFALL_RATE_HZ);
                    save_waterfall_prefs = true;
                }
            } else {
                ui.label("Waterfall controls unavailable");
            }
        });

        // Push the restored rate to the server on acquire. Done outside the collapsing
        // closure so it runs even when the panel is collapsed; the server starts at its
        // own default and won't otherwise hear our per-operator/per-radio preference.
        if let Ok(mut state) = self.state.lock() {
            if state.pending_apply_waterfall_rate {
                state.pending_apply_waterfall_rate = false;
                send_waterfall_rate = Some(state.waterfall_frame_rate_hz);
            }
        }

        if let Some(rate_hz) = send_waterfall_rate {
            self.send_radio_msg(ClientRadioMessage::SetWaterfallFrameRate { rate_hz });
        }

        // Waterfall display persists per-radio via the debounced autosave (and to
        // operator-level defaults here, which seed a radio's first acquire).
        if save_waterfall_prefs {
            self.save_waterfall_display_preferences_to_current_operator();
        }
    }
}

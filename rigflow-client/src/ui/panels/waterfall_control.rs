use crate::ui::app::RigflowApp;
use eframe::egui;

impl RigflowApp {
    pub(crate) fn draw_waterfall_control_panel(&mut self, ui: &mut egui::Ui) {
        let mut save_waterfall_prefs = false;

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
            } else {
                ui.label("Waterfall controls unavailable");
            }
        });

        // Waterfall display persists per-radio via the debounced autosave (and
        // to operator-level defaults here, which seed a radio's first acquire).
        if save_waterfall_prefs {
            self.save_waterfall_display_preferences_to_current_operator();
        }
    }
}

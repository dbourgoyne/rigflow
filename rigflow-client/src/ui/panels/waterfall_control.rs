use crate::ui::app::RigflowApp;
use eframe::egui;

impl RigflowApp {
    pub(crate) fn draw_waterfall_control_panel(&mut self, ui: &mut egui::Ui) {
        let mut save_waterfall_prefs = false;

        ui.collapsing("Waterfall Control", |ui| {
            if let Ok(mut state) = self.state.lock() {
                let zoom_response =
                    ui.add(egui::Slider::new(&mut state.display_zoom, 1.0..=4.0).text("Zoom"));

                if zoom_response.drag_stopped() {
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
                    let top_response = ui.add(
                        egui::Slider::new(&mut state.manual_waterfall_top_db, -120.0..=20.0)
                            .text("Top dB"),
                    );

                    let range_response = ui.add(
                        egui::Slider::new(&mut state.manual_waterfall_range_db, 10.0..=120.0)
                            .text("Range dB"),
                    );

                    if manual_enabled
                        && (top_response.drag_stopped() || range_response.drag_stopped())
                    {
                        save_waterfall_prefs = true;
                    }
                });
            } else {
                ui.label("Waterfall controls unavailable");
            }
        });

        if save_waterfall_prefs {
            self.save_waterfall_display_preferences_to_current_operator();
        }
    }
}

use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;

impl RigflowApp {
    /// Draw the "TX Tune Test" panel.
    ///
    /// The panel is only shown when the active source advertises
    /// `supports_tx_tune_test = true` in its `SourceCapabilities`.
    /// All sources currently return `false`, so this panel is hidden
    /// until a TX-capable source enables it.
    ///
    /// # Safety invariants
    ///
    /// - The "Measure SWR" button is always disabled (`add_enabled(false, …)`).
    /// - The arm checkbox changes only `UiState::tx_tune_armed` — no
    ///   `ClientRadioMessage` is sent and no RF is produced.
    /// - `tx_tune_armed` always starts `false` (see `UiState::default`).
    pub(crate) fn draw_tx_tune_test_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if !snapshot.radio_acquired {
            return;
        }

        if !snapshot.source_capabilities.supports_tx_tune_test {
            return;
        }

        egui::CollapsingHeader::new("TX Tune Test")
            .default_open(true)
            .show(ui, |ui| {
                // ── Safety warning ───────────────────────────────────────────
                ui.label(
                    egui::RichText::new(
                        "⚠ Short low-power carrier pulse.\n\
                         Verify antenna/load and band before use.",
                    )
                    .color(egui::Color32::from_rgb(255, 160, 40))
                    .small(),
                );
                ui.add_space(4.0);

                // ── Arm checkbox ─────────────────────────────────────────────
                // Changing the checkbox updates only local UiState.
                // No server message is sent; no RF is produced.
                let mut armed = snapshot.tx_tune_armed;
                if ui.checkbox(&mut armed, "Arm TX tune test").changed() {
                    if let Ok(mut state) = self.state.lock() {
                        state.tx_tune_armed = armed;
                    }
                }
                ui.add_space(4.0);

                // ── Parameters (static for now; not yet interactive) ─────────
                egui::Grid::new("tx_tune_params_grid")
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("Duration");
                        ui.label("250 ms");
                        ui.end_row();

                        ui.label("Drive");
                        ui.label("minimum");
                        ui.end_row();
                    });
                ui.add_space(4.0);

                // ── Measure SWR — always disabled until TX is implemented ────
                ui.add_enabled(false, egui::Button::new("Measure SWR"));
                ui.add_space(4.0);

                // ── Last result ──────────────────────────────────────────────
                ui.label(egui::RichText::new("Last Result").strong());

                let result = &snapshot.last_tx_tune_result;
                egui::Grid::new("tx_tune_result_grid")
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        ui.label("Forward");
                        ui.label(format_power_w(result.forward_power_w));
                        ui.end_row();

                        ui.label("Reverse");
                        ui.label(format_power_w(result.reverse_power_w));
                        ui.end_row();

                        ui.label("SWR");
                        ui.label(format_swr(result.swr));
                        ui.end_row();

                        ui.label("Result");
                        ui.label(result.message.as_deref().unwrap_or("--"));
                        ui.end_row();
                    });
            });
    }
}

fn format_power_w(power: Option<f32>) -> String {
    match power {
        Some(w) => format!("{w:.1} W"),
        None => "--".to_string(),
    }
}

fn format_swr(swr: Option<f32>) -> String {
    match swr {
        Some(s) => format!("{s:.1}:1"),
        None => "--".to_string(),
    }
}

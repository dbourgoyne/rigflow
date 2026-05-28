use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    /// Draw the "TX Tune Test" panel.
    ///
    /// Only shown when the active source advertises `supports_tx_tune_test`.
    ///
    /// # Safety invariants
    ///
    /// - "Measure SWR" is enabled only when the arm checkbox is checked.
    /// - Clicking "Measure SWR" sends `RequestTxTuneTest { duration_ms: 250,
    ///   drive: 0.0 }`.  The server executes a dry run: PTT is never asserted,
    ///   drive is forced to 0, no RF is produced.
    /// - `tx_tune_armed` defaults to `false` and is never persisted.
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

                // ── Dry-run mode notice ──────────────────────────────────────
                ui.label(
                    egui::RichText::new("Dry run only — RF disabled")
                        .color(egui::Color32::from_rgb(120, 180, 255))
                        .small(),
                );
                ui.add_space(4.0);

                // ── Arm checkbox ─────────────────────────────────────────────
                let mut armed = snapshot.tx_tune_armed;
                if ui.checkbox(&mut armed, "Arm TX tune test").changed() {
                    if let Ok(mut state) = self.state.lock() {
                        state.tx_tune_armed = armed;
                    }
                }
                ui.add_space(4.0);

                // ── Parameters ───────────────────────────────────────────────
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

                // ── Measure SWR — enabled only when armed ────────────────────
                let can_measure = snapshot.tx_tune_armed;
                let clicked = ui
                    .add_enabled(can_measure, egui::Button::new("Measure SWR"))
                    .clicked();

                if clicked {
                    self.send_radio_msg(ClientRadioMessage::RequestTxTuneTest {
                        duration_ms: 250,
                        drive: 0.0,
                    });
                }
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
                        ui.label(format_result_message(result.message.as_deref()));
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

fn format_result_message(msg: Option<&str>) -> &str {
    match msg {
        None => "--",
        Some("dry_run_completed") => "Dry run completed (no RF)",
        Some("not_supported") => "Not supported",
        Some(m) if m.starts_with("rejected:") => {
            // Strip the prefix for display; fall through returns the raw string
            // for unknown rejection codes.
            match m {
                "rejected:frequency_out_of_hf_range" => "Rejected: frequency out of HF range",
                other => other,
            }
        }
        Some(other) => other,
    }
}

use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;
use rigflow_core::radio::tx_tune::TxTuneStatus;
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    /// Draw the "TX Tune Test" panel.
    ///
    /// Only shown when the active source advertises `supports_tx_tune_test`.
    ///
    /// # Safety invariants
    ///
    /// - "Measure SWR" is enabled only when the arm checkbox is checked AND
    ///   no test is currently running.
    /// - Clicking "Measure SWR" sends `RequestTxTuneTest { duration_ms: 250,
    ///   drive: 0.0 }` and immediately disarms + sets `tx_tune_running = true`.
    /// - `tx_tune_armed` and `tx_tune_running` are never persisted.
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
                ui.add_space(2.0);

                // ── State indicator ──────────────────────────────────────────
                let (state_text, state_color) = if snapshot.tx_tune_running {
                    ("● Running…", egui::Color32::from_rgb(100, 220, 100))
                } else {
                    let s = snapshot.last_tx_tune_result.status;
                    (format_status(s), status_color(s))
                };
                ui.label(egui::RichText::new(state_text).color(state_color).small());
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
                        ui.label("minimum (~-26 dBFS)");
                        ui.end_row();
                    });
                ui.add_space(4.0);

                // ── Measure SWR button ───────────────────────────────────────
                // Enabled only when armed and not already running.
                let can_measure = snapshot.tx_tune_armed && !snapshot.tx_tune_running;
                let clicked = ui
                    .add_enabled(can_measure, egui::Button::new("Measure SWR"))
                    .clicked();

                if clicked {
                    self.send_radio_msg(ClientRadioMessage::RequestTxTuneTest {
                        duration_ms: 250,
                        drive: 0.0,
                    });
                    // Disarm and mark running immediately — the result will
                    // arrive via RuntimeChanged once the test completes.
                    if let Ok(mut state) = self.state.lock() {
                        state.tx_tune_armed = false;
                        state.tx_tune_running = true;
                    }
                }
                ui.add_space(4.0);

                // ── Last result grid ─────────────────────────────────────────
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

                        ui.label("Status");
                        ui.label(
                            egui::RichText::new(format_status(result.status))
                                .color(status_color(result.status)),
                        );
                        ui.end_row();

                        if let Some(msg) = &result.message {
                            ui.label("Message");
                            ui.label(msg.as_str());
                            ui.end_row();
                        }
                    });
            });
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

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

fn format_status(status: TxTuneStatus) -> &'static str {
    match status {
        TxTuneStatus::NotRun => "--",
        TxTuneStatus::Ok => "OK",
        TxTuneStatus::NoForwardPower => "No forward power",
        TxTuneStatus::HighSwr => "High SWR",
        TxTuneStatus::TxInhibited => "TX inhibited",
        TxTuneStatus::InvalidFrequency => "Invalid frequency",
        TxTuneStatus::Timeout => "Timeout",
        TxTuneStatus::Underflow => "FIFO underflow",
        TxTuneStatus::Overflow => "FIFO overflow",
        TxTuneStatus::Fault => "Fault",
    }
}

fn status_color(status: TxTuneStatus) -> egui::Color32 {
    match status {
        TxTuneStatus::NotRun => egui::Color32::GRAY,
        TxTuneStatus::Ok => egui::Color32::WHITE,
        TxTuneStatus::NoForwardPower | TxTuneStatus::HighSwr => {
            egui::Color32::from_rgb(255, 200, 50)
        }
        _ => egui::Color32::from_rgb(255, 80, 80),
    }
}

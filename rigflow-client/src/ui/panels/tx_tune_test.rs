use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;
use rigflow_core::radio::tx_tune::TxTuneStatus;
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    /// Draw the "Spot / SWR" panel.
    ///
    /// Transmits a pure, unmodulated single-frequency carrier at the current
    /// TX frequency (Quisk-style Spot) for SWR measurement.  Only shown when
    /// the active source advertises `supports_tx_tune_test`.
    ///
    /// TX power is the operator's **TX Drive (%)** set in HL2 Source Control;
    /// this panel only displays it read-only and uses it for the measurement.
    ///
    /// # Behaviour
    ///
    /// - "Measure SWR" is enabled whenever no measurement is already running.
    /// - Clicking it sends `RequestTxTuneTest { duration_ms: 250 }` and sets
    ///   `tx_tune_running = true`; the result arrives via `RuntimeChanged`.
    /// - TX power is the configured source `tx_drive_percent` (read server-side
    ///   from Source Control); this panel does not carry a drive value.
    pub(crate) fn draw_tx_tune_test_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if !snapshot.radio_acquired {
            return;
        }

        if !snapshot.source_capabilities.supports_tx_tune_test {
            return;
        }

        // Drawn inline inside the Source Control "Diagnostics" section.
        ui.separator();
        ui.label(egui::RichText::new("Spot / SWR").strong());
        {
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

            // ── Current TX Drive (read-only) ─────────────────────────────
            // TX Drive is configured in HL2 Source Control; shown here for
            // reference only.  No slider in this panel.
            ui.label(format!(
                "Current TX Drive: {:.0}%",
                snapshot.source_control.tx_drive_percent
            ));

            // ── Spot Level (%) — digital carrier IQ amplitude ────────────
            // Quisk's Spot-slider equivalent: amplitude_fs = spot_level/100.
            // Spot RF power ≈ TX Drive × Spot Level.  Affects ONLY Spot/SWR
            // (and the SWR sweep) — not voice/CW/digital TX.  Persisted via
            // the source-control prefs.
            // Damage-adjacent (TX carrier amplitude), so gate it behind its own
            // inline lock — independent of TX Drive.  `self` is kept out of the
            // horizontal closure; the lock toggle + value change are applied after.
            let mut locked = snapshot.spot_level_locked;
            let mut toggle_to: Option<bool> = None;
            let mut new_spot: Option<f32> = None;
            ui.horizontal(|ui| {
                super::lock_button(ui, &mut locked, super::LOCK_SMALL);
                if locked != snapshot.spot_level_locked {
                    toggle_to = Some(locked);
                }
                let mut spot_level = snapshot.source_control.spot_level_percent;
                let mut resp = ui.add_enabled(
                    !locked,
                    egui::Slider::new(&mut spot_level, 0.0..=100.0)
                        .step_by(1.0)
                        .fixed_decimals(0)
                        .suffix("%")
                        .text("Spot Level"),
                );
                if locked {
                    resp = resp.on_hover_text("Locked — click the padlock to change Spot Level");
                } else {
                    super::slider_scroll(ui, &mut resp, &mut spot_level, 0.0, 100.0, 1.0);
                }
                if resp.changed() {
                    new_spot = Some(spot_level.clamp(0.0, 100.0).round());
                }
            });
            if let Some(l) = toggle_to {
                if let Ok(mut state) = self.state.lock() {
                    state.spot_level_locked = l;
                    if !l {
                        state.spot_level_unlocked_at = Some(std::time::Instant::now());
                    }
                }
            }
            if let Some(snapped) = new_spot {
                self.send_radio_msg(ClientRadioMessage::SetSourceSpotLevel {
                    spot_level_percent: snapped,
                });
                if let Ok(mut state) = self.state.lock() {
                    state.source_control.spot_level_percent = snapped;
                    state.spot_level_unlocked_at = Some(std::time::Instant::now());
                }
                self.save_source_control_prefs_to_current_operator();
            }

            ui.label(super::note_text(
                "Carrier amplitude for Spot/SWR (default 50%) · \
                 adjust transmitter power via TX Drive in Source Control",
            ));
            ui.add_space(4.0);

            // ── Measure SWR button ───────────────────────────────────────
            // Enabled when no measurement is already running.  TX power is
            // the configured source TX Drive (the server reads it).
            let can_measure = !snapshot.tx_tune_running;
            let clicked = ui
                .add_enabled(can_measure, egui::Button::new("Measure SWR"))
                .clicked();

            if clicked {
                self.send_radio_msg(ClientRadioMessage::RequestTxTuneTest { duration_ms: 250 });
                // Mark running immediately — the result arrives via
                // RuntimeChanged once the measurement completes.
                if let Ok(mut state) = self.state.lock() {
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
                    ui.label("SWR");
                    ui.label(format_swr(result.swr));
                    ui.end_row();

                    ui.label("Forward Raw");
                    ui.label(format_raw(result.forward_raw));
                    ui.end_row();

                    ui.label("Reverse Raw");
                    ui.label(format_raw(result.reverse_raw));
                    ui.end_row();

                    ui.label("Current Raw");
                    ui.label(format_raw(result.current_raw));
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
        }
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

fn format_raw(raw: Option<u16>) -> String {
    match raw {
        Some(n) => n.to_string(),
        None => "--".to_string(),
    }
}

fn format_swr(swr: Option<f32>) -> String {
    match swr {
        Some(s) => format!("{s:.2}:1"),
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

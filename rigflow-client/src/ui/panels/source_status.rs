use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;
use rigflow_core::radio::amplifier::{AmplifierAtuMode, AmplifierKeyingMode, AmplifierStatus};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_protocol::radio_control::ClientRadioMessage;

impl RigflowApp {
    /// Draw the read-only "Source Status" pane.
    ///
    /// The pane is only shown when the active source reports at least one
    /// status field (i.e. `SourceStatus::has_any()` returns `true`).
    /// For RTL-SDR and other sources that leave all fields `None`, this
    /// pane is hidden and takes up no space.
    pub(crate) fn draw_source_status_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        if !snapshot.radio_acquired {
            return;
        }

        if !snapshot.source_status.has_any() {
            return;
        }

        let status = &snapshot.source_status;

        // Drawn inline inside the Source Control "Status" section (the section
        // header provides the heading; no inner collapsible).
        // Detected band (HL2): derived from the tuned target frequency, so it
        // tracks any tuning regardless of how it occurred.
        if snapshot.source_capabilities.supports_band_control {
            let band = rigflow_core::radio::ham_band::band_from_frequency(
                snapshot.target_freq_hz.max(0.0) as u64,
            );
            egui::Grid::new("source_status_band_grid")
                .num_columns(2)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    ui.label("Band");
                    ui.label(band.map(|b| b.label()).unwrap_or("—"));
                    ui.end_row();
                });
            ui.add_space(4.0);
        }

        draw_health_group(ui, status);
        ui.add_space(4.0);
        draw_rf_power_group(ui, status);

        // Generic amplifier row — always shown (Phase 1: HR50).  Kept
        // amplifier-agnostic so future models need no UI redesign.
        ui.add_space(4.0);
        self.draw_amplifier_section(ui, &snapshot.amplifier_status);
    }

    /// Amplifier section: generic "Amplifier: <model|None>" row plus, when an amp
    /// is present, status read-outs and Phase 2 controls.  The amp's reported
    /// status is the single source of truth — controls just send a SET and the
    /// next poll reflects it.
    fn draw_amplifier_section(&self, ui: &mut egui::Ui, amp: &AmplifierStatus) {
        egui::Grid::new("amplifier_status_grid")
            .num_columns(2)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                ui.label("Amplifier");
                match amp.model {
                    Some(model) => ui.strong(model.label()),
                    None => ui.label("None"),
                };
                ui.end_row();

                if amp.model.is_none() {
                    return;
                }

                // Mode — selector (control): reflects HRRX-reported mode, sets HRMD.
                ui.label("    Mode");
                let current = amp
                    .mode
                    .as_deref()
                    .and_then(AmplifierKeyingMode::from_label);
                let mut selected = current.unwrap_or(AmplifierKeyingMode::Off);
                egui::ComboBox::from_id_source("amp_keying_mode")
                    .selected_text(amp.mode.as_deref().unwrap_or("—"))
                    .show_ui(ui, |ui| {
                        for m in AmplifierKeyingMode::ALL {
                            if ui.selectable_value(&mut selected, m, m.label()).clicked() {
                                self.send_radio_msg(ClientRadioMessage::SetAmplifierKeyingMode {
                                    mode: m,
                                });
                            }
                        }
                    });
                ui.end_row();

                ui.label("    Band");
                ui.label(amp.band.as_deref().unwrap_or("—"));
                ui.end_row();

                ui.label("    Temperature");
                ui.label(
                    amp.temperature_c
                        .map(|t| format!("{t:.0} °C"))
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.end_row();

                ui.label("    Voltage");
                ui.label(
                    amp.voltage_v
                        .map(|v| format!("{v:.1} V"))
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.end_row();

                // Last-transmission telemetry (HRMX), shown once a TX has happened.
                if amp.tx_pep_w.is_some() || amp.tx_avg_w.is_some() || amp.tx_swr.is_some() {
                    ui.label("    Last TX");
                    let pep = amp
                        .tx_pep_w
                        .map(|w| format!("PEP {w:.0} W"))
                        .unwrap_or_default();
                    let avg = amp
                        .tx_avg_w
                        .map(|w| format!("Avg {w:.0} W"))
                        .unwrap_or_default();
                    let swr = amp
                        .tx_swr
                        .map(|s| format!("SWR {s:.1}"))
                        .unwrap_or_else(|| "SWR —".to_string());
                    ui.label(format!("{pep}  {avg}  {swr}").trim().to_string());
                    ui.end_row();
                }

                // ATU controls — only when the amp reports an ATU is installed.
                if amp.atu_present {
                    ui.label("    ATU");
                    ui.horizontal(|ui| {
                        let active = matches!(amp.atu_mode, Some(AmplifierAtuMode::Active));
                        if ui.selectable_label(!active, "Bypass").clicked() {
                            self.send_radio_msg(ClientRadioMessage::SetAmplifierAtuMode {
                                mode: AmplifierAtuMode::Bypass,
                            });
                        }
                        if ui.selectable_label(active, "Active").clicked() {
                            self.send_radio_msg(ClientRadioMessage::SetAmplifierAtuMode {
                                mode: AmplifierAtuMode::Active,
                            });
                        }
                        if ui.button("Tune").clicked() {
                            self.send_radio_msg(ClientRadioMessage::TuneAmplifierAtu);
                        }
                    });
                    ui.end_row();
                }
            });
    }
}

/// "Health" group: firmware version, ADC overload, temperature, current.
fn draw_health_group(ui: &mut egui::Ui, status: &SourceStatus) {
    // Only draw this group if at least one health field is present.
    let has_health = status.firmware_version.is_some()
        || status.adc_overload.is_some()
        || status.temperature_c.is_some()
        || status.current_a.is_some()
        || status.tx_inhibited.is_some()
        || status.recovery_status.is_some();

    if !has_health {
        return;
    }

    ui.label(egui::RichText::new("Health").strong());

    egui::Grid::new("source_status_health_grid")
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            if let Some(ref ver) = status.firmware_version {
                ui.label("Firmware");
                ui.label(ver);
                ui.end_row();
            }

            if let Some(overload) = status.adc_overload {
                ui.label("ADC");
                if overload {
                    ui.label(
                        egui::RichText::new("⚠ OVERLOAD")
                            .color(egui::Color32::from_rgb(255, 80, 40))
                            .strong(),
                    );
                } else {
                    ui.label(egui::RichText::new("OK").color(egui::Color32::from_rgb(80, 200, 80)));
                }
                ui.end_row();
            }

            if let Some(temp) = status.temperature_c {
                ui.label("Temperature");
                ui.label(format!("{temp:.1} °C"));
                ui.end_row();
            }

            if let Some(current) = status.current_a {
                ui.label("Current");
                ui.label(format!("{current:.2} A"));
                ui.end_row();
            }

            if let Some(inhibited) = status.tx_inhibited {
                ui.label("TX Status");
                if inhibited {
                    ui.label(
                        egui::RichText::new("Inhibited")
                            .color(egui::Color32::from_rgb(255, 160, 40)),
                    );
                } else {
                    ui.label("Allowed");
                }
                ui.end_row();
            }

            if let Some(ref recovery) = status.recovery_status {
                ui.label("Recovery");
                if recovery == "OK" {
                    ui.label(recovery.as_str());
                } else {
                    ui.label(
                        egui::RichText::new(recovery.as_str())
                            .color(egui::Color32::from_rgb(255, 200, 40))
                            .strong(),
                    );
                }
                ui.end_row();
            }
        });
}

/// "RF Power" group: forward power, reverse power, SWR.
fn draw_rf_power_group(ui: &mut egui::Ui, status: &SourceStatus) {
    // Only draw this group when at least one RF power field is present
    // (including SWR which is derived from them).
    let has_rf = status.forward_power_w.is_some()
        || status.reverse_power_w.is_some()
        || status.swr.is_some();

    if !has_rf {
        return;
    }

    ui.label(egui::RichText::new("RF Power").strong());

    egui::Grid::new("source_status_rf_grid")
        .num_columns(2)
        .spacing([8.0, 2.0])
        .show(ui, |ui| {
            ui.label("Forward");
            ui.label(format_power_w(status.forward_power_w));
            ui.end_row();

            ui.label("Reverse");
            ui.label(format_power_w(status.reverse_power_w));
            ui.end_row();

            ui.label("SWR");
            ui.label(format_swr(status.swr));
            ui.end_row();
        });
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

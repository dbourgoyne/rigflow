use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;
use rigflow_core::radio::source_status::SourceStatus;

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

        egui::CollapsingHeader::new("Source Status")
            .default_open(true)
            .show(ui, |ui| {
                draw_health_group(ui, status);
                ui.add_space(4.0);
                draw_rf_power_group(ui, status);
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

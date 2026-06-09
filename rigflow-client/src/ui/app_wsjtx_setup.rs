//! WSJT-X / FT8 setup helper window.
//!
//! Read-mostly reference popup that shows the exact values to enter in WSJT-X
//! (the virtual audio device names and the CAT host/port), each with a Copy
//! button and a live status chip, plus the one operational control a digital
//! user needs (RX decode routing).  Reuses the Phase A availability surfacing.

use super::app::RigflowApp;
use crate::digital_audio::{DIGITAL_INPUT_NAME, DIGITAL_RX_NAME};
use crate::rigctl_server::DEFAULT_RIGCTL_PORT;
use eframe::egui;

const RIGCTL_HOST: &str = "127.0.0.1";
const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(100, 200, 100);
const BAD_RED: egui::Color32 = egui::Color32::from_rgb(210, 130, 130);

impl RigflowApp {
    /// WSJT-X / FT8 setup helper.  Opened from the Radio Control panel; closing
    /// the window only hides it (the flag is transient, not persisted).
    pub(crate) fn draw_wsjtx_setup_window(&mut self, ctx: &egui::Context) {
        let mut open = {
            let state = self.state.lock().unwrap();
            state.show_wsjtx_setup_window
        };

        if !open {
            return;
        }

        // Snapshot what we need so the window closure never touches `self`.
        let (rx_avail, rx_reason, in_avail, in_reason, rigctl_status, rx_active, rx_routing) = {
            let state = self.state.lock().unwrap();
            (
                state.digital_rx_available,
                state.digital_rx_reason.clone(),
                state.digital_input_available,
                state.digital_input_reason.clone(),
                state.rigctl_status.clone(),
                state.digital_rx.is_active(),
                state.digital_rx.is_enabled(),
            )
        };

        let network_server = format!("{RIGCTL_HOST}:{DEFAULT_RIGCTL_PORT}");
        let mut set_rx_routing: Option<bool> = None;

        egui::Window::new("WSJT-X / FT8 Setup")
            .open(&mut open)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label("Set these in WSJT-X (File → Settings) to talk to Rigflow:");
                ui.add_space(6.0);

                egui::Grid::new("wsjtx_setup_grid")
                    .num_columns(4)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        value_row(
                            ui,
                            "Audio → Input",
                            DIGITAL_RX_NAME,
                            Some((rx_avail, rx_reason.as_deref(), "Available")),
                        );
                        value_row(
                            ui,
                            "Audio → Output",
                            DIGITAL_INPUT_NAME,
                            Some((in_avail, in_reason.as_deref(), "Available")),
                        );
                        info_row(ui, "Radio → Rig", "Hamlib NET rigctl");
                        value_row(
                            ui,
                            "Radio → Network Server",
                            &network_server,
                            Some((
                                rigctl_status.is_none(),
                                rigctl_status.as_deref(),
                                "Listening",
                            )),
                        );
                        info_row(ui, "Radio → PTT Method", "CAT");
                        info_row(ui, "Radio → Mode", "Data/Pkt");
                    });

                ui.add_space(8.0);
                ui.separator();

                // The one operational control a digital user needs here.
                let mut rx_on = rx_routing;
                if ui
                    .checkbox(
                        &mut rx_on,
                        "RX Digital Output (route received audio to WSJT-X)",
                    )
                    .changed()
                {
                    set_rx_routing = Some(rx_on);
                }
                ui.horizontal(|ui| {
                    ui.label("Routing:");
                    if rx_active {
                        ui.colored_label(OK_GREEN, "Active");
                    } else {
                        ui.colored_label(egui::Color32::GRAY, "Inactive");
                    }
                });

                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Acquire a radio, set its mode to USB/Data, and enable RX Digital \
                         Output for decoding.",
                    )
                    .small()
                    .weak(),
                );
            });

        if let Some(on) = set_rx_routing {
            let state = self.state.lock().unwrap();
            state.digital_rx.set_enabled(on);
        }

        // Reflect the window's close affordance back into state.
        if let Ok(mut state) = self.state.lock() {
            state.show_wsjtx_setup_window = open;
        }
    }
}

/// A row with a copyable value and a live status chip:
/// `<where in WSJT-X> | <value monospace> | [Copy] | <status>`.
fn value_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    status: Option<(bool, Option<&str>, &str)>,
) {
    ui.label(label);
    ui.monospace(value);
    if ui.button("Copy").clicked() {
        ui.ctx().copy_text(value.to_string());
    }
    match status {
        Some((true, _, ok_label)) => {
            ui.colored_label(OK_GREEN, ok_label);
        }
        Some((false, reason, _)) => {
            let text = match reason {
                Some(r) if !r.is_empty() => format!("Unavailable: {r}"),
                _ => "Unavailable".to_string(),
            };
            ui.colored_label(BAD_RED, text);
        }
        None => {
            ui.label("");
        }
    }
    ui.end_row();
}

/// A reference row with no copy/status (e.g. a dropdown selection in WSJT-X).
fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(label);
    ui.monospace(value);
    ui.label("");
    ui.label("");
    ui.end_row();
}

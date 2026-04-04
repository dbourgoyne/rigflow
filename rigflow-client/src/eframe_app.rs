use std::sync::{Arc, Mutex};

use eframe::egui;
use tokio::sync::mpsc;

use crate::app::state::UiState;
use crate::net::control::ControlCommand;

pub struct RigflowApp {
    pub state: Arc<Mutex<UiState>>,
    pub ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
}

impl RigflowApp {
    pub fn new(
        state: Arc<Mutex<UiState>>,
        ws_cmd_tx: mpsc::UnboundedSender<ControlCommand>,
    ) -> Self {
        Self { state, ws_cmd_tx }
    }
}

impl eframe::App for RigflowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = {
            let state = self.state.lock().unwrap();
            state.clone()
        };

        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("rigflow");
                ui.separator();

                egui::CollapsingHeader::new("Rigflow Server")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label("rigflow server IP:");

                        let mut ip = snapshot.rigflow_server_ip.clone();
                        if ui.text_edit_singleline(&mut ip).changed() {
                            if let Ok(mut state) = self.state.lock() {
                                state.rigflow_server_ip = ip;
                            }
                        }

                        ui.add_space(8.0);

                        let button_text = if snapshot.server_connected {
                            "Disconnect"
                        } else {
                            "Connect"
                        };

                        if ui.button(button_text).clicked() {
                            let ip = snapshot.rigflow_server_ip.trim().to_string();

                            if snapshot.server_connected {
                                let _ = self.ws_cmd_tx.send(ControlCommand::Disconnect);
                            } else if ip.is_empty() {
                                if let Ok(mut state) = self.state.lock() {
                                    state.server_status =
                                        "connect failed: missing server IP".to_string();
                                }
                            } else {
                                let _ = self.ws_cmd_tx.send(ControlCommand::Connect {
                                    server_ip: ip,
                                });
                            }
                        }

                        ui.add_space(8.0);
                        ui.label("Status:");
                        ui.monospace(&snapshot.server_status);
                    });

                ui.separator();

                egui::CollapsingHeader::new("Radios")
                    .default_open(true)
                    .show(ui, |ui| {
                        if snapshot.available_radios.is_empty() {
                            ui.label("no radios");
                        } else {
                            for radio in &snapshot.available_radios {
                                ui.label(format!(
                                    "{}{}",
                                    radio.display_name,
                                    if radio.is_leased { " (busy)" } else { "" }
                                ));
                            }
                        }
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Spectrum / Waterfall Placeholder");
            ui.separator();

            let available = ui.available_size();
            let (rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());

            ui.painter()
                .rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 20, 24));

            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "central render area",
                egui::TextStyle::Heading.resolve(ui.style()),
                egui::Color32::LIGHT_GRAY,
            );
        });

        ctx.request_repaint();
    }
}

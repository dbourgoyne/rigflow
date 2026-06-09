use crate::UiState;
use crate::net::control::ControlCommand;
use crate::ui::app::RigflowApp;
use eframe::egui;

impl RigflowApp {
    pub(crate) fn draw_server_panel(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &UiState,
        config_mode: bool,
    ) {
        egui::CollapsingHeader::new("Rigflow Server")
            // Open by default while not connected so a first-run user sees the
            // Connect button without having to expand a collapsed header.
            .default_open(config_mode)
            .show(ui, |ui| {
                ui.label("rigflow server IP:");

                let mut ip = snapshot.rigflow_server_ip.clone();
                ui.add_enabled_ui(config_mode, |ui| {
                    if ui.text_edit_singleline(&mut ip).changed() {
                        if let Ok(mut state) = self.state.lock() {
                            state.rigflow_server_ip = ip.clone();
                        }

                        self.save_server_ip();
                    }
                });

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
                            state.server_status = "connect failed: missing server IP".to_string();
                        }
                    } else {
                        let _ = self
                            .ws_cmd_tx
                            .send(ControlCommand::Connect { server_ip: ip });
                    }
                }

                ui.add_space(8.0);
                ui.label("Status:");
                ui.monospace(&snapshot.server_status);
            });
    }
}

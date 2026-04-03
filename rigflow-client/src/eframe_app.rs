use eframe::egui;

#[derive(Debug, Clone)]
pub struct AppState {
    pub rigflow_server_ip: String,
    pub server_connected: bool,
    pub server_status: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            rigflow_server_ip: "192.168.0.225".to_string(),
            server_connected: false,
            server_status: "no server".to_string(),
        }
    }
}

#[derive(Default)]
pub struct RigflowApp {
    pub state: AppState,
}

impl eframe::App for RigflowApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

                        ui.text_edit_singleline(&mut self.state.rigflow_server_ip);

                        ui.add_space(8.0);

                        let button_text = if self.state.server_connected {
                            "Disconnect"
                        } else {
                            "Connect"
                        };

                        if ui.button(button_text).clicked() {
                            if self.state.server_connected {
                                self.state.server_connected = false;
                                self.state.server_status = "no server".to_string();
                            } else {
                                let ip = self.state.rigflow_server_ip.trim();

                                if ip.is_empty() {
                                    self.state.server_status =
                                        "connect failed: missing server IP".to_string();
                                } else {
                                    self.state.server_connected = true;
                                    self.state.server_status =
                                        format!("connected to server {}", ip);
                                }
                            }
                        }

                        ui.add_space(8.0);
                        ui.label("Status:");
                        ui.monospace(&self.state.server_status);
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Spectrum / Waterfall Placeholder");
            ui.separator();
            ui.label("Later this will host the SDR visualizations.");
            ui.add_space(12.0);

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
    }
}

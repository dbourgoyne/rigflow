use crate::net::control::ControlCommand;
use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;

impl RigflowApp {
    pub(crate) fn draw_radios_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        egui::CollapsingHeader::new("Radios")
            .default_open(true)
            .show(ui, |ui| {
                if snapshot.available_radios.is_empty() | !snapshot.server_connected {
                    ui.label("no radios");
                } else {
                    let mut selected = snapshot.selected_radio_id.clone();

                    for radio in &snapshot.available_radios {
                        let label = if radio.is_leased {
                            format!("{} (busy)", radio.display_name)
                        } else {
                            radio.display_name.clone()
                        };

                        let is_selected = selected.as_deref() == Some(&radio.id.0);

                        let response = ui.selectable_label(is_selected, label);

                        if response.double_clicked() {
                            selected = Some(radio.id.0.clone());

                            if let Some(radio_id) = selected.clone() {
                                let _ = self
                                    .ws_cmd_tx
                                    .send(ControlCommand::AcquireRadio { radio_id });
                            }
                        } else if response.clicked() {
                            selected = Some(radio.id.0.clone());
                        }
                    }

                    if selected != snapshot.selected_radio_id {
                        if let Ok(mut state) = self.state.lock() {
                            state.selected_radio_id = selected.clone();

                            if let Some(selected_id) = selected.as_deref() {
                                if let Some(radio) = state
                                    .available_radios
                                    .iter()
                                    .find(|radio| radio.id.0 == selected_id)
                                {
                                    state.source_capabilities = radio.source_capabilities.clone();
                                }
                            }
                        }
                    }

                    ui.add_space(8.0);

                    let can_acquire = selected.is_some() && !snapshot.radio_acquired;
                    let can_release = snapshot.radio_acquired;

                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(can_acquire, egui::Button::new("Acquire"))
                            .clicked()
                        {
                            if let Some(radio_id) = selected.clone() {
                                let _ = self
                                    .ws_cmd_tx
                                    .send(ControlCommand::AcquireRadio { radio_id });
                            }
                        }

                        if ui
                            .add_enabled(can_release, egui::Button::new("Release"))
                            .clicked()
                        {
                            let _ = self.ws_cmd_tx.send(ControlCommand::ReleaseRadio);
                        }
                    });
                }
            });
    }
}

use crate::net::control::ControlCommand;
use crate::ui::app::RigflowApp;
use crate::UiState;
use eframe::egui;
use rigflow_core::radio::RadioSourceKind;
use rigflow_protocol::ClientRadioMessage;

/// Fixed display order for the radio categories (server-provided `source_kind`;
/// the client never infers this from names).  Lower = shown first.
fn category_order(kind: RadioSourceKind) -> u8 {
    match kind {
        RadioSourceKind::Hardware => 0,
        RadioSourceKind::Recording => 1,
        RadioSourceKind::Virtual => 2,
        RadioSourceKind::Unknown => 3,
    }
}

/// Category header text.
fn category_label(kind: RadioSourceKind) -> &'static str {
    match kind {
        RadioSourceKind::Hardware => "Hardware",
        RadioSourceKind::Recording => "Recordings",
        RadioSourceKind::Virtual => "Virtual",
        RadioSourceKind::Unknown => "Other",
    }
}

impl RigflowApp {
    pub(crate) fn draw_radios_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        egui::CollapsingHeader::new(super::panel_header("Radios"))
            .default_open(true)
            .show(ui, |ui| {
                if snapshot.available_radios.is_empty() | !snapshot.server_connected {
                    ui.label("no radios");
                } else {
                    let mut selected = snapshot.selected_radio_id.clone();

                    // Group + order by the server-provided `source_kind`:
                    // (category_order, display_name asc).  Categories always
                    // appear in the same order; empty ones are simply absent.
                    let mut ordered: Vec<_> = snapshot.available_radios.iter().collect();
                    ordered.sort_by(|a, b| {
                        category_order(a.source_kind)
                            .cmp(&category_order(b.source_kind))
                            .then_with(|| {
                                a.display_name
                                    .to_lowercase()
                                    .cmp(&b.display_name.to_lowercase())
                            })
                    });

                    let mut current_category: Option<RadioSourceKind> = None;
                    for radio in ordered {
                        // Emit a header when the category changes.
                        if current_category != Some(radio.source_kind) {
                            if current_category.is_some() {
                                ui.add_space(6.0);
                            }
                            ui.label(
                                egui::RichText::new(category_label(radio.source_kind)).strong(),
                            );
                            ui.separator();
                            current_category = Some(radio.source_kind);
                        }

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

                        // Re-scan the server for radios (e.g. to pick up a
                        // freshly recorded WAV file) without restarting it.
                        if ui
                            .add_enabled(!snapshot.radio_acquired, egui::Button::new("⟳ Rescan"))
                            .on_hover_text("Re-scan for radios (incl. new IQ recordings)")
                            .clicked()
                        {
                            self.send_radio_msg(ClientRadioMessage::RescanRadios);
                        }
                    });
                }
            });
    }
}

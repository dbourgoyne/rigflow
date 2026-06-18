use crate::persistence::apply_operator_settings_to_ui_state;
use crate::ui::app::RigflowApp;
use crate::ui::om_bands::LicenseClass;
use crate::UiState;
use eframe::egui;

impl RigflowApp {
    pub(crate) fn draw_operator_panel(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &UiState,
        config_mode: bool,
    ) {
        // Auto-expand only on first run — no operator yet, while disconnected —
        // to match the "add an operator" cue.  A returning operator stays
        // collapsed (don't distract them just because they're disconnected).
        egui::CollapsingHeader::new(super::panel_header("Radio Operator"))
            .default_open(config_mode && snapshot.known_operator_ids.is_empty())
            .show(ui, |ui| {
                if !config_mode {
                    ui.label("Disconnect from the server to change operator settings.");
                    ui.add_space(6.0);
                }

                ui.add_enabled_ui(config_mode, |ui| {
                    ui.label("Current operator:");

                    let mut selected_operator = if snapshot.operator_id.trim().is_empty() {
                        None
                    } else {
                        Some(snapshot.operator_id.clone())
                    };

                    egui::ComboBox::from_id_salt("operator_combo")
                        .selected_text(
                            selected_operator
                                .clone()
                                .unwrap_or_else(|| "none".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for operator_id in &snapshot.known_operator_ids {
                                ui.selectable_value(
                                    &mut selected_operator,
                                    Some(operator_id.clone()),
                                    operator_id,
                                );
                            }
                        });

                    if selected_operator != Some(snapshot.operator_id.clone()) {
                        if let Some(operator_id) = selected_operator {
                            match self
                                .persistence_store
                                .load_or_create_operator_settings(&operator_id)
                            {
                                Ok(operator_settings) => {
                                    match self.persistence_store.load_app_state() {
                                        Ok(mut app_state) => {
                                            app_state.last_operator_id = Some(operator_id.clone());

                                            if let Err(err) =
                                                self.persistence_store.save_app_state(&app_state)
                                            {
                                                if let Ok(mut state) = self.state.lock() {
                                                    state.persistence_status =
                                                        format!("failed to save app state: {err}");
                                                }
                                            }

                                            // Surface any corrupt-config recovery from
                                            // the loads above; otherwise clear status.
                                            let notices =
                                                self.persistence_store.take_recovery_notices();
                                            if let Ok(mut state) = self.state.lock() {
                                                apply_operator_settings_to_ui_state(
                                                    &mut state,
                                                    &operator_settings,
                                                    &app_state,
                                                );
                                                if notices.is_empty() {
                                                    state.persistence_status.clear();
                                                } else {
                                                    state.persistence_status = notices.join("; ");
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            if let Ok(mut state) = self.state.lock() {
                                                state.persistence_status =
                                                    format!("failed to load app state: {err}");
                                            }
                                        }
                                    }
                                }
                                Err(err) => {
                                    if let Ok(mut state) = self.state.lock() {
                                        state.persistence_status =
                                            format!("failed to load operator: {err}");
                                    }
                                }
                            }
                        }
                    }

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        if ui.button("Add Operator").clicked() {
                            if let Ok(mut state) = self.state.lock() {
                                state.show_add_operator_dialog = true;
                                state.pending_operator_id.clear();
                                state.pending_operator_license = None;
                                state.persistence_status.clear();
                            }
                        }

                        if ui
                            .add_enabled(
                                !snapshot.operator_id.trim().is_empty(),
                                egui::Button::new("Delete Operator"),
                            )
                            .clicked()
                        {
                            if let Ok(mut state) = self.state.lock() {
                                state.show_delete_operator_dialog = true;
                                state.pending_delete_operator_id = Some(state.operator_id.clone());
                                state.persistence_status.clear();
                            }
                        }
                    });

                    ui.add_space(8.0);
                    ui.label("License:");

                    let mut selected_license = snapshot.selected_license;

                    ui.radio_value(
                        &mut selected_license,
                        Some(LicenseClass::AmateurExtra),
                        "Amateur Extra",
                    );
                    ui.radio_value(
                        &mut selected_license,
                        Some(LicenseClass::Advanced),
                        "Advanced",
                    );
                    ui.radio_value(
                        &mut selected_license,
                        Some(LicenseClass::General),
                        "General",
                    );
                    ui.radio_value(
                        &mut selected_license,
                        Some(LicenseClass::Technician),
                        "Technician",
                    );
                    ui.radio_value(&mut selected_license, None, "None");

                    if selected_license != snapshot.selected_license {
                        if let Ok(mut state) = self.state.lock() {
                            state.selected_license = selected_license;
                        }
                        self.save_selected_operator_license();
                    }
                });

                if !snapshot.persistence_status.is_empty() {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::RED, &snapshot.persistence_status);
                }
            });
    }
}

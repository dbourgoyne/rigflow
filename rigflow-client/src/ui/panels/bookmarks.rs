use crate::ui::app::RigflowApp;
use eframe::egui;

impl RigflowApp {
    pub(crate) fn draw_bookmarks_panel(&mut self, ui: &mut egui::Ui) {
        ui.collapsing(super::panel_header("Bookmarks"), |ui| {
            let snapshot = {
                let state = self.state.lock().unwrap();
                state.clone()
            };

            if snapshot.bookmarks.is_empty() {
                ui.label("no bookmarks");
            } else {
                for bookmark in &snapshot.bookmarks {
                    let selected = snapshot
                        .selected_bookmark_id
                        .as_ref()
                        .map(|id| id == &bookmark.id)
                        .unwrap_or(false);

                    let mut label = bookmark.name.clone();
                    if snapshot
                        .default_bookmark_id
                        .as_ref()
                        .map(|id| id == &bookmark.id)
                        .unwrap_or(false)
                    {
                        label.push_str("  [default]");
                    }

                    let invalid = snapshot.radio_acquired
                        && !crate::ui::freq_limits::is_freq_valid(bookmark.frequency_hz, &snapshot);

                    let text = if invalid {
                        egui::RichText::new(&label).color(egui::Color32::from_rgb(100, 100, 100))
                    } else {
                        egui::RichText::new(&label)
                    };

                    let response = ui.selectable_label(selected, text);

                    let response = if invalid {
                        if let Some(msg) = crate::ui::freq_limits::bookmark_rejection_message(
                            bookmark.frequency_hz,
                            &snapshot,
                        ) {
                            response.on_hover_text(msg)
                        } else {
                            response
                        }
                    } else {
                        response
                    };

                    if response.double_clicked() {
                        if let Ok(mut state) = self.state.lock() {
                            state.selected_bookmark_id = Some(bookmark.id.clone());
                        }

                        self.apply_bookmark(&bookmark.id);
                    } else if response.clicked() {
                        if let Ok(mut state) = self.state.lock() {
                            state.selected_bookmark_id = Some(bookmark.id.clone());
                        }
                    }
                }

                ui.add_space(8.0);

                ui.label("Notes:");

                let mut edited_notes = snapshot
                    .selected_bookmark_id
                    .as_ref()
                    .and_then(|selected_id| {
                        snapshot
                            .bookmarks
                            .iter()
                            .find(|b| &b.id == selected_id)
                            .and_then(|b| b.notes.clone())
                    })
                    .unwrap_or_default();

                let notes_changed = ui
                    .add_enabled(
                        snapshot.selected_bookmark_id.is_some(),
                        egui::TextEdit::multiline(&mut edited_notes)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    )
                    .changed();

                if notes_changed {
                    self.update_selected_bookmark_notes(edited_notes);
                }

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    let selected_id = snapshot.selected_bookmark_id.clone();

                    if ui
                        .add_enabled(selected_id.is_some(), egui::Button::new("Apply"))
                        .clicked()
                    {
                        if let Some(bookmark_id) = selected_id.clone() {
                            self.apply_bookmark(&bookmark_id);
                        }
                    }

                    if ui
                        .add_enabled(selected_id.is_some(), egui::Button::new("Set Default"))
                        .clicked()
                    {
                        if let Some(bookmark_id) = selected_id.clone() {
                            self.set_default_bookmark(&bookmark_id);
                        }
                    }

                    if ui
                        .add_enabled(selected_id.is_some(), egui::Button::new("Delete"))
                        .clicked()
                    {
                        self.delete_selected_bookmark();
                    }
                });
            }

            ui.add_space(8.0);

            if ui.button("Save Current as Bookmark").clicked() {
                if let Ok(mut state) = self.state.lock() {
                    state.show_add_bookmark_dialog = true;
                    state.pending_bookmark_name.clear();
                    state.pending_bookmark_notes.clear();
                    state.bookmark_status.clear();
                }
            }

            if !snapshot.bookmark_status.is_empty() {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::RED, &snapshot.bookmark_status);
            }

            let auto_apply_changed = if let Ok(mut state) = self.state.lock() {
                ui.checkbox(
                    &mut state.auto_apply_default_bookmark_on_acquire,
                    "Auto-apply default on radio acquire",
                )
                .changed()
            } else {
                false
            };

            if auto_apply_changed {
                self.save_bookmarks_to_current_operator();
            }
        });
    }
}

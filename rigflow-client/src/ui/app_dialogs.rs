use super::app::RigflowApp;
use eframe::egui;

impl RigflowApp {
    
    pub(crate) fn draw_add_bookmark_dialog (
        &mut self,
        ctx: &egui::Context,
    ) {
	let show_add_bookmark_dialog = {
	    let state = self.state.lock().unwrap();
	    state.show_add_bookmark_dialog
	};

	if show_add_bookmark_dialog {
	    egui::Window::new("Save Current as Bookmark")
		.collapsible(false)
		.resizable(false)
		.show(ctx, |ui| {
		    let mut save_requested = false;
		    let mut cancel_requested = false;

		    if let Ok(mut state) = self.state.lock() {
			ui.label("Bookmark name:");
			ui.text_edit_singleline(&mut state.pending_bookmark_name);

			ui.add_space(8.0);
			ui.label("Notes:");
			ui.add(
			    egui::TextEdit::multiline(&mut state.pending_bookmark_notes)
				.desired_rows(4)
				.desired_width(320.0),
			);

			if !state.bookmark_status.is_empty() {
			    ui.add_space(8.0);
			    ui.colored_label(
				egui::Color32::YELLOW,
				&state.bookmark_status,
			    );
			}

			ui.add_space(10.0);

			ui.horizontal(|ui| {
			    if ui.button("Cancel").clicked() {
				cancel_requested = true;
			    }

			    if ui.button("Save").clicked() {
				save_requested = true;
			    }
			});
		    }

		    if cancel_requested {
			if let Ok(mut state) = self.state.lock() {
			    state.show_add_bookmark_dialog = false;
			    state.pending_bookmark_name.clear();
			    state.pending_bookmark_notes.clear();
			    state.bookmark_status.clear();
			}
		    }

		    if save_requested {
			self.save_current_as_bookmark();
		    }
		});
	}

	let default_bookmark_to_apply = {
	    let mut state = self.state.lock().unwrap();

	    if state.pending_apply_default_bookmark {
		state.pending_apply_default_bookmark = false;
		state.default_bookmark_id.clone()
	    } else {
		None
	    }
	};

	if let Some(bookmark_id) = default_bookmark_to_apply {
	    self.apply_bookmark(&bookmark_id);
	}

	let delete_target = {
	    let state = self.state.lock().unwrap();
	    if state.show_delete_operator_dialog {
		state.pending_delete_operator_id.clone()
	    } else {
		None
	    }
	};

	if let Some(operator_id) = delete_target {
	    egui::Window::new("Delete Operator")
		.collapsible(false)
		.resizable(false)
		.show(ctx, |ui| {
		    ui.label(format!("Delete operator \"{}\"?", operator_id));
		    ui.add_space(6.0);
		    ui.colored_label(
			egui::Color32::RED,
			"All operator settings, including bookmarks, will be lost.",
		    );

		    ui.add_space(10.0);

		    let mut cancel_requested = false;
		    let mut delete_requested = false;

		    ui.horizontal(|ui| {
			if ui.button("Cancel").clicked() {
			    cancel_requested = true;
			}

			if ui.button("Delete").clicked() {
			    delete_requested = true;
			}
		    });

		    if cancel_requested {
			if let Ok(mut state) = self.state.lock() {
			    state.show_delete_operator_dialog = false;
			    state.pending_delete_operator_id = None;
			    state.persistence_status.clear();
			}
		    }

		    if delete_requested {
			self.delete_operator(&operator_id);
		    }
		});
	}
    }

    pub(crate) fn draw_add_operator_dialog (
        &mut self,
        ctx: &egui::Context,
    ) {
	let show_add_operator_dialog = {
	    let state = self.state.lock().unwrap();
	    state.show_add_operator_dialog
	};

	if show_add_operator_dialog {
	    egui::Window::new("Add Operator")
		.collapsible(false)
		.resizable(false)
		.show(ctx, |ui| {
		    let mut save_requested = false;
		    let mut cancel_requested = false;

		    if let Ok(mut state) = self.state.lock() {
			ui.label("Operator ID / Call Sign:");
			ui.text_edit_singleline(&mut state.pending_operator_id);

			ui.add_space(8.0);
			ui.label("License:");

			use crate::ui::om_bands::LicenseClass;

			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::AmateurExtra),
			    "Amateur Extra",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::Advanced),
			    "Advanced",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::General),
			    "General",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    Some(LicenseClass::Technician),
			    "Technician",
			);
			ui.radio_value(
			    &mut state.pending_operator_license,
			    None,
			    "None",
			);

			if !state.persistence_status.is_empty() {
			    ui.add_space(8.0);
			    ui.colored_label(
				egui::Color32::YELLOW,
				&state.persistence_status,
			    );
			}

			ui.add_space(10.0);

			ui.horizontal(|ui| {
			    if ui.button("Cancel").clicked() {
				cancel_requested = true;
			    }

			    if ui.button("Save").clicked() {
				save_requested = true;
			    }
			});
		    }

		    if cancel_requested {
			if let Ok(mut state) = self.state.lock() {
			    state.show_add_operator_dialog = false;
			    state.pending_operator_id.clear();
			    state.pending_operator_license = None;
			    state.persistence_status.clear();
			}
		    }

		    if save_requested {
			self.save_pending_operator();
		    }
		});
	}
    }
}

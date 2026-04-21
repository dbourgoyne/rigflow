use super::app::RigflowApp;

use crate::ControlCommand;
use crate::UiState;
use crate::persistence::{normalize_operator_id,
			 operator_file_path,
			 apply_operator_settings_to_ui_state,
			 BookmarkFile,
			 BookmarkDisplaySettingsFile,
			 apply_ui_state_to_operator_settings,
};

impl RigflowApp {

    pub(crate) fn save_demod_preferences_to_current_operator(&mut self) {
	let snapshot = {
            let state = self.state.lock().unwrap();
            (
		state.operator_id.clone(),
		state.demod_preferences.clone(),
            )
	};

	let (operator_id, demod_preferences) = snapshot;

	log::info!(
            "save_demod_preferences_to_current_operator: operator='{}' prefs={:?}",
            operator_id,
            demod_preferences
	);

	if operator_id.trim().is_empty() {
            log::warn!("save_demod_preferences_to_current_operator: empty operator_id");
            return;
	}

	match self.persistence_store.load_or_create_operator_settings(&operator_id) {
            Ok(mut operator_settings) => {
		operator_settings.demod_preferences = demod_preferences;

		log::info!(
                    "about to save operator settings for '{}' with demod_preferences={:?}",
                    operator_settings.operator_id,
                    operator_settings.demod_preferences
		);

		if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to save demod preferences: {err}");
                    }
		} else if let Ok(mut state) = self.state.lock() {
                    state.persistence_status.clear();
		}
            }
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to load operator settings: {err}");
		}
            }
	}
    }


    pub(crate) fn update_selected_bookmark_notes(&mut self, notes: String) {
	let selected_id = {
            let state = self.state.lock().unwrap();
            state.selected_bookmark_id.clone()
	};

	let Some(selected_id) = selected_id else {
            return;
	};

	if let Ok(mut state) = self.state.lock() {
            if let Some(bookmark) = state.bookmarks.iter_mut().find(|b| b.id == selected_id) {
		let trimmed = notes.trim().to_string();
		bookmark.notes = if trimmed.is_empty() {
                    None
		} else {
                    Some(trimmed)
		};

		state.bookmark_status.clear();
            }
	}

	self.save_bookmarks_to_current_operator();
    }

    pub(crate) fn save_selected_operator_license(&mut self) {
	let (operator_id, selected_license) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.selected_license)
	};

	if operator_id.trim().is_empty() {
            return;
	}

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load operator: {err}");
                    }
                    return;
		}
            };

	operator_settings.selected_license = selected_license;

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save operator license: {err}");
            }
	}
    }

    pub(crate) fn delete_operator(&mut self, operator_id: &str) {
	use std::fs;

	let operator_id = match normalize_operator_id(operator_id) {
            Ok(id) => id,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("invalid operator id: {err}");
		}
		return;
            }
	};

	let path = operator_file_path(self.persistence_store.config_dir(), &operator_id);

	if let Err(err) = fs::remove_file(&path) {
            if err.kind() != std::io::ErrorKind::NotFound {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to delete operator file: {err}");
		}
		return;
            }
	}

	let mut app_state = match self.persistence_store.load_app_state() {
            Ok(app_state) => app_state,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to load app state: {err}");
		}
		return;
            }
	};

	app_state.known_operator_ids.retain(|id| id != &operator_id);
	let next_operator = app_state.known_operator_ids.first().cloned();
	app_state.last_operator_id = next_operator.clone();

	if let Err(err) = self.persistence_store.save_app_state(&app_state) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save app state: {err}");
            }
            return;
	}

	if let Ok(mut state) = self.state.lock() {
            state.show_delete_operator_dialog = false;
            state.pending_delete_operator_id = None;
	}

	if let Some(next_operator_id) = next_operator {
            match self.persistence_store.load_or_create_operator_settings(&next_operator_id) {
		Ok(operator_settings) => {
                    if let Ok(mut state) = self.state.lock() {
			apply_operator_settings_to_ui_state(
                            &mut state,
                            &operator_settings,
                            &app_state,
			);
			state.persistence_status.clear();
                    }
		}
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load replacement operator: {err}");
                    }
		}
            }
	} else if let Ok(mut state) = self.state.lock() {
            let mut new_state = UiState::default();
            new_state.known_operator_ids = Vec::new();
            new_state.persistence_status.clear();
            *state = new_state;
	}
    }
    
    pub(crate) fn delete_selected_bookmark(&mut self) {
	let selected_id = {
            let state = self.state.lock().unwrap();
            state.selected_bookmark_id.clone()
	};

	let Some(selected_id) = selected_id else {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status = "no bookmark selected".to_string();
            }
            return;
	};

	if let Ok(mut state) = self.state.lock() {
            let before_len = state.bookmarks.len();
            state.bookmarks.retain(|bookmark| bookmark.id != selected_id);

            if state.bookmarks.len() == before_len {
		state.bookmark_status = "bookmark not found".to_string();
		return;
            }

            if state
		.default_bookmark_id
		.as_ref()
		.map(|id| id == &selected_id)
		.unwrap_or(false)
            {
		state.default_bookmark_id = None;
            }

            state.selected_bookmark_id = None;
            state.bookmark_status.clear();
	}

	self.save_bookmarks_to_current_operator();
    }

    pub(crate) fn set_default_bookmark(&mut self, bookmark_id: &str) {
	if let Ok(mut state) = self.state.lock() {
            state.default_bookmark_id = Some(bookmark_id.to_string());
            state.bookmark_status.clear();
	}

	self.save_bookmarks_to_current_operator();
    }

    pub(crate) fn apply_bookmark(&mut self, bookmark_id: &str) {
	let bookmark = {
            let state = self.state.lock().unwrap();
            state
		.bookmarks
		.iter()
		.find(|b| b.id == bookmark_id)
		.cloned()
	};

	let Some(bookmark) = bookmark else {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status = "bookmark not found".to_string();
            }
            return;
	};

	{

	    let center_freq_hz = bookmark.frequency_hz;

	    {
		let mut state = self.state.lock().unwrap();

		state.center_freq_hz = center_freq_hz;
		state.target_freq_hz = bookmark.frequency_hz;
		state.demod_mode = bookmark.demod_mode;

		if let Some(sideband) = bookmark.sideband {
		    state.sideband = sideband;
		}

		if let Some(display) = &bookmark.display {
		    if let Some(zoom) = display.zoom {
			state.display_zoom = zoom;
		    }
		    if let Some(adaptive) = display.adaptive_waterfall_normalization {
			state.adaptive_waterfall_normalization = adaptive;
		    }
		    if let Some(top_db) = display.waterfall_top_db {
			state.display_top_db = top_db;
		    }
		    if let Some(range_db) = display.waterfall_range_db {
			state.display_range_db = range_db;
		    }
		}

		state.selected_bookmark_id = Some(bookmark.id.clone());
		state.bookmark_status.clear();
	    }

	    let _ = self.ws_cmd_tx.send(
		ControlCommand::LegacyClientMessage(
		    rigflow_protocol::ClientMessage::SetCenterFrequency {
			center_freq_hz,
		    },
		),
	    );

	    let _ = self.ws_cmd_tx.send(
		ControlCommand::LegacyClientMessage(
		    rigflow_protocol::ClientMessage::SetFrequency {
			target_freq_hz: bookmark.frequency_hz,
		    },
		),
	    );

	    let _ = self.ws_cmd_tx.send(
		ControlCommand::LegacyClientMessage(
		    rigflow_protocol::ClientMessage::SetDemodMode {
			mode: bookmark.demod_mode,
		    },
		),
	    );

	    if let Some(sideband) = bookmark.sideband {
		let _ = self.ws_cmd_tx.send(
		    ControlCommand::LegacyClientMessage(
			rigflow_protocol::ClientMessage::SetSideband { sideband },
		    ),
		);
	    }
	}
    }

    pub(crate) fn save_current_as_bookmark(&mut self) {
	let (
	    name,
	    notes,
	    target_freq_hz,
	    demod_mode,
	    sideband,
	    zoom,
	    adaptive_waterfall_normalization,
	    display_top_db,
	    display_range_db,
	    existing_ids,
	) = {
            let state = self.state.lock().unwrap();

	    (
		state.pending_bookmark_name.trim().to_string(),
		state.pending_bookmark_notes.trim().to_string(),
		state.target_freq_hz,
		state.demod_mode,
		state.sideband,
		state.display_zoom,
		state.adaptive_waterfall_normalization,
		state.display_top_db,
		state.display_range_db,
		state
		    .bookmarks
		    .iter()
		    .map(|b| b.id.clone())
		    .collect::<Vec<_>>(),
	    )
	};

	if name.is_empty() {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status = "bookmark name cannot be empty".to_string();
            }
            return;
	}

	let mut bookmark_id = Self::make_bookmark_id(&name);
	if existing_ids.iter().any(|id| id == &bookmark_id) {
            let mut suffix = 2;
            while existing_ids.iter().any(|id| id == &format!("{bookmark_id}-{suffix}")) {
		suffix += 1;
            }
            bookmark_id = format!("{bookmark_id}-{suffix}");
	}

	let bookmark = BookmarkFile {
	    id: bookmark_id.clone(),
	    name,
	    frequency_hz: target_freq_hz,
	    demod_mode,
	    sideband: Some(sideband),
	    display: Some(BookmarkDisplaySettingsFile {
		zoom: Some(zoom),
		adaptive_waterfall_normalization: Some(
		    adaptive_waterfall_normalization,
		),
		waterfall_top_db: Some(display_top_db),
		waterfall_range_db: Some(display_range_db),
	    }),
	    notes: if notes.is_empty() { None } else { Some(notes) },
	};

	if let Ok(mut state) = self.state.lock() {
	    state.bookmarks.push(bookmark);
	    state.selected_bookmark_id = Some(bookmark_id);
	    state.show_add_bookmark_dialog = false;
	    state.pending_bookmark_name.clear();
	    state.pending_bookmark_notes.clear();
	    state.bookmark_status.clear();
	}

	self.save_bookmarks_to_current_operator();
    }

    fn make_bookmark_id(name: &str) -> String {
	let mut id = String::new();

	for ch in name.trim().chars() {
            if ch.is_ascii_alphanumeric() {
		id.push(ch.to_ascii_lowercase());
            } else if ch == ' ' || ch == '-' || ch == '_' {
		if !id.ends_with('-') {
                    id.push('-');
		}
            }
	}

	let id = id.trim_matches('-').to_string();

	if id.is_empty() {
            "bookmark".to_string()
	} else {
            id
	}
    }

    pub(crate) fn save_bookmarks_to_current_operator(&mut self) {
	let operator_id = {
            let state = self.state.lock().unwrap();
            state.operator_id.clone()
	};

	if operator_id.trim().is_empty() {
            return;
	}

	let operator_id = match normalize_operator_id(&operator_id) {
            Ok(id) => id,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.bookmark_status = format!("invalid operator id: {err}");
		}
		return;
            }
	};

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.bookmark_status =
                            format!("failed to load operator settings: {err}");
                    }
                    return;
		}
            };

	{
            let state = self.state.lock().unwrap();
            apply_ui_state_to_operator_settings(&state, &mut operator_settings);
	}

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.bookmark_status =
                    format!("failed to save operator settings: {err}");
            }
	}
    }

    pub(crate) fn save_server_ip(&mut self) {
	let (operator_id, server_ip) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.rigflow_server_ip.clone())
	};

	if operator_id.trim().is_empty() {
            return;
	}

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load operator: {err}");
                    }
                    return;
		}
            };

	operator_settings.server_ip = server_ip;

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save server IP: {err}");
            }
	}
    }

    pub(crate) fn save_pending_operator(&mut self) {
	let (raw_operator_id, selected_license) = {
            let state = self.state.lock().unwrap();
            (
		state.pending_operator_id.clone(),
		state.pending_operator_license,
            )
	};

	let operator_id = match normalize_operator_id(&raw_operator_id) {
            Ok(id) => id,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("invalid operator id: {err}");
		}
		return;
            }
	};

	let mut operator_settings =
            match self.persistence_store.load_or_create_operator_settings(&operator_id) {
		Ok(settings) => settings,
		Err(err) => {
                    if let Ok(mut state) = self.state.lock() {
			state.persistence_status =
                            format!("failed to load/create operator: {err}");
                    }
                    return;
		}
            };

	operator_settings.selected_license = selected_license;

	if let Err(err) = self.persistence_store.save_operator_settings(&operator_settings) {
            if let Ok(mut state) = self.state.lock() {
		state.persistence_status =
                    format!("failed to save operator settings: {err}");
            }
            return;
	}

	let app_state = match self.persistence_store.upsert_known_operator(&operator_id) {
            Ok(app_state) => app_state,
            Err(err) => {
		if let Ok(mut state) = self.state.lock() {
                    state.persistence_status =
			format!("failed to update known operators: {err}");
		}
		return;
            }
	};

	if let Ok(mut state) = self.state.lock() {
            apply_operator_settings_to_ui_state(
		&mut state,
		&operator_settings,
		&app_state,
            );

            state.show_add_operator_dialog = false;
            state.pending_operator_id.clear();
            state.pending_operator_license = None;
            state.persistence_status.clear();
	}
    }

}

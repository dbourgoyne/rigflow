use std::time::{Duration, Instant};

use super::app::RigflowApp;

use crate::ControlCommand;
use crate::UiState;
use crate::persistence::models::{RadioSettingsFile, WaterfallDisplayPreferencesFile};
use crate::persistence::{
    BookmarkDisplaySettingsFile, BookmarkFile, apply_operator_settings_to_ui_state,
    apply_ui_state_to_operator_settings, normalize_operator_id, operator_file_path,
};

/// Build a `RadioSettingsFile` snapshot from the current UI state (the per-radio
/// operating bundle: frequency, mode, filters, squelch/NR2/AGC, volume, CW
/// sidetone/hang, waterfall).  Shared by the immediate save and the diff autosave.
pub(crate) fn radio_settings_from_ui(state: &UiState) -> RadioSettingsFile {
    RadioSettingsFile {
        center_freq_hz: state.center_freq_hz,
        target_freq_hz: state.target_freq_hz,
        demod_mode: state.demod_mode,
        sideband: state.sideband,
        demod_preferences: state.demod_preferences.clone(),
        waterfall_display_preferences: WaterfallDisplayPreferencesFile {
            display_zoom: state.display_zoom,
            adaptive_waterfall_normalization: state.adaptive_waterfall_normalization,
            manual_waterfall_top_db: state.manual_waterfall_top_db,
            manual_waterfall_range_db: state.manual_waterfall_range_db,
            waterfall_frame_rate_hz: state.waterfall_frame_rate_hz,
            waterfall_smoothing: state.waterfall_smoothing,
        },
        volume_percent: state.volume_percent,
        cw_sidetone_volume: state.cw_sidetone_volume,
        cw_hang_ms: state.cw_hang_ms,
        squelch_enabled: state.squelch_enabled,
        squelch_threshold_db: state.squelch_threshold_db,
        nr2_enabled: state.nr2_enabled,
        nr2_strength: state.nr2_strength,
        nb_enabled: state.nb_enabled,
        nb_threshold: state.nb_threshold,
        notch_auto_enabled: state.notch_auto_enabled,
        agc_enabled: state.agc_enabled,
        agc_strength: state.agc_strength,
        tx_limiter_enabled: state.tx_limiter_enabled,
        tx_limiter_threshold_percent: state.tx_limiter_threshold_percent,
        compressor_enabled: state.compressor_enabled,
        compressor_level: state.compressor_level,
        cw_decode_enabled: state.cw_decode.enabled(),
    }
}

impl RigflowApp {
    /// Persist the current `source_control` state for the active radio into the
    /// operator's settings file.
    ///
    /// This is a no-op when no operator is active or no radio is acquired.
    pub(crate) fn save_source_control_prefs_to_current_operator(&mut self) {
        let (operator_id, radio_id, source_control) = {
            let state = self.state.lock().unwrap();
            (
                state.operator_id.clone(),
                state.selected_radio_id.clone(),
                state.source_control.clone(),
            )
        };

        let Some(radio_id) = radio_id else {
            return;
        };
        if operator_id.trim().is_empty() {
            return;
        }

        // Update the in-memory mirror so subsequent acquire-apply uses the
        // latest value even before the file is re-read.
        if let Ok(mut state) = self.state.lock() {
            state
                .source_control_preferences
                .insert(radio_id.clone(), source_control.clone());
        }

        // Load-modify-save: update only the source_control_preferences entry
        // so no other operator data is lost.
        let mut settings = match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(s) => s,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator settings: {err}");
                }
                return;
            }
        };

        settings
            .source_control_preferences
            .insert(radio_id, source_control);

        if let Err(err) = self.persistence_store.save_operator_settings(&settings) {
            if let Ok(mut state) = self.state.lock() {
                state.persistence_status = format!("failed to save source control prefs: {err}");
            }
        }
    }

    /// Debounced per-radio settings autosave (called every frame).  Diffs the
    /// live per-radio bundle against the saved one and persists ~600 ms after it
    /// stops changing — so it captures every change path (frequency/band tuning,
    /// any control) without slider drags thrashing the file.
    pub(crate) fn autosave_radio_settings(&mut self) {
        let id_and_current = {
            let state = self.state.lock().unwrap();
            if state.radio_acquired {
                state
                    .selected_radio_id
                    .clone()
                    .filter(|s| !s.is_empty())
                    .map(|id| (id, radio_settings_from_ui(&state)))
            } else {
                None
            }
        };

        let Some((id, current)) = id_and_current else {
            self.radio_settings_last = None;
            self.radio_settings_stable_since = None;
            return;
        };

        // Still changing? Reset the debounce window and wait.
        if self.radio_settings_last.as_ref() != Some(&current) {
            self.radio_settings_last = Some(current);
            self.radio_settings_stable_since = Some(Instant::now());
            return;
        }

        // Stable — once the window elapses, save if it differs from what's stored.
        if let Some(since) = self.radio_settings_stable_since {
            if since.elapsed() >= Duration::from_millis(600) {
                self.radio_settings_stable_since = None;
                let differs = {
                    let state = self.state.lock().unwrap();
                    state.radio_settings.get(&id) != Some(&current)
                };
                if differs {
                    self.save_radio_settings_for_current_radio();
                }
            }
        }
    }

    /// Persist the current per-radio operating state (Radio Control + Waterfall)
    /// into `radio_settings[<acquired radio id>]`, so the operator resumes this
    /// radio exactly where they left off.  Scoped per (operator, radio) since it
    /// lives in the per-operator file.  When no radio is acquired, falls back to
    /// the operator-level defaults (which seed a radio's first acquire).
    pub(crate) fn save_radio_settings_for_current_radio(&mut self) {
        let (operator_id, radio_id_opt) = {
            let state = self.state.lock().unwrap();
            let id = if state.radio_acquired {
                state.selected_radio_id.clone().filter(|s| !s.is_empty())
            } else {
                None
            };
            (state.operator_id.clone(), id)
        };

        if operator_id.trim().is_empty() {
            return;
        }

        let radio_id = match radio_id_opt {
            Some(id) => id,
            None => {
                // No acquired radio — persist operator-level defaults instead.
                self.save_demod_preferences_to_current_operator();
                self.save_volume_to_current_operator();
                self.save_waterfall_display_preferences_to_current_operator();
                return;
            }
        };

        let bundle = {
            let state = self.state.lock().unwrap();
            radio_settings_from_ui(&state)
        };

        // Update the in-memory mirror so a re-acquire uses the latest value.
        if let Ok(mut state) = self.state.lock() {
            state
                .radio_settings
                .insert(radio_id.clone(), bundle.clone());
        }

        // Load-modify-save the operator file's radio_settings entry only.
        match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(mut settings) => {
                settings.radio_settings.insert(radio_id, bundle);
                if let Err(err) = self.persistence_store.save_operator_settings(&settings) {
                    if let Ok(mut state) = self.state.lock() {
                        state.persistence_status = format!("failed to save radio settings: {err}");
                    }
                }
            }
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator settings: {err}");
                }
            }
        }
    }
}

impl RigflowApp {
    pub(crate) fn save_waterfall_display_preferences_to_current_operator(&mut self) {
        let snapshot = {
            let state = self.state.lock().unwrap();
            (
                state.operator_id.clone(),
                state.display_zoom,
                state.adaptive_waterfall_normalization,
                state.manual_waterfall_top_db,
                state.manual_waterfall_range_db,
                state.waterfall_frame_rate_hz,
                state.waterfall_smoothing,
            )
        };

        let (
            operator_id,
            display_zoom,
            adaptive_waterfall_normalization,
            manual_top_db,
            manual_range_db,
            waterfall_frame_rate_hz,
            waterfall_smoothing,
        ) = snapshot;

        if operator_id.trim().is_empty() {
            return;
        }

        match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(mut operator_settings) => {
                operator_settings.waterfall_display_preferences.display_zoom = display_zoom;

                operator_settings
                    .waterfall_display_preferences
                    .adaptive_waterfall_normalization = adaptive_waterfall_normalization;

                operator_settings
                    .waterfall_display_preferences
                    .manual_waterfall_top_db = manual_top_db;

                operator_settings
                    .waterfall_display_preferences
                    .manual_waterfall_range_db = manual_range_db;

                operator_settings
                    .waterfall_display_preferences
                    .waterfall_frame_rate_hz = waterfall_frame_rate_hz;

                operator_settings
                    .waterfall_display_preferences
                    .waterfall_smoothing = waterfall_smoothing;

                if let Err(err) = self
                    .persistence_store
                    .save_operator_settings(&operator_settings)
                {
                    if let Ok(mut state) = self.state.lock() {
                        state.persistence_status = format!("failed to save waterfall prefs: {err}");
                    }
                } else if let Ok(mut state) = self.state.lock() {
                    state.persistence_status.clear();
                }
            }

            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator settings: {err}");
                }
            }
        }
    }

    /// Persist the per-mode grid-snap / tuning-step sizes for the current operator.
    pub(crate) fn save_tuning_step_preferences_to_current_operator(&mut self) {
        let (operator_id, tuning_step_preferences) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.tuning_step_preferences)
        };

        if operator_id.trim().is_empty() {
            return;
        }

        match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(mut operator_settings) => {
                operator_settings.tuning_step_preferences = tuning_step_preferences;
                if let Err(err) = self
                    .persistence_store
                    .save_operator_settings(&operator_settings)
                {
                    if let Ok(mut state) = self.state.lock() {
                        state.persistence_status = format!("failed to save tuning step: {err}");
                    }
                } else if let Ok(mut state) = self.state.lock() {
                    state.persistence_status.clear();
                }
            }
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator settings: {err}");
                }
            }
        }
    }

    /// Persist the receive-audio volume (%) for the current operator.
    pub(crate) fn save_volume_to_current_operator(&mut self) {
        let (operator_id, volume_percent, volume_percent_b) = {
            let state = self.state.lock().unwrap();
            (
                state.operator_id.clone(),
                state.volume_percent,
                state.volume_percent_b,
            )
        };

        if operator_id.trim().is_empty() {
            return;
        }

        match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(mut operator_settings) => {
                operator_settings.volume_percent = volume_percent;
                operator_settings.volume_percent_b = volume_percent_b;

                if let Err(err) = self
                    .persistence_store
                    .save_operator_settings(&operator_settings)
                {
                    if let Ok(mut state) = self.state.lock() {
                        state.persistence_status = format!("failed to save volume: {err}");
                    }
                } else if let Ok(mut state) = self.state.lock() {
                    state.persistence_status.clear();
                }
            }

            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator settings: {err}");
                }
            }
        }
    }

    /// Persist the global settings-lock toggle to the current operator.
    pub(crate) fn save_config_lock_to_current_operator(&mut self) {
        let (operator_id, config_locked) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.config_locked)
        };
        if operator_id.trim().is_empty() {
            return;
        }
        if let Ok(mut op) = self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            op.config_locked = config_locked;
            let _ = self.persistence_store.save_operator_settings(&op);
        }
    }

    /// Persist the "Show advanced & diagnostics" toggle to the current operator.
    pub(crate) fn save_show_advanced_to_current_operator(&mut self) {
        let (operator_id, show_advanced) = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.show_advanced)
        };

        if operator_id.trim().is_empty() {
            return;
        }

        match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(mut operator_settings) => {
                operator_settings.show_advanced = show_advanced;

                if let Err(err) = self
                    .persistence_store
                    .save_operator_settings(&operator_settings)
                {
                    if let Ok(mut state) = self.state.lock() {
                        state.persistence_status = format!("failed to save advanced toggle: {err}");
                    }
                } else if let Ok(mut state) = self.state.lock() {
                    state.persistence_status.clear();
                }
            }

            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator settings: {err}");
                }
            }
        }
    }

    /// Persist the Text-to-CW message, speed, and memory macros for the current
    /// operator.
    pub(crate) fn save_cw_message_to_current_operator(&mut self) {
        let (operator_id, cw_message, cw_speed_wpm, cw_macros) = {
            let state = self.state.lock().unwrap();
            (
                state.operator_id.clone(),
                state.cw_message.clone(),
                state.cw_speed_wpm,
                state
                    .cw_macros
                    .iter()
                    .map(|m| crate::persistence::models::CwMacroFile {
                        label: m.label.clone(),
                        text: m.text.clone(),
                    })
                    .collect::<Vec<_>>(),
            )
        };

        if operator_id.trim().is_empty() {
            return;
        }

        if let Ok(mut operator_settings) = self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            operator_settings.cw_message = cw_message;
            operator_settings.cw_speed_wpm = cw_speed_wpm;
            operator_settings.cw_macros = cw_macros;
            if let Err(err) = self
                .persistence_store
                .save_operator_settings(&operator_settings)
            {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to save CW settings: {err}");
                }
            }
        }
    }

    /// Persist the microphone device + gain for the current operator.
    pub(crate) fn save_mic_settings_to_current_operator(&mut self) {
        let (operator_id, mic_device, mic_gain_percent) = {
            let state = self.state.lock().unwrap();
            (
                state.operator_id.clone(),
                state.mic_device.clone(),
                state.mic_gain_percent,
            )
        };

        if operator_id.trim().is_empty() {
            return;
        }

        if let Ok(mut operator_settings) = self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            operator_settings.mic_device = mic_device;
            operator_settings.mic_gain_percent = mic_gain_percent;
            if let Err(err) = self
                .persistence_store
                .save_operator_settings(&operator_settings)
            {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to save mic settings: {err}");
                }
            }
        }
    }

    pub(crate) fn save_demod_preferences_to_current_operator(&mut self) {
        let snapshot = {
            let state = self.state.lock().unwrap();
            (state.operator_id.clone(), state.demod_preferences.clone())
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

        match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(mut operator_settings) => {
                operator_settings.demod_preferences = demod_preferences;

                log::info!(
                    "about to save operator settings for '{}' with demod_preferences={:?}",
                    operator_settings.operator_id,
                    operator_settings.demod_preferences
                );

                if let Err(err) = self
                    .persistence_store
                    .save_operator_settings(&operator_settings)
                {
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
                    state.persistence_status = format!("failed to load operator settings: {err}");
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

        let mut operator_settings = match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(settings) => settings,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator: {err}");
                }
                return;
            }
        };

        operator_settings.selected_license = selected_license;

        if let Err(err) = self
            .persistence_store
            .save_operator_settings(&operator_settings)
        {
            if let Ok(mut state) = self.state.lock() {
                state.persistence_status = format!("failed to save operator license: {err}");
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
                    state.persistence_status = format!("failed to delete operator file: {err}");
                }
                return;
            }
        }

        let mut app_state = match self.persistence_store.load_app_state() {
            Ok(app_state) => app_state,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load app state: {err}");
                }
                return;
            }
        };

        app_state.known_operator_ids.retain(|id| id != &operator_id);
        let next_operator = app_state.known_operator_ids.first().cloned();
        app_state.last_operator_id = next_operator.clone();

        if let Err(err) = self.persistence_store.save_app_state(&app_state) {
            if let Ok(mut state) = self.state.lock() {
                state.persistence_status = format!("failed to save app state: {err}");
            }
            return;
        }

        if let Ok(mut state) = self.state.lock() {
            state.show_delete_operator_dialog = false;
            state.pending_delete_operator_id = None;
        }

        if let Some(next_operator_id) = next_operator {
            match self
                .persistence_store
                .load_or_create_operator_settings(&next_operator_id)
            {
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
            state
                .bookmarks
                .retain(|bookmark| bookmark.id != selected_id);

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
        let (bookmark, rejection) = {
            let state = self.state.lock().unwrap();
            let bookmark = state
                .bookmarks
                .iter()
                .find(|b| b.id == bookmark_id)
                .cloned();
            let rejection = bookmark.as_ref().and_then(|b| {
                crate::ui::freq_limits::bookmark_rejection_message(b.frequency_hz, &state)
            });
            (bookmark, rejection)
        };

        let Some(bookmark) = bookmark else {
            if let Ok(mut state) = self.state.lock() {
                state.bookmark_status = "bookmark not found".to_string();
            }
            return;
        };

        if let Some(msg) = rejection {
            if let Ok(mut state) = self.state.lock() {
                state.bookmark_status = msg.to_string();
            }
            return;
        }

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
                        state.manual_waterfall_top_db = top_db;
                    }
                    if let Some(range_db) = display.waterfall_range_db {
                        state.manual_waterfall_range_db = range_db;
                    }
                }

                state.selected_bookmark_id = Some(bookmark.id.clone());
                state.bookmark_status.clear();
            }

            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                rigflow_protocol::ClientRadioMessage::SetCenterFrequency {
                    center_freq_hz: bookmark.frequency_hz as u64,
                },
            ));

            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                rigflow_protocol::ClientRadioMessage::SetTargetFrequency {
                    target_freq_hz: bookmark.frequency_hz as u64,
                },
            ));

            let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                rigflow_protocol::ClientRadioMessage::SetDemodMode {
                    mode: bookmark.demod_mode,
                },
            ));

            if let Some(sideband) = bookmark.sideband {
                let _ = self.ws_cmd_tx.send(ControlCommand::RadioMessage(
                    rigflow_protocol::ClientRadioMessage::SetSideband { sideband },
                ));
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
            manual_waterfall_top_db,
            manual_waterfall_range_db,
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
                state.manual_waterfall_top_db,
                state.manual_waterfall_range_db,
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
            while existing_ids
                .iter()
                .any(|id| id == &format!("{bookmark_id}-{suffix}"))
            {
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
                adaptive_waterfall_normalization: Some(adaptive_waterfall_normalization),
                waterfall_top_db: Some(manual_waterfall_top_db),
                waterfall_range_db: Some(manual_waterfall_range_db),
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

        let mut operator_settings = match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(settings) => settings,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.bookmark_status = format!("failed to load operator settings: {err}");
                }
                return;
            }
        };

        {
            let state = self.state.lock().unwrap();
            apply_ui_state_to_operator_settings(&state, &mut operator_settings);
        }

        if let Err(err) = self
            .persistence_store
            .save_operator_settings(&operator_settings)
        {
            if let Ok(mut state) = self.state.lock() {
                state.bookmark_status = format!("failed to save operator settings: {err}");
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

        let mut operator_settings = match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(settings) => settings,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load operator: {err}");
                }
                return;
            }
        };

        operator_settings.server_ip = server_ip;

        if let Err(err) = self
            .persistence_store
            .save_operator_settings(&operator_settings)
        {
            if let Ok(mut state) = self.state.lock() {
                state.persistence_status = format!("failed to save server IP: {err}");
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

        let mut operator_settings = match self
            .persistence_store
            .load_or_create_operator_settings(&operator_id)
        {
            Ok(settings) => settings,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to load/create operator: {err}");
                }
                return;
            }
        };

        operator_settings.selected_license = selected_license;

        if let Err(err) = self
            .persistence_store
            .save_operator_settings(&operator_settings)
        {
            if let Ok(mut state) = self.state.lock() {
                state.persistence_status = format!("failed to save operator settings: {err}");
            }
            return;
        }

        let app_state = match self.persistence_store.upsert_known_operator(&operator_id) {
            Ok(app_state) => app_state,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.persistence_status = format!("failed to update known operators: {err}");
                }
                return;
            }
        };

        if let Ok(mut state) = self.state.lock() {
            apply_operator_settings_to_ui_state(&mut state, &operator_settings, &app_state);

            state.show_add_operator_dialog = false;
            state.pending_operator_id.clear();
            state.pending_operator_license = None;
            state.persistence_status.clear();
        }
    }
}

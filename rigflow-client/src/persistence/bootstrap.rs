use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::{
    persistence::{
        error::PersistenceError,
        models::{AppStateFile, OperatorSettingsFile},
        paths::{app_state_path, operator_file_path, resolve_config_dir},
        store::PersistenceStore,
    },
    ui::state::UiState,
};

#[derive(Debug)]
pub enum StartupError {
    ConfigDirectory {
        source: PersistenceError,
    },
    ConfigLayout {
        path: PathBuf,
        source: PersistenceError,
    },
    AppState {
        path: PathBuf,
        source: PersistenceError,
    },
    OperatorSettings {
        operator_id: String,
        path: PathBuf,
        source: PersistenceError,
    },
}

impl fmt::Display for StartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigDirectory { source } => {
                write!(
                    f,
                    "could not determine the client config directory: {source}"
                )
            }
            Self::ConfigLayout { path, source } => write!(
                f,
                "could not prepare the client config directory '{}': {source}",
                path.display()
            ),
            Self::AppState { path, source } => write!(
                f,
                "could not load app settings from '{}': {source}",
                path.display()
            ),
            Self::OperatorSettings {
                operator_id,
                path,
                source,
            } => write!(
                f,
                "could not load settings for operator '{operator_id}' from '{}': {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StartupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::ConfigDirectory { source }
            | Self::ConfigLayout { source, .. }
            | Self::AppState { source, .. }
            | Self::OperatorSettings { source, .. } => source,
        })
    }
}

#[derive(Debug)]
pub struct InitialClientState {
    pub ui_state: UiState,
    pub persistence_store: PersistenceStore,
}

/// Initialize the UI state and persistence store as one all-or-nothing result.
///
/// Behavior:
/// - resolves the config directory
/// - creates the persistence store
/// - loads the global app state
/// - starts from `UiState::default()`
/// - populates known operators
/// - if a last operator exists, loads and applies that operator's settings
///
/// No partially-loaded state or fallback store is returned: any unrecovered
/// error aborts initialization before the rest of the client starts.
pub fn initialize_client_state(
    cli_config_dir: Option<&Path>,
) -> Result<InitialClientState, StartupError> {
    let config_dir = resolve_config_dir(cli_config_dir)
        .map_err(|source| StartupError::ConfigDirectory { source })?;
    let store = PersistenceStore::new(config_dir.clone());

    store
        .ensure_layout()
        .map_err(|source| StartupError::ConfigLayout {
            path: config_dir.clone(),
            source,
        })?;

    let app_state = store
        .load_app_state()
        .map_err(|source| StartupError::AppState {
            path: app_state_path(&config_dir),
            source,
        })?;

    let mut ui_state = UiState::default();
    apply_app_state_to_ui_state(&mut ui_state, &app_state);

    if let Some(operator_id) = app_state.last_operator_id.as_deref() {
        let operator_settings = store
            .load_or_create_operator_settings(operator_id)
            .map_err(|source| StartupError::OperatorSettings {
                operator_id: operator_id.to_string(),
                path: operator_file_path(&config_dir, operator_id),
                source,
            })?;
        apply_operator_settings_to_ui_state(&mut ui_state, &operator_settings, &app_state);
    }

    // Surface any corrupt-config recovery that happened during the loads above,
    // so the user sees it in the Problems area instead of a silent reset.
    let notices = store.take_recovery_notices();
    if !notices.is_empty() {
        ui_state.persistence_status = notices.join("; ");
    }

    Ok(InitialClientState {
        ui_state,
        persistence_store: store,
    })
}

/// Apply global app state to runtime UI state.
///
/// This should only copy fields that are truly global, such as:
/// - known operators
///
/// It should not assume that a last operator exists.
pub fn apply_app_state_to_ui_state(state: &mut UiState, app_state: &AppStateFile) {
    state.known_operator_ids = app_state.known_operator_ids.clone();
    // Global station location (shared across operators).
    state.station_profile = app_state.station.clone();
}

/// Apply persisted operator settings to runtime UI state.
///
/// This should copy only fields that are intentionally persistent and
/// already supported by your runtime UI state.
pub fn apply_operator_settings_to_ui_state(
    state: &mut UiState,
    operator: &OperatorSettingsFile,
    app_state: &AppStateFile,
) {
    state.operator_id = operator.operator_id.clone();
    state.operator_name = operator.name.clone();
    state.known_operator_ids = app_state.known_operator_ids.clone();

    state.selected_license = operator.selected_license;
    state.rigflow_server_ip = operator.server_ip.clone();

    state.default_bookmark_id = operator.default_bookmark_id.clone();
    state.auto_apply_default_bookmark_on_acquire = operator.auto_apply_default_bookmark_on_acquire;

    state.bookmarks = operator.bookmarks.clone();

    state.display_zoom = operator.waterfall_display_preferences.display_zoom;
    state.adaptive_waterfall_normalization = operator
        .waterfall_display_preferences
        .adaptive_waterfall_normalization;
    state.manual_waterfall_top_db = operator
        .waterfall_display_preferences
        .manual_waterfall_top_db;
    state.manual_waterfall_range_db = operator
        .waterfall_display_preferences
        .manual_waterfall_range_db;
    state.waterfall_frame_rate_hz = operator
        .waterfall_display_preferences
        .waterfall_frame_rate_hz;
    state.waterfall_smoothing = operator.waterfall_display_preferences.waterfall_smoothing;
    // VFO B's session smoothing seeds from VFO A's persisted value.
    state.vfo_b_waterfall_smoothing = state.waterfall_smoothing;

    // Keep selection stable if possible, otherwise clear it.
    let selected_still_exists = state
        .selected_bookmark_id
        .as_ref()
        .map(|selected_id| {
            state
                .bookmarks
                .iter()
                .any(|bookmark| &bookmark.id == selected_id)
        })
        .unwrap_or(false);

    if !selected_still_exists {
        state.selected_bookmark_id = state.default_bookmark_id.clone();
    }

    state.bookmark_status.clear();

    // --- NEW: load per-demod preferences ---
    state.demod_preferences = operator.demod_preferences.clone();
    state.tuning_step_preferences = operator.tuning_step_preferences;

    let prefs = state.demod_preferences.get(state.demod_mode);

    state.filter_bandwidth_hz = prefs.filter_bandwidth_hz;
    state.pitch_hz = prefs.pitch_hz;
    state.deemphasis_mode = prefs.deemphasis_mode;

    state.filter_bw_debounce = crate::ui::state::DebounceState::new(state.filter_bandwidth_hz);
    state.pitch_debounce = crate::ui::state::DebounceState::new(state.pitch_hz);

    state.last_demod_mode_for_controls = Some(state.demod_mode);

    // Mirror the per-radio source-control preferences into UiState so the
    // WebSocket handler can apply them on radio acquire without needing to
    // touch the persistence store.
    state.source_control_preferences = operator.source_control_preferences.clone();
    state.radio_settings = operator.radio_settings.clone();

    state.volume_percent = operator.volume_percent;
    state.volume_percent_b = operator.volume_percent_b;
    state.show_advanced = operator.show_advanced;
    state.config_locked = operator.config_locked;
    state.band_memory = operator.band_memory.clone();

    // Text-to-CW: restore the last-used message and speed.
    state.cw_message = operator.cw_message.clone();
    state.cw_speed_wpm = operator.cw_speed_wpm;

    // CW macros: copy up to 4 persisted slots over the defaults (a short or
    // missing list keeps the stock defaults for the remaining slots).
    for (i, m) in operator.cw_macros.iter().take(4).enumerate() {
        state.cw_macros[i].label = m.label.clone();
        state.cw_macros[i].text = m.text.clone();
    }

    // Microphone: restore selected device + gain.
    state.mic_device = operator.mic_device.clone();
    state.mic_gain_percent = operator.mic_gain_percent;

    // Voice keyer: restore the last selected clip filename.
    state.voice_keyer_clip = operator.voice_keyer_clip.clone();
}

pub fn apply_ui_state_to_operator_settings(state: &UiState, operator: &mut OperatorSettingsFile) {
    operator.operator_id = state.operator_id.clone();
    operator.name = state.operator_name.clone();
    operator.selected_license = state.selected_license;
    operator.server_ip = state.rigflow_server_ip.clone();

    operator.default_bookmark_id = state.default_bookmark_id.clone();
    operator.auto_apply_default_bookmark_on_acquire = state.auto_apply_default_bookmark_on_acquire;

    operator.bookmarks = state.bookmarks.clone();

    // --- NEW: persist per-demod preferences ---
    operator.demod_preferences = state.demod_preferences.clone();
    operator.tuning_step_preferences = state.tuning_step_preferences;

    operator.waterfall_display_preferences.display_zoom = state.display_zoom;
    operator
        .waterfall_display_preferences
        .adaptive_waterfall_normalization = state.adaptive_waterfall_normalization;
    operator
        .waterfall_display_preferences
        .manual_waterfall_top_db = state.manual_waterfall_top_db;
    operator
        .waterfall_display_preferences
        .manual_waterfall_range_db = state.manual_waterfall_range_db;

    // Write the current per-radio source-control preferences back to the file.
    operator.source_control_preferences = state.source_control_preferences.clone();
    operator.radio_settings = state.radio_settings.clone();

    operator.volume_percent = state.volume_percent;
    operator.volume_percent_b = state.volume_percent_b;
    operator.show_advanced = state.show_advanced;
    operator.config_locked = state.config_locked;
    operator.band_memory = state.band_memory.clone();

    operator.cw_message = state.cw_message.clone();
    operator.cw_speed_wpm = state.cw_speed_wpm;
    operator.cw_macros = state
        .cw_macros
        .iter()
        .map(|m| crate::persistence::models::CwMacroFile {
            label: m.label.clone(),
            text: m.text.clone(),
        })
        .collect();

    operator.mic_device = state.mic_device.clone();
    operator.mic_gain_percent = state.mic_gain_percent;

    operator.voice_keyer_clip = state.voice_keyer_clip.clone();
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::persistence::{models::AppStateFile, paths::operators_dir};

    fn unique_tmp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "rigflow-startup-test-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn config_layout_failure_stops_initialization() {
        let parent = unique_tmp_dir();
        let config_path = parent.join("config-is-a-file");
        fs::write(&config_path, b"not a directory").unwrap();

        let error = initialize_client_state(Some(&config_path)).unwrap_err();
        assert!(matches!(
            error,
            StartupError::ConfigLayout { path, .. } if path == config_path
        ));

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn app_state_read_failure_stops_initialization() {
        let config_dir = unique_tmp_dir();
        fs::create_dir_all(app_state_path(&config_dir)).unwrap();

        let error = initialize_client_state(Some(&config_dir)).unwrap_err();
        assert!(matches!(
            error,
            StartupError::AppState { path, .. } if path == app_state_path(&config_dir)
        ));

        let _ = fs::remove_dir_all(config_dir);
    }

    #[test]
    fn operator_settings_read_failure_stops_initialization() {
        let config_dir = unique_tmp_dir();
        let store = PersistenceStore::new(config_dir.clone());
        let app_state = AppStateFile {
            last_operator_id: Some("W1AW".to_string()),
            known_operator_ids: vec!["W1AW".to_string()],
            ..AppStateFile::default()
        };
        store.save_app_state(&app_state).unwrap();
        fs::create_dir_all(operator_file_path(&config_dir, "W1AW")).unwrap();

        let error = initialize_client_state(Some(&config_dir)).unwrap_err();
        assert!(matches!(
            error,
            StartupError::OperatorSettings {
                operator_id,
                path,
                ..
            } if operator_id == "W1AW" && path == operator_file_path(&config_dir, "W1AW")
        ));

        let _ = fs::remove_dir_all(config_dir);
    }

    #[test]
    fn recoverable_corruption_still_starts_with_a_notice() {
        let config_dir = unique_tmp_dir();
        fs::write(app_state_path(&config_dir), b"not valid json").unwrap();

        let initial = initialize_client_state(Some(&config_dir)).unwrap();
        assert!(initial.ui_state.persistence_status.contains("corrupt"));
        assert_eq!(initial.persistence_store.config_dir(), config_dir);
        assert!(
            fs::read_dir(&config_dir)
                .unwrap()
                .filter_map(Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
        );
        assert!(operators_dir(&config_dir).is_dir());

        let _ = fs::remove_dir_all(config_dir);
    }
}

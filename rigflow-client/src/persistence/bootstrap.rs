use std::path::Path;

use crate::{
    persistence::{
        error::PersistenceError,
        models::{AppStateFile, OperatorSettingsFile},
        paths::resolve_config_dir,
        store::PersistenceStore,
    },
    ui::state::UiState,
};

/// Load the initial UI state and persistence store.
///
/// Behavior:
/// - resolves the config directory
/// - creates the persistence store
/// - loads the global app state
/// - starts from `UiState::default()`
/// - populates known operators
/// - if a last operator exists, loads and applies that operator's settings
///
/// This is intentionally load-only. Saving is wired later.
pub fn load_initial_ui_state(
    cli_config_dir: Option<&Path>,
) -> Result<(UiState, PersistenceStore), PersistenceError> {
    let config_dir = resolve_config_dir(cli_config_dir)?;
    let store = PersistenceStore::new(config_dir);

    let app_state = store.load_app_state()?;

    let mut ui_state = UiState::default();
    apply_app_state_to_ui_state(&mut ui_state, &app_state);

    if let Some(operator_id) = app_state.last_operator_id.as_deref() {
        let operator_settings = store.load_or_create_operator_settings(operator_id)?;
        apply_operator_settings_to_ui_state(&mut ui_state, &operator_settings, &app_state);
    }

    // Surface any corrupt-config recovery that happened during the loads above,
    // so the user sees it in the Problems area instead of a silent reset.
    let notices = store.take_recovery_notices();
    if !notices.is_empty() {
        ui_state.persistence_status = notices.join("; ");
    }

    Ok((ui_state, store))
}

/// Apply global app state to runtime UI state.
///
/// This should only copy fields that are truly global, such as:
/// - known operators
///
/// It should not assume that a last operator exists.
pub fn apply_app_state_to_ui_state(state: &mut UiState, app_state: &AppStateFile) {
    state.known_operator_ids = app_state.known_operator_ids.clone();
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

    state.volume_percent = operator.volume_percent;
    state.show_advanced = operator.show_advanced;

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
}

pub fn apply_ui_state_to_operator_settings(state: &UiState, operator: &mut OperatorSettingsFile) {
    operator.operator_id = state.operator_id.clone();
    operator.selected_license = state.selected_license;
    operator.server_ip = state.rigflow_server_ip.clone();

    operator.default_bookmark_id = state.default_bookmark_id.clone();
    operator.auto_apply_default_bookmark_on_acquire = state.auto_apply_default_bookmark_on_acquire;

    operator.bookmarks = state.bookmarks.clone();

    // --- NEW: persist per-demod preferences ---
    operator.demod_preferences = state.demod_preferences.clone();

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

    operator.volume_percent = state.volume_percent;
    operator.show_advanced = state.show_advanced;

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
}

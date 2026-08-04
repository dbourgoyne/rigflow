use std::{
    env,
    path::{Path, PathBuf},
};

use crate::persistence::error::PersistenceError;

/// Resolve the rigflow config directory.
///
/// Priority:
/// 1. explicit CLI override
/// 2. RIGFLOW_CONFIG_DIR
/// 3. the platform config directory reported by `dirs`
pub fn resolve_config_dir(cli_override: Option<&Path>) -> Result<PathBuf, PersistenceError> {
    if let Some(path) = cli_override {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = env::var_os("RIGFLOW_CONFIG_DIR") {
        return Ok(PathBuf::from(path));
    }

    dirs::config_dir()
        .map(|path| path.join("rigflow"))
        .ok_or(PersistenceError::NoConfigDirectory)
}

pub fn app_state_path(config_dir: &Path) -> PathBuf {
    config_dir.join("app_state.json")
}

pub fn operators_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("operators")
}

pub fn operator_file_path(config_dir: &Path, operator_id: &str) -> PathBuf {
    let file_name = format!("{}.json", sanitize_operator_id_for_file(operator_id));
    operators_dir(config_dir).join(file_name)
}

/// Per-operator data directory: `operators/<ID>/`, a sibling of the operator's
/// `<ID>.json` settings file.  Holds per-operator audio files (RX recordings and
/// voice-keyer clips) that must live with the operator on the client host.
pub fn operator_data_dir(config_dir: &Path, operator_id: &str) -> PathBuf {
    operators_dir(config_dir).join(sanitize_operator_id_for_file(operator_id))
}

/// Directory for this operator's RX audio recordings.
pub fn rx_recordings_dir(config_dir: &Path, operator_id: &str) -> PathBuf {
    operator_data_dir(config_dir, operator_id).join("rx_recordings")
}

/// Directory for this operator's SSB voice-keyer clips.
pub fn voice_keyer_clips_dir(config_dir: &Path, operator_id: &str) -> PathBuf {
    operator_data_dir(config_dir, operator_id).join("voice_keyer_clips")
}

/// This operator's contact-log SQLite database (the source of truth).
pub fn qso_log_db_path(config_dir: &Path, operator_id: &str) -> PathBuf {
    operator_data_dir(config_dir, operator_id).join("rigflow_log.db")
}

/// This operator's append-only ADIF journal (recovery + interop).
pub fn qso_log_journal_path(config_dir: &Path, operator_id: &str) -> PathBuf {
    operator_data_dir(config_dir, operator_id).join("rigflow_log.adi")
}

/// Normalize operator IDs for persistence.
///
/// Current behavior:
/// - trim whitespace
/// - uppercase
/// - reject path separators
pub fn normalize_operator_id(raw: &str) -> Result<String, PersistenceError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PersistenceError::InvalidOperatorId);
    }

    let normalized = trimmed.to_ascii_uppercase();

    if normalized.contains('/') || normalized.contains('\\') {
        return Err(PersistenceError::InvalidOperatorId);
    }

    Ok(normalized)
}

fn sanitize_operator_id_for_file(operator_id: &str) -> String {
    operator_id
        .chars()
        .map(|c| match c {
            'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

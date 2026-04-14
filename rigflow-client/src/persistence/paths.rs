use std::{
    env,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;

use crate::persistence::error::PersistenceError;

/// Resolve the rigflow config directory.
///
/// Priority:
/// 1. explicit CLI override
/// 2. RIGFLOW_CONFIG_DIR
/// 3. OS-standard config directory
pub fn resolve_config_dir(
    cli_override: Option<&Path>,
) -> Result<PathBuf, PersistenceError> {
    if let Some(path) = cli_override {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = env::var_os("RIGFLOW_CONFIG_DIR") {
        return Ok(PathBuf::from(path));
    }

    let project_dirs = ProjectDirs::from("com", "rigflow", "rigflow")
        .ok_or(PersistenceError::NoConfigDirectory)?;

    Ok(project_dirs.config_dir().to_path_buf())
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

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::persistence::{
    error::PersistenceError,
    migrations::migrate_operator_settings_value,
    models::{AppStateFile, OperatorSettingsFile},
    paths::{app_state_path, normalize_operator_id, operator_file_path, operators_dir},
};

#[derive(Debug, Clone)]
pub struct PersistenceStore {
    config_dir: PathBuf,
}

impl PersistenceStore {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Ensure the config directory layout exists.
    pub fn ensure_layout(&self) -> Result<(), PersistenceError> {
        fs::create_dir_all(&self.config_dir)?;
        fs::create_dir_all(operators_dir(&self.config_dir))?;
        Ok(())
    }

    pub fn load_app_state(&self) -> Result<AppStateFile, PersistenceError> {
        self.ensure_layout()?;

        let path = app_state_path(&self.config_dir);
        if !path.exists() {
            let default = AppStateFile::default();
            self.save_app_state(&default)?;
            return Ok(default);
        }

        read_json_file(&path)
    }

    pub fn save_app_state(&self, state: &AppStateFile) -> Result<(), PersistenceError> {
        self.ensure_layout()?;
        let path = app_state_path(&self.config_dir);
        write_json_file_atomic(&path, state)
    }

    pub fn load_operator_settings(
        &self,
        operator_id: &str,
    ) -> Result<OperatorSettingsFile, PersistenceError> {
        self.ensure_layout()?;

        let operator_id = normalize_operator_id(operator_id)?;
        let path = operator_file_path(&self.config_dir, &operator_id);

        if !path.exists() {
            let default = OperatorSettingsFile::new(operator_id);
            self.save_operator_settings(&default)?;
            return Ok(default);
        }

        // Load as raw JSON value, run the migration chain, then deserialize.
        let text = fs::read_to_string(&path)?;
        let raw: serde_json::Value = serde_json::from_str(&text)?;
        let (migrated, did_migrate) = migrate_operator_settings_value(raw)?;

        let settings: OperatorSettingsFile = serde_json::from_value(migrated)?;

        if did_migrate {
            // Best-effort write-back: log but don't fail the load if the
            // upgraded file can't be saved (e.g. read-only filesystem).
            if let Err(err) = self.save_operator_settings(&settings) {
                log::warn!(
                    "operator settings migrated successfully but could not \
                     be saved back to disk ({}): {err}",
                    path.display()
                );
            } else {
                log::info!(
                    "operator settings migrated to v{} and saved: {}",
                    crate::persistence::models::OPERATOR_SETTINGS_FILE_VERSION,
                    path.display()
                );
            }
        }

        Ok(settings)
    }

    pub fn save_operator_settings(
        &self,
        settings: &OperatorSettingsFile,
    ) -> Result<(), PersistenceError> {
        self.ensure_layout()?;

        let operator_id = normalize_operator_id(&settings.operator_id)?;
        let path = operator_file_path(&self.config_dir, &operator_id);
        write_json_file_atomic(&path, settings)
    }

    pub fn load_or_create_operator_settings(
        &self,
        operator_id: &str,
    ) -> Result<OperatorSettingsFile, PersistenceError> {
        self.load_operator_settings(operator_id)
    }

    pub fn upsert_known_operator(
        &self,
        operator_id: &str,
    ) -> Result<AppStateFile, PersistenceError> {
        let operator_id = normalize_operator_id(operator_id)?;
        let mut app_state = self.load_app_state()?;

        if !app_state
            .known_operator_ids
            .iter()
            .any(|id| id == &operator_id)
        {
            app_state.known_operator_ids.push(operator_id.clone());
            app_state.known_operator_ids.sort();
        }

        app_state.last_operator_id = Some(operator_id);
        self.save_app_state(&app_state)?;
        Ok(app_state)
    }
}

fn read_json_file<T>(path: &Path) -> Result<T, PersistenceError>
where
    T: serde::de::DeserializeOwned,
{
    let text = fs::read_to_string(path)?;
    let value = serde_json::from_str::<T>(&text)?;
    Ok(value)
}

fn write_json_file_atomic<T>(path: &Path, value: &T) -> Result<(), PersistenceError>
where
    T: serde::Serialize,
{
    let parent = path
        .parent()
        .ok_or_else(|| PersistenceError::InvalidPath(path.to_path_buf()))?;

    fs::create_dir_all(parent)?;

    let temp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value)?;

    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }

    fs::rename(&temp_path, path)?;
    Ok(())
}

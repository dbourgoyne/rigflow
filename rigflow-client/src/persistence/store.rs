use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
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
    /// Human-readable notices recorded when a corrupt/future-version config file
    /// was recovered (quarantined + reset, or preserved with defaults).  Shared
    /// across clones so a recovery on any clone is visible to all; drained by the
    /// UI via [`take_recovery_notices`] and surfaced in the Problems area.
    recovery_notices: Arc<Mutex<Vec<String>>>,
}

impl PersistenceStore {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            recovery_notices: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Drain and return any recovery notices accumulated since the last call.
    pub fn take_recovery_notices(&self) -> Vec<String> {
        match self.recovery_notices.lock() {
            Ok(mut notices) => std::mem::take(&mut *notices),
            Err(_) => Vec::new(),
        }
    }

    fn push_recovery_notice(&self, message: String) {
        log::warn!("[persistence] {message}");
        if let Ok(mut notices) = self.recovery_notices.lock() {
            notices.push(message);
        }
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

        match read_json_file::<AppStateFile>(&path) {
            Ok(state) => Ok(state),
            // Corrupt content: quarantine + reset rather than leaving the user
            // stuck.  Genuine Io/dir errors still propagate.
            Err(err) if err.is_content_corruption() => self.recover_app_state(&path, err),
            Err(err) => Err(err),
        }
    }

    /// Quarantine a corrupt `app_state.json` and return a fresh default (saved).
    fn recover_app_state(
        &self,
        path: &Path,
        err: PersistenceError,
    ) -> Result<AppStateFile, PersistenceError> {
        match quarantine_corrupt_file(path) {
            Ok(backup) => self.push_recovery_notice(format!(
                "app settings were corrupt ({err}) and have been reset to \
                 defaults (backup: {})",
                backup.display()
            )),
            Err(rename_err) => {
                log::warn!(
                    "[persistence] could not back up corrupt app state ({rename_err}); \
                     resetting in place"
                );
                self.push_recovery_notice(format!(
                    "app settings were corrupt ({err}) and have been reset to defaults"
                ));
            }
        }

        let default = AppStateFile::default();
        // Overwrite the (possibly still-present) corrupt file with defaults so we
        // don't recover on every launch.
        self.save_app_state(&default)?;
        Ok(default)
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

        match self.try_load_operator_settings(&path) {
            Ok(settings) => Ok(settings),
            // Bad bytes on disk: quarantine + reset.
            Err(err) if err.is_content_corruption() => {
                self.recover_operator_settings_corrupt(&path, &operator_id, err)
            }
            // Valid file from a newer build (downgrade): preserve it, use
            // in-memory defaults, tell the user to upgrade.
            Err(err) if err.is_version_too_new() => {
                self.recover_operator_settings_future_version(&operator_id, err)
            }
            // Genuine Io / invalid-id errors still propagate.
            Err(err) => Err(err),
        }
    }

    /// Read + migrate + deserialize an existing operator file (no recovery).
    fn try_load_operator_settings(
        &self,
        path: &Path,
    ) -> Result<OperatorSettingsFile, PersistenceError> {
        // Load as raw JSON value, run the migration chain, then deserialize.
        let text = fs::read_to_string(path)?;
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

    /// Quarantine a corrupt operator file and return fresh defaults (saved).
    fn recover_operator_settings_corrupt(
        &self,
        path: &Path,
        operator_id: &str,
        err: PersistenceError,
    ) -> Result<OperatorSettingsFile, PersistenceError> {
        match quarantine_corrupt_file(path) {
            Ok(backup) => self.push_recovery_notice(format!(
                "operator '{operator_id}' settings were corrupt ({err}) and have \
                 been reset to defaults (backup: {})",
                backup.display()
            )),
            Err(rename_err) => {
                log::warn!(
                    "[persistence] could not back up corrupt operator settings \
                     ({rename_err}); resetting in place"
                );
                self.push_recovery_notice(format!(
                    "operator '{operator_id}' settings were corrupt ({err}) and \
                     have been reset to defaults"
                ));
            }
        }

        let default = OperatorSettingsFile::new(operator_id.to_string());
        self.save_operator_settings(&default)?;
        Ok(default)
    }

    /// A valid operator file from a *newer* build: leave it on disk untouched,
    /// run on in-memory defaults, and surface an "upgrade" notice.
    fn recover_operator_settings_future_version(
        &self,
        operator_id: &str,
        err: PersistenceError,
    ) -> Result<OperatorSettingsFile, PersistenceError> {
        self.push_recovery_notice(format!(
            "config for '{operator_id}' is from a newer Rigflow build and was not \
             loaded ({err}); using defaults this session — upgrade to use it"
        ));
        // Deliberately NOT saved, so the good file is preserved for an upgrade.
        Ok(OperatorSettingsFile::new(operator_id.to_string()))
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

/// Rename a corrupt file aside to `<path>.corrupt-<unix_secs>` so it's preserved
/// for inspection while the caller writes fresh defaults.  Returns the backup
/// path on success.
fn quarantine_corrupt_file(path: &Path) -> std::io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut backup = path.as_os_str().to_os_string();
    backup.push(format!(".corrupt-{secs}"));
    let backup = PathBuf::from(backup);
    fs::rename(path, &backup)?;
    Ok(backup)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::models::{APP_STATE_FILE_VERSION, OPERATOR_SETTINGS_FILE_VERSION};

    fn unique_tmp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "rigflow-persist-test-{}-{nanos}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Count files in `dir` whose name marks a quarantined backup.
    fn corrupt_backups(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.contains(".corrupt-"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn corrupt_operator_file_is_quarantined_and_reset() {
        let dir = unique_tmp_dir();
        let store = PersistenceStore::new(dir.clone());
        store.ensure_layout().unwrap();

        let path = operator_file_path(&dir, "W1AW");
        fs::write(&path, b"{ this is not valid json").unwrap();

        let settings = store.load_operator_settings("W1AW").unwrap();
        assert_eq!(settings.operator_id, "W1AW");
        assert_eq!(settings.version, OPERATOR_SETTINGS_FILE_VERSION);

        // Exactly one backup, and the live file is now valid defaults.
        assert_eq!(corrupt_backups(&operators_dir(&dir)).len(), 1);
        let reloaded: OperatorSettingsFile =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reloaded.operator_id, "W1AW");

        let notices = store.take_recovery_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("corrupt"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn future_version_operator_file_is_preserved() {
        let dir = unique_tmp_dir();
        let store = PersistenceStore::new(dir.clone());
        store.ensure_layout().unwrap();

        let path = operator_file_path(&dir, "W1AW");
        let original = br#"{"version": 9999}"#;
        fs::write(&path, original).unwrap();

        let settings = store.load_operator_settings("W1AW").unwrap();
        assert_eq!(settings.version, OPERATOR_SETTINGS_FILE_VERSION);

        // The good file is left byte-for-byte intact; no backup made.
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(corrupt_backups(&operators_dir(&dir)).is_empty());

        let notices = store.take_recovery_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("newer"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_operator_file_is_untouched() {
        let dir = unique_tmp_dir();
        let store = PersistenceStore::new(dir.clone());

        // First load creates a valid default file.
        let created = store.load_operator_settings("W1AW").unwrap();
        let path = operator_file_path(&dir, "W1AW");
        let before = fs::read(&path).unwrap();

        let loaded = store.load_operator_settings("W1AW").unwrap();
        assert_eq!(loaded.operator_id, created.operator_id);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(corrupt_backups(&operators_dir(&dir)).is_empty());
        assert!(store.take_recovery_notices().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_app_state_is_quarantined_and_reset() {
        let dir = unique_tmp_dir();
        let store = PersistenceStore::new(dir.clone());
        store.ensure_layout().unwrap();

        let path = app_state_path(&dir);
        fs::write(&path, b"garbage").unwrap();

        let state = store.load_app_state().unwrap();
        assert_eq!(state.version, APP_STATE_FILE_VERSION);
        assert_eq!(corrupt_backups(&dir).len(), 1);
        let notices = store.take_recovery_notices();
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("corrupt"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn error_classification() {
        let json_err = PersistenceError::from(serde_json::from_str::<i32>("x").unwrap_err());
        assert!(json_err.is_content_corruption());
        assert!(!json_err.is_version_too_new());

        let io_err = PersistenceError::from(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "x",
        ));
        assert!(!io_err.is_content_corruption());
        assert!(!io_err.is_version_too_new());

        let mig = PersistenceError::Migration("too new".to_string());
        assert!(mig.is_version_too_new());
        assert!(!mig.is_content_corruption());
    }

    #[test]
    fn recovery_notices_shared_across_clones() {
        let dir = unique_tmp_dir();
        let store = PersistenceStore::new(dir.clone());
        store.ensure_layout().unwrap();
        fs::write(operator_file_path(&dir, "W1AW"), b"not json").unwrap();

        // Trigger recovery through a clone; the original must see the notice.
        let clone = store.clone();
        let _ = clone.load_operator_settings("W1AW").unwrap();
        assert_eq!(store.take_recovery_notices().len(), 1);

        let _ = fs::remove_dir_all(&dir);
    }
}

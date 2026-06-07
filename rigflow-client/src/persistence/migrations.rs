//! Versioned migration chain for operator settings files.
//!
//! ## Version history
//!
//! | Version | Change |
//! |---------|--------|
//! | 1       | Original format (no waterfall_display_preferences) |
//! | 2       | Added `waterfall_display_preferences` |
//! | 3       | Added `source_control_preferences` |
//!
//! ## Usage
//!
//! Call [`migrate_operator_settings_value`] after reading the raw JSON bytes
//! and before deserializing into [`OperatorSettingsFile`].
//!
//! The function returns `(migrated_value, did_migrate)`.  When `did_migrate`
//! is `true`, the caller should write the migrated value back to disk so that
//! subsequent loads skip the migration entirely.

use serde_json::{Value, json};

use crate::persistence::error::PersistenceError;
use crate::persistence::models::OPERATOR_SETTINGS_FILE_VERSION;

/// Apply the full migration chain to a raw operator-settings JSON value.
///
/// Returns `(value, did_migrate)`:
/// - `value` — the (possibly upgraded) JSON object ready for deserialization.
/// - `did_migrate` — `true` when at least one migration was applied; the
///   caller should save the upgraded value back to disk.
///
/// # Errors
///
/// Returns [`PersistenceError::Migration`] when the stored version is higher
/// than [`OPERATOR_SETTINGS_FILE_VERSION`], which means this binary is too
/// old to handle the file.
pub fn migrate_operator_settings_value(
    mut value: Value,
) -> Result<(Value, bool), PersistenceError> {
    let stored_version = extract_version(&value);
    let current = OPERATOR_SETTINGS_FILE_VERSION;

    if stored_version > current {
        return Err(PersistenceError::Migration(format!(
            "operator settings file is version {stored_version} but this build \
             only understands up to version {current}; please upgrade rigflow"
        )));
    }

    if stored_version == current {
        return Ok((value, false));
    }

    // Apply sequential migrations.
    let mut did_migrate = false;

    if stored_version < 2 {
        value = migrate_v1_to_v2(value);
        did_migrate = true;
    }

    if stored_version < 3 {
        value = migrate_v2_to_v3(value);
        did_migrate = true;
    }

    // Set the version field to current so the saved file won't be migrated again.
    if did_migrate {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("version".to_string(), json!(current));
        }
    }

    Ok((value, did_migrate))
}

/// Extract the `version` field from a JSON object.
///
/// Returns `1` when the field is absent (pre-versioning files) and `0` when
/// the value is not a JSON object at all (defensive fallback).
fn extract_version(value: &Value) -> u32 {
    value
        .as_object()
        .and_then(|obj| obj.get("version"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(1) // absent version → treat as v1
}

/// v1 → v2: add `waterfall_display_preferences` with defaults if missing.
fn migrate_v1_to_v2(mut value: Value) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.entry("waterfall_display_preferences")
            .or_insert_with(|| {
                json!({
                    "display_zoom": 1.0,
                    "adaptive_waterfall_normalization": true,
                    "manual_waterfall_top_db": -35.0,
                    "manual_waterfall_range_db": 70.0
                })
            });
    }
    value
}

/// v2 → v3: add `source_control_preferences` as an empty object if missing.
fn migrate_v2_to_v3(mut value: Value) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.entry("source_control_preferences")
            .or_insert_with(|| json!({}));
    }
    value
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_v2() -> Value {
        json!({
            "version": 2,
            "operator_id": "W1AW",
            "selected_license": null,
            "server_ip": "192.168.1.1",
            "demod_preferences": {
                "wfm": { "filter_bandwidth_hz": 15000.0, "pitch_hz": 0.0, "deemphasis_mode": "tau75us" },
                "nfm": { "filter_bandwidth_hz": 4000.0,  "pitch_hz": 0.0, "deemphasis_mode": "tau75us" },
                "am":  { "filter_bandwidth_hz": 5000.0,  "pitch_hz": 0.0, "deemphasis_mode": "off" },
                "usb": { "filter_bandwidth_hz": 2700.0,  "pitch_hz": 0.0, "deemphasis_mode": "off" },
                "lsb": { "filter_bandwidth_hz": 2700.0,  "pitch_hz": 0.0, "deemphasis_mode": "off" },
                "cw":  { "filter_bandwidth_hz": 500.0,   "pitch_hz": 600.0, "deemphasis_mode": "off" }
            },
            "default_bookmark_id": null,
            "auto_apply_default_bookmark_on_acquire": false,
            "bookmarks": [],
            "waterfall_display_preferences": {
                "display_zoom": 1.5,
                "adaptive_waterfall_normalization": false,
                "manual_waterfall_top_db": -40.0,
                "manual_waterfall_range_db": 80.0
            }
        })
    }

    fn minimal_v1() -> Value {
        json!({
            "version": 1,
            "operator_id": "W1AW",
            "selected_license": null,
            "server_ip": "",
            "demod_preferences": {
                "wfm": { "filter_bandwidth_hz": 15000.0, "pitch_hz": 0.0, "deemphasis_mode": "tau75us" },
                "nfm": { "filter_bandwidth_hz": 4000.0,  "pitch_hz": 0.0, "deemphasis_mode": "tau75us" },
                "am":  { "filter_bandwidth_hz": 5000.0,  "pitch_hz": 0.0, "deemphasis_mode": "off" },
                "usb": { "filter_bandwidth_hz": 2700.0,  "pitch_hz": 0.0, "deemphasis_mode": "off" },
                "lsb": { "filter_bandwidth_hz": 2700.0,  "pitch_hz": 0.0, "deemphasis_mode": "off" },
                "cw":  { "filter_bandwidth_hz": 500.0,   "pitch_hz": 600.0, "deemphasis_mode": "off" }
            },
            "default_bookmark_id": null,
            "auto_apply_default_bookmark_on_acquire": false,
            "bookmarks": []
        })
    }

    // ------------------------------------------------------------------
    // Version detection
    // ------------------------------------------------------------------

    #[test]
    fn current_version_loads_without_migration() {
        let v = json!({ "version": 3 });
        let (_, did_migrate) = migrate_operator_settings_value(v).unwrap();
        assert!(!did_migrate);
    }

    #[test]
    fn future_version_returns_error() {
        let v = json!({ "version": 9999 });
        assert!(matches!(
            migrate_operator_settings_value(v),
            Err(PersistenceError::Migration(_))
        ));
    }

    // ------------------------------------------------------------------
    // v2 → v3
    // ------------------------------------------------------------------

    #[test]
    fn v2_migrates_to_v3() {
        let (migrated, did_migrate) = migrate_operator_settings_value(minimal_v2()).unwrap();
        assert!(did_migrate);
        assert_eq!(migrated["version"], 3);
    }

    #[test]
    fn v2_gains_empty_source_control_preferences() {
        let (migrated, _) = migrate_operator_settings_value(minimal_v2()).unwrap();
        assert_eq!(migrated["source_control_preferences"], json!({}));
    }

    #[test]
    fn v2_existing_waterfall_prefs_are_preserved() {
        let (migrated, _) = migrate_operator_settings_value(minimal_v2()).unwrap();
        assert_eq!(
            migrated["waterfall_display_preferences"]["display_zoom"],
            1.5
        );
        assert_eq!(
            migrated["waterfall_display_preferences"]["manual_waterfall_top_db"],
            -40.0
        );
    }

    #[test]
    fn v2_bookmarks_are_preserved() {
        let mut v = minimal_v2();
        v["bookmarks"] = json!([
            { "id": "bk1", "name": "Test", "frequency_hz": 14200000.0,
              "demod_mode": "usb", "sideband": null, "display": null, "notes": null }
        ]);
        let (migrated, _) = migrate_operator_settings_value(v).unwrap();
        assert_eq!(migrated["bookmarks"].as_array().unwrap().len(), 1);
        assert_eq!(migrated["bookmarks"][0]["id"], "bk1");
    }

    #[test]
    fn v2_server_ip_preserved() {
        let (migrated, _) = migrate_operator_settings_value(minimal_v2()).unwrap();
        assert_eq!(migrated["server_ip"], "192.168.1.1");
    }

    // ------------------------------------------------------------------
    // v1 → v3 (full chain)
    // ------------------------------------------------------------------

    #[test]
    fn v1_migrates_to_v3() {
        let (migrated, did_migrate) = migrate_operator_settings_value(minimal_v1()).unwrap();
        assert!(did_migrate);
        assert_eq!(migrated["version"], 3);
    }

    #[test]
    fn v1_gains_waterfall_defaults() {
        let (migrated, _) = migrate_operator_settings_value(minimal_v1()).unwrap();
        assert!(migrated["waterfall_display_preferences"].is_object());
        assert_eq!(
            migrated["waterfall_display_preferences"]["adaptive_waterfall_normalization"],
            true
        );
    }

    #[test]
    fn v1_gains_empty_source_control_preferences() {
        let (migrated, _) = migrate_operator_settings_value(minimal_v1()).unwrap();
        assert_eq!(migrated["source_control_preferences"], json!({}));
    }

    // ------------------------------------------------------------------
    // source_control_preferences round-trip
    // ------------------------------------------------------------------

    #[test]
    fn source_control_preferences_survives_migration_if_already_present() {
        // A v2 file that somehow already has the field (shouldn't happen, but
        // migration must be idempotent and not overwrite an existing value).
        let mut v = minimal_v2();
        v["source_control_preferences"] = json!({
            "hl2:192.168.1.116": {
                "sample_rate_hz": 96000,
                "gain_mode": "manual",
                "gain_db": 30.0,
                "ppm_correction": 0,
                "direct_sampling": "off"
            }
        });
        let (migrated, _) = migrate_operator_settings_value(v).unwrap();
        // The existing entry must be preserved.
        let prefs = &migrated["source_control_preferences"];
        assert!(prefs["hl2:192.168.1.116"].is_object());
        assert_eq!(prefs["hl2:192.168.1.116"]["sample_rate_hz"], 96000);
        assert_eq!(prefs["hl2:192.168.1.116"]["gain_db"], 30.0);
    }

    #[test]
    fn demod_preferences_preserved_through_migration() {
        let (migrated, _) = migrate_operator_settings_value(minimal_v2()).unwrap();
        assert_eq!(migrated["demod_preferences"]["cw"]["pitch_hz"], 600.0);
        assert_eq!(
            migrated["demod_preferences"]["usb"]["filter_bandwidth_hz"],
            2700.0
        );
    }
}

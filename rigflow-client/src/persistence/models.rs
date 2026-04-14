use serde::{Deserialize, Serialize};

use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::{DemodMode, Sideband};

pub const APP_STATE_FILE_VERSION: u32 = 1;
pub const OPERATOR_SETTINGS_FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateFile {
    pub version: u32,
    pub last_operator_id: Option<String>,
    pub known_operator_ids: Vec<String>,
}

impl Default for AppStateFile {
    fn default() -> Self {
        Self {
            version: APP_STATE_FILE_VERSION,
            last_operator_id: None,
            known_operator_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSettingsFile {
    pub version: u32,
    pub operator_id: String,

    pub selected_license: Option<LicenseClass>,
    pub server_ip: String,

    pub default_bookmark_id: Option<String>,
    pub auto_apply_default_bookmark_on_acquire: bool,

    pub bookmarks: Vec<BookmarkFile>,
}

impl OperatorSettingsFile {
    pub fn new(operator_id: String) -> Self {
        Self {
            version: OPERATOR_SETTINGS_FILE_VERSION,
            operator_id,
            selected_license: None,
            server_ip: String::new(),
            default_bookmark_id: None,
            auto_apply_default_bookmark_on_acquire: false,
            bookmarks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkFile {
    pub id: String,
    pub name: String,

    pub frequency_hz: f32,
    pub demod_mode: DemodMode,
    pub sideband: Option<Sideband>,

    pub display: Option<BookmarkDisplaySettingsFile>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkDisplaySettingsFile {
    pub zoom: Option<f32>,
    pub adaptive_waterfall_normalization: Option<bool>,
    pub waterfall_top_db: Option<f32>,
    pub waterfall_range_db: Option<f32>,
}

use serde::{Deserialize, Serialize};

use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::{DemodMode, Sideband};

pub const APP_STATE_FILE_VERSION: u32 = 1;
pub const OPERATOR_SETTINGS_FILE_VERSION: u32 = 2;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DemodPreferencesFile {
    pub filter_bandwidth_hz: f32,
    pub pitch_hz: f32,
}

impl DemodPreferencesFile {
    pub fn new(filter_bandwidth_hz: f32, pitch_hz: f32) -> Self {
        Self {
            filter_bandwidth_hz,
            pitch_hz,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemodPreferenceSetFile {
    pub wfm: DemodPreferencesFile,
    pub nfm: DemodPreferencesFile,
    pub am: DemodPreferencesFile,
    pub usb: DemodPreferencesFile,
    pub lsb: DemodPreferencesFile,
    pub cw: DemodPreferencesFile,
}

impl Default for DemodPreferenceSetFile {
    fn default() -> Self {
        Self {
            wfm: DemodPreferencesFile::new(15_000.0, 0.0),
            nfm: DemodPreferencesFile::new(4_000.0, 0.0),
            am: DemodPreferencesFile::new(5_000.0, 0.0),
            usb: DemodPreferencesFile::new(2_700.0, 0.0),
            lsb: DemodPreferencesFile::new(2_700.0, 0.0),
            cw: DemodPreferencesFile::new(500.0, 600.0),
        }
    }
}

impl DemodPreferenceSetFile {
    pub fn get(&self, mode: DemodMode) -> DemodPreferencesFile {
        match mode {
            DemodMode::Wfm => self.wfm,
            DemodMode::Nfm => self.nfm,
            DemodMode::Am => self.am,
            DemodMode::Usb => self.usb,
            DemodMode::Lsb => self.lsb,
            DemodMode::Cw => self.cw,
        }
    }

    pub fn get_mut(&mut self, mode: DemodMode) -> &mut DemodPreferencesFile {
        match mode {
            DemodMode::Wfm => &mut self.wfm,
            DemodMode::Nfm => &mut self.nfm,
            DemodMode::Am => &mut self.am,
            DemodMode::Usb => &mut self.usb,
            DemodMode::Lsb => &mut self.lsb,
            DemodMode::Cw => &mut self.cw,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorSettingsFile {
    pub version: u32,
    pub operator_id: String,

    pub selected_license: Option<LicenseClass>,
    pub server_ip: String,

    pub demod_preferences: DemodPreferenceSetFile,

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
            demod_preferences: DemodPreferenceSetFile::default(),
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

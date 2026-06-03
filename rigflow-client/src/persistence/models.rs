use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::{DeemphasisMode, DemodMode, Sideband};
use rigflow_core::radio::source_control::SourceControlState;

pub const APP_STATE_FILE_VERSION: u32 = 1;
pub const OPERATOR_SETTINGS_FILE_VERSION: u32 = 3;

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
pub struct WaterfallDisplayPreferencesFile {
    pub display_zoom: f32,
    pub adaptive_waterfall_normalization: bool,
    pub manual_waterfall_top_db: f32,
    pub manual_waterfall_range_db: f32,
}

impl Default for WaterfallDisplayPreferencesFile {
    fn default() -> Self {
        Self {
            display_zoom: 1.0,
            adaptive_waterfall_normalization: true,
            manual_waterfall_top_db: -35.0,
            manual_waterfall_range_db: 70.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DemodPreferencesFile {
    pub filter_bandwidth_hz: f32,
    pub pitch_hz: f32,
    pub deemphasis_mode: DeemphasisMode,
}

impl DemodPreferencesFile {
    pub fn new(filter_bandwidth_hz: f32, pitch_hz: f32, deemphasis_mode: DeemphasisMode) -> Self {
        Self {
            filter_bandwidth_hz,
            pitch_hz,
            deemphasis_mode,
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
            wfm: DemodPreferencesFile::new(15_000.0, 0.0, DeemphasisMode::Tau75us),
            nfm: DemodPreferencesFile::new(4_000.0, 0.0, DeemphasisMode::Tau75us),
            am: DemodPreferencesFile::new(5_000.0, 0.0, DeemphasisMode::Off),
            usb: DemodPreferencesFile::new(2_700.0, 0.0, DeemphasisMode::Off),
            lsb: DemodPreferencesFile::new(2_700.0, 0.0, DeemphasisMode::Off),
            cw: DemodPreferencesFile::new(500.0, 600.0, DeemphasisMode::Off),
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
            // CWU and CWL share one CW preference set (filter bw, pitch).
            DemodMode::Cwu | DemodMode::Cwl => self.cw,
        }
    }

    pub fn get_mut(&mut self, mode: DemodMode) -> &mut DemodPreferencesFile {
        match mode {
            DemodMode::Wfm => &mut self.wfm,
            DemodMode::Nfm => &mut self.nfm,
            DemodMode::Am => &mut self.am,
            DemodMode::Usb => &mut self.usb,
            DemodMode::Lsb => &mut self.lsb,
            DemodMode::Cwu | DemodMode::Cwl => &mut self.cw,
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

    pub waterfall_display_preferences: WaterfallDisplayPreferencesFile,

    /// Persisted source-control settings, keyed by radio ID string
    /// (e.g. `"hl2:192.168.1.116"`, `"rtl:0"`).
    ///
    /// Added in schema v3.  Deserialization falls back to an empty map so
    /// that a migrated file that is somehow missing this field still loads.
    /// TX Drive (`tx_drive_percent`) lives inside `SourceControlState`, so it
    /// persists per-radio here alongside sample rate / gain.
    #[serde(default)]
    pub source_control_preferences: HashMap<String, SourceControlState>,

    /// Receive-audio volume in percent (0–100), persisted per operator.
    /// Serde default so older settings files load without migration.
    #[serde(default = "default_volume_percent")]
    pub volume_percent: u8,
}

pub fn default_volume_percent() -> u8 {
    50
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
            waterfall_display_preferences: WaterfallDisplayPreferencesFile::default(),
            source_control_preferences: HashMap::new(),
            volume_percent: default_volume_percent(),
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

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaterfallDisplayPreferencesFile {
    pub display_zoom: f32,
    pub adaptive_waterfall_normalization: bool,
    pub manual_waterfall_top_db: f32,
    pub manual_waterfall_range_db: f32,
    /// Waterfall frame rate in Hz (0 = off). Serde default keeps older operator
    /// files (written before this field existed) loading cleanly.
    #[serde(default = "default_waterfall_frame_rate_hz")]
    pub waterfall_frame_rate_hz: f32,
}

fn default_waterfall_frame_rate_hz() -> f32 {
    20.0
}

impl Default for WaterfallDisplayPreferencesFile {
    fn default() -> Self {
        Self {
            display_zoom: 1.0,
            adaptive_waterfall_normalization: true,
            manual_waterfall_top_db: -35.0,
            manual_waterfall_range_db: 70.0,
            waterfall_frame_rate_hz: default_waterfall_frame_rate_hz(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DemodPreferenceSetFile {
    pub wfm: DemodPreferencesFile,
    pub nfm: DemodPreferencesFile,
    pub am: DemodPreferencesFile,
    pub usb: DemodPreferencesFile,
    pub lsb: DemodPreferencesFile,
    pub cw: DemodPreferencesFile,
    /// Data/digital USB (FT8 etc.).  Serde default (3 kHz) so older operator
    /// files — which predate this mode — load unchanged.
    #[serde(default = "default_dgt_u_prefs")]
    pub dgt_u: DemodPreferencesFile,
}

fn default_dgt_u_prefs() -> DemodPreferencesFile {
    DemodPreferencesFile::new(3_000.0, 0.0, DeemphasisMode::Off)
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
            dgt_u: default_dgt_u_prefs(),
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
            DemodMode::DgtU => self.dgt_u,
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
            DemodMode::DgtU => &mut self.dgt_u,
        }
    }
}

/// Per-(operator, radio) operating state — restored when the radio is acquired
/// and saved on change, so an operator resumes each radio exactly where they
/// left off.  Lives in `OperatorSettingsFile.radio_settings` keyed by radio ID,
/// so it is inherently scoped per (operator, radio).  Source-control state stays
/// in the separate `source_control_preferences` map (also per-radio).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadioSettingsFile {
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub demod_mode: DemodMode,
    pub sideband: Sideband,
    pub demod_preferences: DemodPreferenceSetFile,
    pub waterfall_display_preferences: WaterfallDisplayPreferencesFile,
    pub volume_percent: u8,
    pub cw_sidetone_volume: u8,
    pub cw_hang_ms: u32,
    pub squelch_enabled: bool,
    pub squelch_threshold_db: f32,
    pub nr2_enabled: bool,
    pub nr2_strength: f32,
    pub agc_enabled: bool,
    pub agc_strength: f32,

    // TX processing + CW decode — added after the first release of
    // `radio_settings`, so each carries a serde default to keep already-saved
    // buckets (which lack these fields) loading cleanly.
    #[serde(default = "default_tx_limiter_enabled")]
    pub tx_limiter_enabled: bool,
    #[serde(default = "default_tx_limiter_threshold_percent")]
    pub tx_limiter_threshold_percent: u16,
    #[serde(default)]
    pub compressor_enabled: bool,
    #[serde(default = "default_compressor_level")]
    pub compressor_level: u8,
    #[serde(default)]
    pub cw_decode_enabled: bool,
}

fn default_tx_limiter_enabled() -> bool {
    true
}
fn default_tx_limiter_threshold_percent() -> u16 {
    90
}
fn default_compressor_level() -> u8 {
    3
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

    /// Per-radio operating state (Radio Control + Waterfall): mode, filters,
    /// squelch/NR2/AGC, volume, CW sidetone/hang, waterfall display — keyed by
    /// radio ID.  Serde default (empty) so older files load; a radio with no
    /// entry starts from the operator-level defaults below and gets an entry on
    /// first acquire.
    #[serde(default)]
    pub radio_settings: HashMap<String, RadioSettingsFile>,

    /// Receive-audio volume in percent (0–100), persisted per operator.
    /// Serde default so older settings files load without migration.
    #[serde(default = "default_volume_percent")]
    pub volume_percent: u8,

    /// Show the Advanced & Diagnostics controls in Radio Control.  Serde default
    /// (false) so older settings files load without migration.
    #[serde(default)]
    pub show_advanced: bool,

    /// Text-to-CW: last-used message text.  Serde default (empty) for old files.
    #[serde(default)]
    pub cw_message: String,

    /// Text-to-CW: sending speed in WPM (5–50).  Serde default for old files.
    #[serde(default = "default_cw_speed_wpm")]
    pub cw_speed_wpm: u32,

    /// CW memory macros (label + text).  Serde default = the 4 stock macros so
    /// older settings files load with sensible content.
    #[serde(default = "default_cw_macros")]
    pub cw_macros: Vec<CwMacroFile>,

    /// Selected microphone input device name ("" = system default).
    #[serde(default)]
    pub mic_device: String,

    /// Microphone measurement gain in percent (0–200).
    #[serde(default = "default_mic_gain_percent")]
    pub mic_gain_percent: u16,
}

/// Persisted CW memory macro (label + transmit text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwMacroFile {
    pub label: String,
    pub text: String,
}

pub fn default_volume_percent() -> u8 {
    50
}

pub fn default_cw_speed_wpm() -> u32 {
    20
}

pub fn default_mic_gain_percent() -> u16 {
    100
}

pub fn default_cw_macros() -> Vec<CwMacroFile> {
    crate::cw_text::CW_MACRO_DEFAULTS
        .iter()
        .map(|(label, text)| CwMacroFile {
            label: label.to_string(),
            text: text.to_string(),
        })
        .collect()
}

impl OperatorSettingsFile {
    pub fn new(operator_id: String) -> Self {
        Self {
            version: OPERATOR_SETTINGS_FILE_VERSION,
            operator_id,
            selected_license: None,
            // Seed new operators with localhost so a single-box (client+server on
            // one machine) setup can Connect with no typing; the user edits it for
            // a remote/Pi server.  Persisted per-operator thereafter.
            server_ip: "127.0.0.1".to_string(),
            demod_preferences: DemodPreferenceSetFile::default(),
            default_bookmark_id: None,
            auto_apply_default_bookmark_on_acquire: false,
            bookmarks: Vec::new(),
            waterfall_display_preferences: WaterfallDisplayPreferencesFile::default(),
            source_control_preferences: HashMap::new(),
            radio_settings: HashMap::new(),
            volume_percent: default_volume_percent(),
            show_advanced: false,
            cw_message: String::new(),
            cw_speed_wpm: default_cw_speed_wpm(),
            cw_macros: default_cw_macros(),
            mic_device: String::new(),
            mic_gain_percent: default_mic_gain_percent(),
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

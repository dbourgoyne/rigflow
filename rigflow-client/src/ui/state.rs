use std::collections::HashMap;
use std::time::Instant;

use crate::persistence::models::DemodPreferenceSetFile;
use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::DeemphasisMode;
use rigflow_core::dsp::modes::{DemodMode, Sideband};
use rigflow_core::radio::source_control::{SourceCapabilities, SourceControlState};
use rigflow_core::radio::source_status::SourceStatus;
use rigflow_core::radio::tx_tune::TxTuneResult;
use rigflow_core::radio::RadioCapabilities;

#[derive(Debug, Clone, Copy)]
pub struct DebounceState {
    pub last_sent_value: f32,
    pub last_send_time: Instant,
}
impl DebounceState {
    pub fn new(initial: f32) -> Self {
        Self {
            last_sent_value: initial,
            last_send_time: Instant::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiState {
    // =====================================================================
    // RADIO STATE (Operator-facing, synchronized with server)
    // =====================================================================
    /// Center frequency (LO), in Hz
    pub center_freq_hz: f32,

    /// Target tuned frequency, in Hz
    pub target_freq_hz: f32,

    /// Current demodulation mode
    pub demod_mode: DemodMode,

    /// Current sideband (SSB)
    pub sideband: Sideband,

    /// pitch (Hz)
    pub pitch_hz: f32,

    /// Audio filter bandwidth (Hz)
    pub filter_bandwidth_hz: f32,

    pub deemphasis_mode: DeemphasisMode,

    /// Input sample rate from SDR source (Hz)
    pub input_sample_rate_hz: f32,

    // =====================================================================
    // RADIO-DERIVED UI STATE
    // (depends on demod mode or radio state)
    // =====================================================================
    /// Tracks last demod mode for applying defaults (e.g. bandwidth)
    pub last_demod_mode_for_controls: Option<DemodMode>,

    /// One-shot flag: after radio acquire, reapply current mode controls
    pub pending_apply_mode_controls: bool,

    // =====================================================================
    // UI RUNTIME / HELPER STATE (non-persistent, non-radio)
    // =====================================================================
    /// Last filter bandwidth value sent to server (for debounce)
    pub filter_bw_debounce: DebounceState,

    /// Pitch debounce (shared across modes)
    pub pitch_debounce: DebounceState,

    // =====================================================================
    // CONNECTION / SERVER STATE
    // =====================================================================
    /// Server IP address entered by the user
    pub rigflow_server_ip: String,

    /// WebSocket port
    pub rigflow_server_ws_port: u16,

    /// UDP port (server)
    pub rigflow_server_udp_port: u16,

    /// Local UDP listen port
    pub udp_listen_port: u16,

    /// Whether connected to server
    pub server_connected: bool,

    /// Whether a radio is currently acquired
    pub radio_acquired: bool,

    /// Human-readable server status
    pub server_status: String,

    /// Available radios
    pub available_radios: Vec<rigflow_protocol::radio_control::RadioInfo>,

    /// Selected radio
    pub selected_radio_id: Option<String>,

    // =====================================================================
    // UI STATE (Rendering / Interaction)
    // =====================================================================
    pub runtime_error: String,

    pub selected_license: Option<LicenseClass>,

    pub spectrum_zoom_x: f32,

    // =====================================================================
    // WATERFALL / DISPLAY
    // =====================================================================
    pub adaptive_waterfall_normalization: bool,

    // persisted manual controls
    pub manual_waterfall_top_db: f32,
    pub manual_waterfall_range_db: f32,

    // runtime adaptive estimates, not persisted
    pub adaptive_top_db_estimate: f32,
    pub adaptive_floor_db_estimate: f32,
    pub adaptive_range_db_estimate: f32,

    pub display_zoom: f32,

    // =====================================================================
    // OPERATOR / PERSISTENCE (logical state, even if not yet persisted)
    // =====================================================================
    pub operator_id: String,
    pub known_operator_ids: Vec<String>,

    pub show_add_operator_dialog: bool,
    pub pending_operator_id: String,
    pub pending_operator_license: Option<LicenseClass>,

    pub show_delete_operator_dialog: bool,
    pub pending_delete_operator_id: Option<String>,

    pub persistence_status: String,

    // =====================================================================
    // PER-DEMOD OPERATOR PREFERENCES
    // =====================================================================
    pub demod_preferences: DemodPreferenceSetFile,

    // =====================================================================
    // BOOKMARKS
    // =====================================================================
    pub bookmarks: Vec<crate::persistence::BookmarkFile>,
    pub selected_bookmark_id: Option<String>,
    pub default_bookmark_id: Option<String>,
    pub auto_apply_default_bookmark_on_acquire: bool,

    pub show_add_bookmark_dialog: bool,
    pub pending_bookmark_name: String,
    pub pending_bookmark_notes: String,
    pub bookmark_status: String,
    pub pending_apply_default_bookmark: bool,

    // =====================================================================
    // SOURCE
    // =====================================================================
    pub source_control: SourceControlState,
    pub source_capabilities: SourceCapabilities,
    pub radio_capabilities: RadioCapabilities,
    /// Latest read-only telemetry from the active source.
    /// Empty (`SourceStatus::default()`) when the source does not report status.
    pub source_status: SourceStatus,

    /// Persisted source-control settings keyed by radio ID string.
    /// Mirrors `OperatorSettingsFile::source_control_preferences`.
    pub source_control_preferences: HashMap<String, SourceControlState>,

    /// When `true`, `draw_source_control_panel` should re-send all source
    /// control values to the server (used after applying saved preferences
    /// on radio acquire).
    pub pending_apply_source_control: bool,

    // =====================================================================
    // TX TUNE TEST (client-local; never persisted; never sent to server)
    // =====================================================================
    /// Whether the operator has armed the TX tune test checkbox.
    ///
    /// Always starts `false`. Never persisted. Cleared automatically when
    /// "Measure SWR" is clicked.
    pub tx_tune_armed: bool,

    /// True while a TX tune test request has been sent and no result has
    /// arrived yet.  Used to disable the "Measure SWR" button and show a
    /// running indicator.  Never persisted.
    pub tx_tune_running: bool,

    /// Cached result from the most recent TX tune test measurement.
    /// `status = NotRun` until an actual tune test is executed.
    pub last_tx_tune_result: TxTuneResult,
}

impl Default for UiState {
    fn default() -> Self {
        let mut state = Self {
            // =================================================================
            // RADIO STATE
            // =================================================================
            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            demod_mode: DemodMode::Wfm,
            sideband: Sideband::Lsb,

            demod_preferences: DemodPreferenceSetFile::default(),
            pitch_hz: 0.0,
            filter_bandwidth_hz: 3000.0,
            deemphasis_mode: DeemphasisMode::Off,
            input_sample_rate_hz: 0.0,

            // =================================================================
            // RADIO-DERIVED UI STATE
            // =================================================================
            last_demod_mode_for_controls: None,
            pending_apply_mode_controls: false,

            // =================================================================
            // UI RUNTIME / HELPER STATE
            // =================================================================
            filter_bw_debounce: DebounceState::new(0.0),

            pitch_debounce: DebounceState::new(0.0),

            // =================================================================
            // CONNECTION / SERVER STATE
            // =================================================================
            rigflow_server_ip: "192.168.0.225".to_string(),
            rigflow_server_ws_port: 9000,
            rigflow_server_udp_port: 9001,
            udp_listen_port: 0,

            server_connected: false,
            radio_acquired: false,

            server_status: "no server".to_string(),

            available_radios: Vec::new(),
            selected_radio_id: None,

            // =================================================================
            // UI STATE
            // =================================================================
            runtime_error: String::new(),
            selected_license: None,
            spectrum_zoom_x: 1.0,

            // =================================================================
            // WATERFALL / DISPLAY
            // =================================================================
            manual_waterfall_top_db: -35.0,
            manual_waterfall_range_db: 80.0,

            adaptive_waterfall_normalization: true,
            adaptive_top_db_estimate: -35.0,
            adaptive_floor_db_estimate: -140.0,
            adaptive_range_db_estimate: 100.0,

            display_zoom: 1.0,

            // =================================================================
            // OPERATOR / PERSISTENCE
            // =================================================================
            operator_id: String::new(),
            known_operator_ids: Vec::new(),

            show_add_operator_dialog: false,
            pending_operator_id: String::new(),
            pending_operator_license: None,

            show_delete_operator_dialog: false,
            pending_delete_operator_id: None,

            persistence_status: String::new(),

            // =================================================================
            // BOOKMARKS
            // =================================================================
            bookmarks: Vec::new(),
            selected_bookmark_id: None,
            default_bookmark_id: None,
            auto_apply_default_bookmark_on_acquire: false,

            show_add_bookmark_dialog: false,
            pending_bookmark_name: String::new(),
            pending_bookmark_notes: String::new(),
            bookmark_status: String::new(),
            pending_apply_default_bookmark: false,

            // =====================================================================
            // SOURCE
            // =====================================================================
            source_control: SourceControlState::default(),
            source_capabilities: SourceCapabilities::none(),
            radio_capabilities: RadioCapabilities::default(),
            source_status: SourceStatus::default(),
            source_control_preferences: HashMap::new(),
            pending_apply_source_control: false,

            tx_tune_armed: false,
            tx_tune_running: false,
            last_tx_tune_result: TxTuneResult::default(),
        };

        let prefs = state.demod_preferences.get(state.demod_mode);

        state.filter_bandwidth_hz = prefs.filter_bandwidth_hz;
        state.pitch_hz = prefs.pitch_hz;

        state.filter_bw_debounce = DebounceState::new(state.filter_bandwidth_hz);
        state.pitch_debounce = DebounceState::new(state.pitch_hz);

        state
    }
}

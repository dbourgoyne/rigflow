use std::time::Instant;
use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::{DemodMode, Sideband};

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

    /// SSB pitch offset (Hz)
    pub ssb_pitch_hz: f32,

    /// CW pitch (Hz)
    pub cw_pitch_hz: f32,

    /// Audio filter bandwidth (Hz)
    pub filter_bandwidth_hz: f32,

    /// Input sample rate from SDR source (Hz)
    pub input_sample_rate_hz: f32,

    // =====================================================================
    // RADIO-DERIVED UI STATE
    // (depends on demod mode or radio state)
    // =====================================================================

    /// Tracks last demod mode for applying defaults (e.g. bandwidth)
    pub last_demod_mode_for_bw: Option<DemodMode>,

    // =====================================================================
    // UI RUNTIME / HELPER STATE (non-persistent, non-radio)
    // =====================================================================

    /// Last filter bandwidth value sent to server (for debounce)
    pub last_filter_bw_sent_hz: f32,
    pub filter_bw_debounce: DebounceState,

    /// Timestamp of last filter bandwidth send
    pub last_filter_bw_send_time: std::time::Instant,

    /// Last SSB pitch sent to server
    pub last_ssb_pitch_sent_hz: f32,
    pub ssb_pitch_debounce: DebounceState,

    /// Timestamp of last SSB pitch send
    pub last_ssb_pitch_send_time: std::time::Instant,

    /// Last CW pitch sent to server
    pub last_cw_pitch_sent_hz: f32,
    pub cw_pitch_debounce: DebounceState,

    /// Timestamp of last CW pitch send
    pub last_cw_pitch_send_time: std::time::Instant,

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

    pub display_top_db: f32,
    pub display_range_db: f32,

    pub adaptive_waterfall_normalization: bool,

    pub adaptive_top_db_estimate: f32,
    pub adaptive_floor_db_estimate: f32,

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
}


impl Default for UiState {
    fn default() -> Self {
        Self {
            // =================================================================
            // RADIO STATE
            // =================================================================

            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            demod_mode: DemodMode::Wfm,
            sideband: Sideband::Lsb,

            ssb_pitch_hz: 0.0,
            cw_pitch_hz: 600.0,

            filter_bandwidth_hz: 3000.0,
            input_sample_rate_hz: 0.0,

            // =================================================================
            // RADIO-DERIVED UI STATE
            // =================================================================

            last_demod_mode_for_bw: None,

            // =================================================================
            // UI RUNTIME / HELPER STATE
            // =================================================================

	    last_filter_bw_sent_hz: 0.0,
            last_filter_bw_send_time: std::time::Instant::now(),
	    filter_bw_debounce: DebounceState::new(0.0),

            last_ssb_pitch_sent_hz: 0.0,
            last_ssb_pitch_send_time: std::time::Instant::now(),
	    ssb_pitch_debounce: DebounceState::new(0.0),

            last_cw_pitch_sent_hz: 0.0,
            last_cw_pitch_send_time: std::time::Instant::now(),
	    cw_pitch_debounce: DebounceState::new(600.0),

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

            display_top_db: -35.0,
            display_range_db: 70.0,

            adaptive_waterfall_normalization: true,
            adaptive_top_db_estimate: -35.0,
            adaptive_floor_db_estimate: -105.0,

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
        }
    }
}

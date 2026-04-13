use crate::ui::om_bands::LicenseClass;
use rigflow_core::dsp::modes::{DemodMode, Sideband};

/// Central UI state shared between:
/// - egui rendering thread
/// - networking (WebSocket)
/// - media runtime
///
/// This struct is intentionally simple (plain data) and is typically
/// wrapped in `Arc<Mutex<UiState>>`.
#[derive(Debug, Clone)]
pub struct UiState {
    // ---------------------------------------------------------------------
    // Radio tuning state
    // ---------------------------------------------------------------------

    /// Center frequency (LO), in Hz
    pub center_freq_hz: f32,

    /// Target tuned frequency, in Hz
    pub target_freq_hz: f32,

    /// Current sideband
    pub sideband: Sideband,

    /// Current demodulation mode
    pub demod_mode: DemodMode,

    /// SSB pitch offset (Hz)
    pub ssb_pitch_hz: f32,

    /// Input sample rate from SDR source (Hz)
    pub input_sample_rate_hz: f32,

    // ---------------------------------------------------------------------
    // UI / interaction state
    // ---------------------------------------------------------------------

    /// Last runtime error message to display in UI
    pub runtime_error: String,

    /// Currently hovered digit in LO widget (for interaction feedback)
    //pub hovered_center_freq_digit: Option<usize>,

    /// Selected amateur radio license class (for band overlays)
    pub selected_license: Option<LicenseClass>,

    /// Horizontal zoom level for spectrum view
    pub spectrum_zoom_x: f32,

    /// Whether zoom slider is currently being dragged
    //pub zoom_slider_dragging: bool,

    /// Whether the "Rigflow Server" menu is expanded
    //pub rigflow_server_menu_expanded: bool,

    /// Whether the server IP field is actively being edited
    //pub editing_server_ip: bool,

    // ---------------------------------------------------------------------
    // Server connection state
    // ---------------------------------------------------------------------

    /// WebSocket port for rigflow server
    pub rigflow_server_ws_port: u16,

    /// UDP port for rigflow media plane
    pub rigflow_server_udp_port: u16,

    /// Local UDP port this client is listening on
    pub udp_listen_port: u16,

    /// List of radios reported by the server
    pub available_radios: Vec<rigflow_protocol::radio_control::RadioInfo>,

    /// Currently selected radio ID (if any)
    pub selected_radio_id: Option<String>,

    /// Whether the client is currently connected to a server
    pub server_connected: bool,

    /// Human-readable server status string (UI display)
    pub server_status: String,

    /// Server IP address entered by the user
    pub rigflow_server_ip: String,

    /// Whether a radio is currently acquired (lease held)
    pub radio_acquired: bool,

    // ---------------------------------------------------------------------
    // Waterfall Display
    // ---------------------------------------------------------------------

    /// Waterfall/spectrum display top level in dB.
    pub display_top_db: f32,

    /// Waterfall/spectrum display range in dB.
    pub display_range_db: f32,

    /// Whether display scaling is controlled automatically.
    pub adaptive_waterfall_normalization: bool,

    /// Waterfall/spectrum display zoom
    pub display_zoom: f32,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            // --- Radio defaults ------------------------------------------

            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            sideband: Sideband::Lsb,
            demod_mode: DemodMode::Wfm,
            ssb_pitch_hz: 0.0,
            input_sample_rate_hz: 0.0,

            // --- UI defaults ---------------------------------------------

            runtime_error: String::new(),
            //hovered_center_freq_digit: None,
            selected_license: None,
            spectrum_zoom_x: 1.0,
            //zoom_slider_dragging: false,
            //rigflow_server_menu_expanded: false,
            //editing_server_ip: false,

            // --- Server defaults -----------------------------------------

            rigflow_server_ws_port: 9000,
            rigflow_server_udp_port: 9001,
            udp_listen_port: 0,
            available_radios: Vec::new(),
            selected_radio_id: None,
            server_connected: false,
            server_status: "no server".to_string(),

            // Default dev/test IP — consider making configurable later
            rigflow_server_ip: "192.168.0.225".to_string(),

            radio_acquired: false,

	    // --- Waterfall Display defaults ------------------------------
	    display_top_db: -35.0,
            display_range_db: 70.0,
            adaptive_waterfall_normalization: false,
	    display_zoom: 1.0,
        }
    }
}

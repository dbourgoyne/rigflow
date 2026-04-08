use crate::app::om_bands::LicenseClass;

#[derive(Debug, Clone)]
pub struct UiState {
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub sideband: String,
    pub demod_mode: String,
    pub ssb_pitch_hz: f32,
    pub input_sample_rate_hz: f32,
    pub runtime_status: String,
    pub hovered_center_freq_digit: Option<usize>,
    pub selected_license: LicenseClass,
    pub spectrum_zoom_x: f32,
    pub zoom_slider_dragging: bool,
    pub rigflow_server_menu_expanded: bool,
    pub editing_server_ip: bool,
    pub rigflow_server_ws_port: u16,
    pub rigflow_server_udp_port: u16,
    pub udp_listen_port: u16,
    pub available_radios: Vec<rigflow_protocol::radio_control::RadioInfo>,
    pub selected_radio_id: Option<String>,
    pub server_connected: bool,
    pub server_status: String,
    pub rigflow_server_ip: String,
    pub radio_acquired: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            center_freq_hz: 0.0,
            target_freq_hz: 0.0,
            sideband: "lsb".to_string(),
            demod_mode: "wfm".to_string(),
            ssb_pitch_hz: 0.0,
            input_sample_rate_hz: 0.0,
            runtime_status: String::new(),
            hovered_center_freq_digit: None,
            selected_license: LicenseClass::General,
            spectrum_zoom_x: 1.0,
            zoom_slider_dragging: false,
	    rigflow_server_menu_expanded: false,
	    editing_server_ip: false,
	    rigflow_server_ws_port: 9000,
	    rigflow_server_udp_port: 9001,
	    udp_listen_port: 0,
	    available_radios: Vec::new(),
	    selected_radio_id: None,
	    server_connected: false,
	    server_status: "no server".to_string(),
	    rigflow_server_ip: "192.168.0.225".to_string(),
	    radio_acquired: false,
        }
    }
}

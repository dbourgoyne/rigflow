use crate::app::om_bands::LicenseClass;

#[derive(Debug, Clone)]
pub struct UiState {
    pub center_freq_hz: f32,
    pub target_freq_hz: f32,
    pub sideband: String,
    pub demod_mode: String,
    pub ssb_pitch_hz: f32,
    pub input_sample_rate_hz: f32,
    pub waterfall_bins: usize,
    pub audio_sample_rate_hz: f32,
    pub audio_format: String,
    pub waterfall_frame_rate_hz: f32,
    pub status: String,
    pub hovered_center_freq_digit: Option<usize>,
    pub selected_license: LicenseClass,
    pub spectrum_zoom_x: f32,
    pub zoom_slider_dragging: bool,
    pub radio_acquired: bool,
    pub available_radios: Vec<rigflow_protocol::radio_control::RadioInfo>,
    pub acquired_radio_id: Option<rigflow_core::radio::RadioId>,
    pub lease_id: Option<rigflow_core::radio::LeaseId>,
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
            waterfall_bins: 0,
            audio_sample_rate_hz: 0.0,
            audio_format: "unknown".to_string(),
            waterfall_frame_rate_hz: 0.0,
            status: "starting".to_string(),
            hovered_center_freq_digit: None,
            selected_license: LicenseClass::General,
            spectrum_zoom_x: 1.0,
            zoom_slider_dragging: false,
            radio_acquired: false,
            available_radios: Vec::new(),
            acquired_radio_id: None,
            lease_id: None,
        }
    }
}

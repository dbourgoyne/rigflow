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
        }
    }
}


use crate::app::state::UiState;

pub fn build_window_title(state: &UiState) -> String {
    format!(
        "Rigflow | Mode: {} | CF: {:.0} Hz | TF: {:.0} Hz | Pitch: {:+.0} Hz | SB: {} | Audio: {} @ {:.0} Hz | WF: {} bins @ {:.1} fps | {}",
        state.demod_mode.to_uppercase(),
        state.center_freq_hz,
        state.target_freq_hz,
        state.ssb_pitch_hz,
        state.sideband.to_uppercase(),
        state.audio_format,
        state.audio_sample_rate_hz,
        state.waterfall_bins,
        state.waterfall_frame_rate_hz,
        state.status,
    )
}

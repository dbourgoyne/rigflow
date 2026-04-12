use crate::{
    app::{
        state::UiState,
    },
};

pub fn visible_span_hz(state: &UiState) -> f32 {
    if state.input_sample_rate_hz <= 0.0 {
        0.0
    } else {
        state.input_sample_rate_hz / state.spectrum_zoom_x.clamp(1.0, 10.0)
    }
}

pub fn visible_left_hz(state: &UiState) -> f32 {
    state.center_freq_hz - visible_span_hz(state) * 0.5
}

pub fn visible_right_hz(state: &UiState) -> f32 {
    state.center_freq_hz + visible_span_hz(state) * 0.5
}

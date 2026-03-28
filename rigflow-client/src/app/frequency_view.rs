use crate::{
    app::{
        layout::{SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
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

pub fn freq_to_plot_x(freq_hz: f32, state: &UiState) -> Option<usize> {
    let left_hz = visible_left_hz(state);
    let right_hz = visible_right_hz(state);

    if right_hz <= left_hz || freq_hz < left_hz || freq_hz > right_hz {
        return None;
    }

    let frac = (freq_hz - left_hz) / (right_hz - left_hz);
    let plot_width = (SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0) as f32;
    let x = SPECTRUM_PLOT_X0 as f32 + frac * plot_width;

    Some(x.round() as usize)
}

/*
pub fn plot_x_to_freq_hz(x: usize, state: &UiState) -> Option<f32> {
    if !(SPECTRUM_PLOT_X0..SPECTRUM_PLOT_X1).contains(&x) {
        return None;
    }

    let span = visible_span_hz(state);
    if span <= 0.0 {
        return None;
    }

    let frac = (x - SPECTRUM_PLOT_X0) as f32 / (SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0) as f32;
    Some(visible_left_hz(state) + frac * span)
}
*/

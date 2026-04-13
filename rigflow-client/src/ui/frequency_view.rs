use crate::ui::state::UiState;

/// Compute the visible frequency span (Hz) of the spectrum view.
///
/// This is derived from:
/// - SDR input sample rate (full bandwidth)
/// - current horizontal zoom level
///
/// Behavior:
/// - returns 0.0 if sample rate is not yet known
/// - clamps zoom to a safe range to avoid extreme values
pub fn visible_span_hz(state: &UiState) -> f32 {
    if state.input_sample_rate_hz <= 0.0 {
        return 0.0;
    }

    let zoom = state.spectrum_zoom_x.clamp(1.0, 10.0);
    state.input_sample_rate_hz / zoom
}

/// Compute the left edge (minimum frequency) of the visible spectrum.
pub fn visible_left_hz(state: &UiState) -> f32 {
    let span = visible_span_hz(state);
    state.center_freq_hz - span * 0.5
}

/// Compute the right edge (maximum frequency) of the visible spectrum.
pub fn visible_right_hz(state: &UiState) -> f32 {
    let span = visible_span_hz(state);
    state.center_freq_hz + span * 0.5
}

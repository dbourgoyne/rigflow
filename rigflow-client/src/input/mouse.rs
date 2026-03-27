use minifb::{MouseButton, MouseMode, Window};

use crate::{
    app::{
        layout::{SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1, SPECTRUM_PLOT_WIDTH},
        state::UiState,
    },
    input::keyboard::UiAction,
};

pub fn collect_mouse_actions(
    window: &Window,
    state: &UiState,
) -> Vec<UiAction> {
    let mut actions = Vec::new();

    if !window.get_mouse_down(MouseButton::Left) {
        return actions;
    }

    let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) else {
        return actions;
    };

    let x = mx as usize;
    let _y = my as usize;

    if !(SPECTRUM_PLOT_X0..SPECTRUM_PLOT_X1).contains(&x) {
        return actions;
    }

    let Some(freq_hz) = plot_x_to_frequency_hz(x, state) else {
        return actions;
    };

    actions.push(UiAction::SetTargetFrequency(freq_hz));

    actions
}

fn plot_x_to_frequency_hz(x: usize, state: &UiState) -> Option<f32> {
    if state.input_sample_rate_hz <= 0.0 || SPECTRUM_PLOT_WIDTH == 0 {
        return None;
    }

    let plot_x = x.checked_sub(SPECTRUM_PLOT_X0)?;
    let frac = plot_x as f32 / SPECTRUM_PLOT_WIDTH as f32;

    let left_hz = state.center_freq_hz - state.input_sample_rate_hz * 0.5;
    let freq_hz = left_hz + frac * state.input_sample_rate_hz;

    Some(freq_hz)
}

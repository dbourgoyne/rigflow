use minifb::{Key, MouseButton, MouseMode, Window};

use crate::{
    app::{
        layout::{
	    SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1, SPECTRUM_PLOT_WIDTH,
	    FREQ_WIDGET_X, FREQ_WIDGET_Y,
	    HEIGHT, WATERFALL_TOP, WIDTH,
	    ZOOM_SLIDER_X0, ZOOM_SLIDER_X1, ZOOM_SLIDER_Y0, ZOOM_SLIDER_Y1,
	},
        state::UiState,
    },
    input::keyboard::UiAction,
};
use crate::app::frequency_view::plot_x_to_freq_hz;

const WATERFALL_TUNE_STEP_HZ: f32 = 1_000.0;
const WATERFALL_TUNE_STEP_FAST_HZ: f32 = 10_000.0;

#[derive(Debug, Default, Clone, Copy)]
pub struct MouseClickState {
    pub prev_left_down: bool,
}

pub fn collect_waterfall_wheel_actions(
    window: &Window,
    state: &UiState,
) -> Vec<UiAction> {
    let mut actions = Vec::new();

    let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) else {
        return actions;
    };

    if !mouse_over_waterfall(mx, my) {
        return actions;
    }

    let (_, wheel_y) = window.get_scroll_wheel().unwrap_or((0.0, 0.0));
    if wheel_y == 0.0 {
        return actions;
    }

    let step_hz = if window.is_key_down(Key::LeftShift) || window.is_key_down(Key::RightShift) {
        WATERFALL_TUNE_STEP_FAST_HZ
    } else {
        WATERFALL_TUNE_STEP_HZ
    };

    let dir = wheel_y.signum();
    let next = state.target_freq_hz - dir * step_hz;

    actions.push(UiAction::SetTargetFrequency(next));

    actions
}

fn mouse_over_waterfall(mx: f32, my: f32) -> bool {
    let x = mx as isize;
    let y = my as isize;

    x >= 0
        && x < WIDTH as isize
        && y >= WATERFALL_TOP as isize
        && y < HEIGHT as isize
}

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

    let Some(freq_hz) = plot_x_to_freq_hz(x, state) else {
        return actions;
    };

    actions.push(UiAction::SetTargetFrequency(freq_hz));

    actions
}

pub fn update_zoom_slider(window: &Window, state: &mut UiState) {
    let Some((mx, my)) = window.get_mouse_pos(MouseMode::Discard) else {
        state.zoom_slider_dragging = false;
        return;
    };

    let x = mx as isize;
    let y = my as isize;

    let over_slider =
        x >= ZOOM_SLIDER_X0 as isize
            && x <= ZOOM_SLIDER_X1 as isize
            && y >= ZOOM_SLIDER_Y0 as isize
            && y <= ZOOM_SLIDER_Y1 as isize;

    if window.get_mouse_down(MouseButton::Left) {
        if over_slider || state.zoom_slider_dragging {
            state.zoom_slider_dragging = true;
            state.spectrum_zoom_x = zoom_from_slider_y(my).clamp(1.0, 10.0);
        }
    } else {
        state.zoom_slider_dragging = false;
    }
}

fn zoom_from_slider_y(mouse_y: f32) -> f32 {
    let y = mouse_y.clamp(ZOOM_SLIDER_Y0 as f32, ZOOM_SLIDER_Y1 as f32);
    let t = (ZOOM_SLIDER_Y1 as f32 - y) / (ZOOM_SLIDER_Y1 - ZOOM_SLIDER_Y0) as f32;
    1.0 + t * 9.0
}

pub fn poll_left_click(window: &Window, click_state: &mut MouseClickState) -> bool {
    let left_down = window.get_mouse_down(MouseButton::Left);
    let clicked = left_down && !click_state.prev_left_down;
    click_state.prev_left_down = left_down;
    clicked
}

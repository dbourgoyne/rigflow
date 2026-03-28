use crate::{
    app::{
        layout::{
            CONTROL_PANEL_X0, CONTROL_PANEL_X1, HEIGHT, ZOOM_SLIDER_X0, ZOOM_SLIDER_X1,
            ZOOM_SLIDER_Y0, ZOOM_SLIDER_Y1,
        },
        state::UiState,
    },
    render::color::{
        COLOR_AXIS, COLOR_BACKGROUND,
    },
};

pub fn draw_control_panel(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    // Panel background
    for y in 0..HEIGHT {
        let row = y * fb_width;
        for x in CONTROL_PANEL_X0..CONTROL_PANEL_X1 {
            buffer[row + x] = COLOR_BACKGROUND;
        }
    }

    // Left border of panel
    for y in 0..HEIGHT {
        buffer[y * fb_width + CONTROL_PANEL_X0] = COLOR_AXIS;
    }

    draw_zoom_slider(buffer, fb_width, state);
}

fn draw_zoom_slider(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    let track_x = (ZOOM_SLIDER_X0 + ZOOM_SLIDER_X1) / 2;

    // Track
    for y in ZOOM_SLIDER_Y0..ZOOM_SLIDER_Y1 {
        buffer[y * fb_width + track_x] = 0x00808080;
    }

    // Knob position: 1x at bottom, 10x at top
    let zoom = state.spectrum_zoom_x.clamp(1.0, 10.0);
    let t = (zoom - 1.0) / 9.0;
    let knob_y =
        ZOOM_SLIDER_Y1 as f32 - t * (ZOOM_SLIDER_Y1 - ZOOM_SLIDER_Y0) as f32;

    let knob_y = knob_y.round() as isize;

    for y in (knob_y - 4)..=(knob_y + 4) {
        if y < ZOOM_SLIDER_Y0 as isize || y >= ZOOM_SLIDER_Y1 as isize {
            continue;
        }
        let row = y as usize * fb_width;
        for x in ZOOM_SLIDER_X0..=ZOOM_SLIDER_X1 {
            buffer[row + x] = 0x00d0d0d0;
        }
    }
}

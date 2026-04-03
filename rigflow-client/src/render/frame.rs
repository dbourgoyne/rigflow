use crate::{
    app::{
        layout::{
            WIDTH, HEIGHT,
            LEFT_PANE_WIDTH, MAIN_CONTENT_X0,
            SPECTRUM_DB_MAX, SPECTRUM_DB_MIN, SPECTRUM_HEIGHT,
            WATERFALL_TOP,
            FREQ_WIDGET_X, FREQ_WIDGET_Y,
        },
        state::UiState,
    },
    render::{
        color::COLOR_BACKGROUND,
        left_panel::draw_left_pane,
        spectrum::{
            draw_frequency_overlay, draw_passband, draw_separator,
            draw_spectrum_axes_and_labels, draw_spectrum_background,
            draw_spectrum_grid, draw_spectrum_trace, draw_tuning_marker,
        },
    },
};

use crate::{
    widgets::frequency_widget::{draw_center_frequency_widget, FrequencyWidgetLayout},
};

use crate::render::bands::draw_band_strip;
use crate::render::om_band::draw_om_band_strip;
use crate::render::control_panel::draw_control_panel;

pub fn render_frame(
    display_buffer: &mut [u32],
    waterfall_buffer: &[u32],
    spectrum_db: &[f32],
    state: &UiState,
) {
    if display_buffer.len() != WIDTH * HEIGHT {
        return;
    }

    // Start from the latest waterfall image if available.
    if waterfall_buffer.len() == display_buffer.len() {
	println!(
	    "FRAME copy: waterfall_buffer len={} display_buffer len={}",
	    waterfall_buffer.len(),
	    display_buffer.len()
	);

        display_buffer.copy_from_slice(waterfall_buffer);
    } else {
        display_buffer.fill(COLOR_BACKGROUND);
    }

    // Explicitly clear the left pane so no stale waterfall pixels remain there.
    clear_left_pane_region(display_buffer);

    // Clear only the radio-content spectrum region, not the full width.
    clear_spectrum_region(display_buffer);

    draw_spectrum_background(display_buffer, WIDTH, SPECTRUM_HEIGHT);
    draw_passband(display_buffer, WIDTH, state);
    draw_spectrum_grid(
        display_buffer,
        WIDTH,
        SPECTRUM_HEIGHT,
        SPECTRUM_DB_MIN,
        SPECTRUM_DB_MAX,
    );
    draw_spectrum_axes_and_labels(
        display_buffer,
        WIDTH,
        state,
    );
    draw_spectrum_trace(
        display_buffer,
        WIDTH,
        spectrum_db,
        state,
    );
    draw_tuning_marker(
        display_buffer,
        WIDTH,
        HEIGHT,
        WATERFALL_TOP,
        state,
    );
    draw_om_band_strip(display_buffer, WIDTH, state);
    draw_band_strip(display_buffer, WIDTH, state);
    draw_frequency_overlay(display_buffer, WIDTH, state);

    draw_center_frequency_widget(
        display_buffer,
        WIDTH,
        FrequencyWidgetLayout {
            x: FREQ_WIDGET_X,
            y: FREQ_WIDGET_Y,
        },
        state,
        state.hovered_center_freq_digit,
    );

    draw_separator(display_buffer, WIDTH, WATERFALL_TOP.saturating_sub(1));
    draw_control_panel(display_buffer, WIDTH, state);

    // Draw the left pane last so it stays visible on top of any copied waterfall pixels.
    draw_left_pane(display_buffer, WIDTH, HEIGHT, state);
}

fn clear_spectrum_region(display_buffer: &mut [u32]) {
    let rows = SPECTRUM_HEIGHT.min(HEIGHT);

    for y in 0..rows {
        let row_start = y * WIDTH;
        let start = row_start + MAIN_CONTENT_X0;
        let end = row_start + WIDTH;
        display_buffer[start..end].fill(COLOR_BACKGROUND);
    }
}

fn clear_left_pane_region(display_buffer: &mut [u32]) {
    for y in 0..HEIGHT {
        let row_start = y * WIDTH;
        let end = row_start + LEFT_PANE_WIDTH.min(WIDTH);
        display_buffer[row_start..end].fill(COLOR_BACKGROUND);
    }
}


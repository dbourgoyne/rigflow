use crate::{
    app::{
        layout::{
            HEIGHT, SPECTRUM_DB_MAX, SPECTRUM_DB_MIN, SPECTRUM_HEIGHT, WATERFALL_TOP, WIDTH,
        },
        state::UiState,
    },
    render::{
        color::COLOR_BACKGROUND,
        spectrum::{
            draw_frequency_overlay, draw_passband, draw_separator,
	    draw_spectrum_axes_and_labels, draw_spectrum_background,
	    draw_spectrum_grid, draw_spectrum_trace, draw_tuning_marker,
        },
    },
};

pub fn render_frame(
    display_buffer: &mut [u32],
    waterfall_buffer: &[u32],
    spectrum_db: &[f32],
    state: &UiState,
) {
    if display_buffer.len() != WIDTH * HEIGHT {
        return;
    }

    // Start from the latest waterfall image.
    if waterfall_buffer.len() == display_buffer.len() {
        display_buffer.copy_from_slice(waterfall_buffer);
    } else {
        display_buffer.fill(COLOR_BACKGROUND);
    }

    // Clear the spectrum region explicitly so it is always redrawn fresh.
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
    );
    draw_tuning_marker(
	display_buffer,
	WIDTH,
	HEIGHT,
	WATERFALL_TOP,
	state,
    );
    draw_frequency_overlay(display_buffer, WIDTH, state);
    draw_separator(display_buffer, WIDTH, WATERFALL_TOP.saturating_sub(1));
}

fn clear_spectrum_region(display_buffer: &mut [u32]) {
    let rows = SPECTRUM_HEIGHT.min(HEIGHT);
    for y in 0..rows {
        let start = y * WIDTH;
        let end = start + WIDTH;
        display_buffer[start..end].fill(COLOR_BACKGROUND);
    }
}

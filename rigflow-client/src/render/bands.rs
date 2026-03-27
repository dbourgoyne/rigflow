use crate::{
    app::{
        bands::band_for_frequency,
        layout::{BAND_STRIP_Y0, BAND_STRIP_Y1, SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
        state::UiState,
    },
    render::text::draw_text,
};

pub fn draw_band_strip(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    let Some(band) = band_for_frequency(state.center_freq_hz) else {
        return;
    };

    let x0 = SPECTRUM_PLOT_X0;
    let x1 = SPECTRUM_PLOT_X1.saturating_sub(1);

    if x0 >= x1 {
        return;
    }

    for y in BAND_STRIP_Y0..BAND_STRIP_Y1 {
        let row = y * fb_width;
        for x in x0..=x1 {
            buffer[row + x] = band.color;
        }
    }

    let label = format!("{} ({})", band.name, band.preferred_demod);

    let text_x = x0 + 6;
    let text_y = BAND_STRIP_Y0 + 7;

    draw_text(buffer, fb_width, text_x, text_y, &label, 0x00f0f0f0);
}

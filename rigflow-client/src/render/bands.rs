use crate::{
    app::{
        bands::{RadioBand, RADIO_BANDS},
        layout::{BAND_STRIP_Y0, BAND_STRIP_Y1, SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
        state::UiState,
    },
    render::text::draw_text,
};
use crate::render::spectrum::{visible_left_hz, visible_right_hz};

pub fn draw_band_strip(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    if state.input_sample_rate_hz <= 0.0 {
        return;
    }

    let left_hz = visible_left_hz(state);
    let right_hz = visible_right_hz(state);

    let mut active_band: Option<(&'static RadioBand, usize, usize)> = None;

    for band in RADIO_BANDS {
        let visible_start_hz = left_hz.max(band.start_hz);
        let visible_end_hz = right_hz.min(band.end_hz);

        if visible_start_hz >= visible_end_hz {
            continue;
        }

        let Some(mut x0) = freq_to_plot_x(visible_start_hz, state) else {
            continue;
        };
        let Some(mut x1) = freq_to_plot_x(visible_end_hz, state) else {
            continue;
        };

        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
        }

        x0 = x0.max(SPECTRUM_PLOT_X0);
        x1 = x1.min(SPECTRUM_PLOT_X1.saturating_sub(1));

        if x0 >= x1 {
            continue;
        }

        for y in BAND_STRIP_Y0..BAND_STRIP_Y1 {
            let row = y * fb_width;
            for x in x0..=x1 {
                buffer[row + x] = band.color;
            }
        }

        // Choose the band containing the center frequency as the one to label.
        if state.center_freq_hz >= band.start_hz && state.center_freq_hz <= band.end_hz {
            active_band = Some((band, x0, x1));
        }
    }

    if let Some((band, x0, x1)) = active_band {
        let label = format!("{} ({})", band.name, band.preferred_demod);

        let text_width = label.len() * 6; // draw_text uses 5 px + 1 px spacing
        let band_width = x1.saturating_sub(x0);
        let text_x = if band_width > text_width {
            x0 + (band_width - text_width) / 2
        } else {
            x0 + 4
        };

        let text_y = BAND_STRIP_Y0 + 7;
        draw_text(buffer, fb_width, text_x, text_y, &label, 0x00f0f0f0);
    }
}

fn freq_to_plot_x(freq_hz: f32, state: &UiState) -> Option<usize> {
    if state.input_sample_rate_hz <= 0.0 {
        return None;
    }

    let left_hz = visible_left_hz(state);
    let right_hz = visible_right_hz(state);

    if freq_hz < left_hz || freq_hz > right_hz {
        return None;
    }

    let frac = (freq_hz - left_hz) / (right_hz - left_hz);
    let plot_width = (SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0) as f32;
    let x = SPECTRUM_PLOT_X0 as f32 + frac * plot_width;

    Some(x.round() as usize)
}

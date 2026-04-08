use crate::{
    app::{
        layout::{SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
        state::UiState,
    },
    app::spectrum_utils::color_map,
};

pub fn draw_row(
    buffer: &mut [u32],
    waterfall_width: usize,
    waterfall_height: usize,
    row: &[u8],
) {
    if buffer.len() != waterfall_width * waterfall_height || waterfall_width == 0 || waterfall_height == 0 {
        return;
    }

    // scroll everything up by one row
    buffer.copy_within(waterfall_width.., 0);

    // clear/write the last row
    let last_row_start = (waterfall_height - 1) * waterfall_width;

    for x in 0..waterfall_width {
        let src_idx = x * row.len() / waterfall_width;
        let color = color_map(row[src_idx]);
        buffer[last_row_start + x] = color;
    }
}

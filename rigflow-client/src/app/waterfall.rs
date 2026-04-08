use crate::{
    app::{
        layout::{SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
        state::UiState,
    },
    app::spectrum_utils::color_map,
};

pub fn draw_row(
    buffer: &mut [u32],
    wf_width: usize,
    wf_height: usize,
    row: &[u8],
) {
    if wf_width == 0 || wf_height == 0 || row.is_empty() {
        return;
    }

    if buffer.len() != wf_width * wf_height {
        return;
    }

    // Scroll existing image up by one row.
    if wf_height > 1 {
        buffer.copy_within(wf_width.., 0);
    }

    // Draw new row into the final row.
    let last_row_start = (wf_height - 1) * wf_width;

    for x in 0..wf_width {
        let src_idx = x * row.len() / wf_width;
        let intensity = row[src_idx];
        buffer[last_row_start + x] = color_map(intensity);
    }
}

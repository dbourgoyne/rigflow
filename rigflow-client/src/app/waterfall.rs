use crate::{
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

    // Move existing rows DOWN by one row.
    if wf_height > 1 {
        buffer.copy_within(0..((wf_height - 1) * wf_width), wf_width);
    }

    // Write newest row at the TOP.
    for x in 0..wf_width {
        let src_idx = x * row.len() / wf_width;
        let intensity = row[src_idx];
        buffer[x] = color_map(intensity);
    }
}

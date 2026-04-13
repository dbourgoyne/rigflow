use crate::ui::spectrum_utils::{color_map, db_to_u8};

/// Draw a new waterfall row into the display buffer.
///
/// Behavior:
/// - existing rows are shifted down by one row
/// - newest row is written at the top
/// - incoming row is raw spectral dB data
/// - dB → intensity mapping happens here on the client
pub fn draw_row_db(
    buffer: &mut [u32],
    wf_width: usize,
    wf_height: usize,
    row_db: &[f32],
    top_db: f32,
    range_db: f32,
) {
    if wf_width == 0 || wf_height == 0 || row_db.is_empty() {
        return;
    }

    if buffer.len() != wf_width * wf_height {
        return;
    }

    if wf_height > 1 {
        let copy_len = (wf_height - 1) * wf_width;
        buffer.copy_within(0..copy_len, wf_width);
    }

    for x in 0..wf_width {
        let src_idx = x * row_db.len() / wf_width;
        let db = row_db[src_idx];
        let intensity = db_to_u8(db, top_db, range_db);
        buffer[x] = color_map(intensity);
    }
}

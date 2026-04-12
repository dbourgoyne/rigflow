use crate::app::spectrum_utils::color_map;

/// Draw a new waterfall row into the buffer.
///
/// Behavior:
/// - Existing rows are shifted down by one row
/// - The newest row is written at the top (row 0)
///
/// Parameters:
/// - `buffer`: ARGB pixel buffer (row-major, size = width * height)
/// - `wf_width`: width of the waterfall in pixels
/// - `wf_height`: height of the waterfall in pixels
/// - `row`: incoming intensity data (typically FFT magnitude, 0–255)
pub fn draw_row(
    buffer: &mut [u32],
    wf_width: usize,
    wf_height: usize,
    row: &[u8],
) {
    // --- Guard conditions -------------------------------------------------

    if wf_width == 0 || wf_height == 0 || row.is_empty() {
        return;
    }

    // Ensure buffer matches expected dimensions
    if buffer.len() != wf_width * wf_height {
        return;
    }

    // --- Scroll existing image down --------------------------------------

    // Move all rows down by one:
    // row N → row N+1
    // Bottom row is dropped
    if wf_height > 1 {
        let copy_len = (wf_height - 1) * wf_width;
        buffer.copy_within(0..copy_len, wf_width);
    }

    // --- Render new top row ----------------------------------------------

    // Map incoming row to display width and apply color map.
    //
    // Note:
    // - `row.len()` may differ from `wf_width`, so we resample by index mapping.
    // - This is a simple nearest-neighbor down/up-sample.
    for x in 0..wf_width {
        let src_idx = x * row.len() / wf_width;
        let intensity = row[src_idx];
        buffer[x] = color_map(intensity);
    }
}

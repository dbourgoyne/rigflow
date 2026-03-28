use crate::UiState;

pub fn draw_row(
    framebuffer: &mut [u32],
    row: &[u8],
    width: usize,
    height: usize,
    waterfall_top: usize,
    state: &UiState,
) {
    if width == 0 || height == 0 || waterfall_top >= height || row.is_empty() {
        return;
    }

    for y in (waterfall_top + 1..height).rev() {
        let dst = y * width;
        let src = (y - 1) * width;
        framebuffer.copy_within(src..src + width, dst);
    }

    let top = &mut framebuffer[waterfall_top * width..(waterfall_top + 1) * width];

    let zoom = state.spectrum_zoom_x.clamp(1.0, 10.0);
    let visible_bins = (row.len() as f32 / zoom).round().max(1.0) as usize;
    let start_bin = (row.len().saturating_sub(visible_bins)) / 2;

    for (x, pixel) in top.iter_mut().enumerate() {
        let src_x = start_bin + x * visible_bins / width;
        *pixel = crate::render::spectrum::color_map(row[src_x.min(row.len() - 1)]);
    }
}

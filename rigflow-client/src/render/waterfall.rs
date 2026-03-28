use crate::{
    app::{
        layout::{SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
        state::UiState,
    },
    render::spectrum::color_map,
};

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

    // Scroll the existing waterfall image down by one row.
    // This preserves older rows exactly as they were rendered, which matches
    // your requirement that zoom only affect incoming waterfall lines.
    for y in (waterfall_top + 1..height).rev() {
        let dst = y * width;
        let src = (y - 1) * width;
        framebuffer.copy_within(src..src + width, dst);
    }

    let top = &mut framebuffer[waterfall_top * width..(waterfall_top + 1) * width];

    // Clear the new top row before drawing the incoming line.
    top.fill(0x000000);

    let plot_x0 = SPECTRUM_PLOT_X0.min(width);
    let plot_x1 = SPECTRUM_PLOT_X1.min(width);
    let plot_width = plot_x1.saturating_sub(plot_x0);
    if plot_width == 0 {
        return;
    }

    let zoom = state.spectrum_zoom_x.clamp(1.0, 10.0);
    let visible_bins = (row.len() as f32 / zoom).round().max(1.0) as usize;
    let start_bin = row.len().saturating_sub(visible_bins) / 2;

    for x in plot_x0..plot_x1 {
        let plot_x = x - plot_x0;
        let src_x = start_bin + plot_x * visible_bins / plot_width;
        top[x] = color_map(row[src_x.min(row.len() - 1)]);
    }
}

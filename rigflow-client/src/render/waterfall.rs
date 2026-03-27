use crate::render::spectrum::color_map;

pub fn draw_row(
    framebuffer: &mut [u32],
    row: &[u8],
    width: usize,
    height: usize,
    waterfall_top: usize,
) {
    if width == 0 || height == 0 || waterfall_top >= height {
        return;
    }

    let waterfall_height = height - waterfall_top;
    if waterfall_height == 0 {
        return;
    }

    // Scroll existing waterfall downward by one row.
    for y in (waterfall_top + 1..height).rev() {
        let dst = y * width;
        let src = (y - 1) * width;
        framebuffer.copy_within(src..src + width, dst);
    }

    // Draw newest row at the top of the waterfall region.
    let top = &mut framebuffer[waterfall_top * width..(waterfall_top + 1) * width];

    if row.is_empty() {
        for pixel in top.iter_mut() {
            *pixel = 0x000000;
        }
        return;
    }

    for (x, pixel) in top.iter_mut().enumerate() {
        let src_x = x * row.len() / width;
        *pixel = color_map(row[src_x.min(row.len() - 1)]);
    }
}

use crate::ui::layout::SPECTRUM_SMOOTHING_ALPHA;

/// Map an 8-bit waterfall/spectrum intensity value to packed RGB.
///
/// Output format:
/// - 0xRRGGBB
///
/// Gradient:
/// - black   → blue
/// - blue    → cyan
/// - cyan    → yellow
/// - yellow  → red
pub fn color_map(v: u8) -> u32 {
    let x = v as f32 / 255.0;

    let (r, g, b) = if x < 0.25 {
        let t = x / 0.25;
        (0.0, 0.0, 255.0 * t)
    } else if x < 0.5 {
        let t = (x - 0.25) / 0.25;
        (0.0, 255.0 * t, 255.0)
    } else if x < 0.75 {
        let t = (x - 0.5) / 0.25;
        (255.0 * t, 255.0, 255.0 * (1.0 - t))
    } else {
        let t = (x - 0.75) / 0.25;
        (255.0, 255.0 * (1.0 - t), 0.0)
    };

    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Convert a dB value into an 8-bit display intensity.
///
/// `top_db` is the brightest visible level.
/// `range_db` is the visible dynamic range below `top_db`.
pub fn db_to_u8(db: f32, top_db: f32, range_db: f32) -> u8 {
    let range_db = range_db.max(1.0);
    let bottom_db = top_db - range_db;

    let normalized = ((db - bottom_db) / range_db).clamp(0.0, 1.0);
    (normalized * 255.0).round() as u8
}

/// Update the smoothed spectrum trace from an incoming dB row.
///
/// Behavior:
/// - if width changed, replace immediately
/// - otherwise, apply exponential smoothing per bin
pub fn update_spectrum_db(spectrum: &mut Vec<f32>, row_db: &[f32]) {
    if row_db.is_empty() {
        return;
    }

    if spectrum.len() != row_db.len() {
        spectrum.clear();
        spectrum.extend_from_slice(row_db);
        return;
    }

    for (dst, &src) in spectrum.iter_mut().zip(row_db.iter()) {
        *dst = (1.0 - SPECTRUM_SMOOTHING_ALPHA) * *dst
            + SPECTRUM_SMOOTHING_ALPHA * src;
    }
}

use crate::ui::layout::{
    SPECTRUM_DB_MAX, SPECTRUM_DB_MIN, SPECTRUM_SMOOTHING_ALPHA,
};

/// Map an 8-bit waterfall/spectrum intensity value to packed RGB.
///
/// The output format is:
/// - 0xRRGGBB
///
/// The gradient is:
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

/// Update the smoothed spectrum dB trace from an incoming 8-bit row.
///
/// Behavior:
/// - if the row length changes, rebuild the spectrum buffer immediately
/// - otherwise, apply exponential smoothing per bin
pub fn update_spectrum_db(spectrum: &mut Vec<f32>, row: &[u8]) {
    if row.is_empty() {
        return;
    }

    // Reinitialize if the incoming row width changed.
    if spectrum.len() != row.len() {
        spectrum.clear();
        spectrum.reserve(row.len());

        for &value in row {
            spectrum.push(byte_to_relative_db(value));
        }

        return;
    }

    // Apply simple exponential smoothing to reduce frame-to-frame flicker.
    for (dst, &src) in spectrum.iter_mut().zip(row.iter()) {
        let new_db = byte_to_relative_db(src);
        *dst = (1.0 - SPECTRUM_SMOOTHING_ALPHA) * *dst
            + SPECTRUM_SMOOTHING_ALPHA * new_db;
    }
}

/// Convert an 8-bit normalized magnitude value into the display dB range.
fn byte_to_relative_db(v: u8) -> f32 {
    SPECTRUM_DB_MIN
        + (v as f32 / 255.0) * (SPECTRUM_DB_MAX - SPECTRUM_DB_MIN)
}

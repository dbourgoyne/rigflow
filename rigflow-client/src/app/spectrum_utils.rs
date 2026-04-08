use crate::app::layout::{SPECTRUM_SMOOTHING_ALPHA, SPECTRUM_DB_MIN, SPECTRUM_DB_MAX};


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


pub fn update_spectrum_db(spectrum: &mut Vec<f32>, row: &[u8]) {
    if row.is_empty() {
        return;
    }

    if spectrum.len() != row.len() {
        spectrum.clear();
        spectrum.reserve(row.len());
        for &v in row {
            spectrum.push(byte_to_relative_db(v));
        }
        return;
    }

    for (dst, &src) in spectrum.iter_mut().zip(row.iter()) {
        let new_db = byte_to_relative_db(src);
        *dst = (1.0 - SPECTRUM_SMOOTHING_ALPHA) * *dst + SPECTRUM_SMOOTHING_ALPHA * new_db;
    }
}


fn byte_to_relative_db(v: u8) -> f32 {
    SPECTRUM_DB_MIN + (v as f32 / 255.0) * (SPECTRUM_DB_MAX - SPECTRUM_DB_MIN)
}

use crate::app::layout::{
    HEIGHT,
    SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1,
    SPECTRUM_PLOT_Y0, SPECTRUM_PLOT_Y1,
    SPECTRUM_PLOT_WIDTH, SPECTRUM_PLOT_HEIGHT,
    SPECTRUM_DB_MIN, SPECTRUM_DB_MAX,
    SPECTRUM_SMOOTHING_ALPHA,
};

use crate::render::color::{
    COLOR_AXIS,
    COLOR_LABEL,
    COLOR_BLACK,
    COLOR_GRID,
    COLOR_SPECTRUM,
    COLOR_SEPARATOR,
};

use crate::render::text::draw_text;
use crate::app::state::UiState;
use crate::render::color::COLOR_PASSBAND;

const SSB_LOW_HZ: f32 = 300.0;
const SSB_HIGH_HZ: f32 = 3000.0;
const WFM_HALF_BW_HZ: f32 = 75_000.0;

pub fn draw_passband(
    buffer: &mut [u32],
    width: usize,
    state: &UiState,
) {
    let Some((mut x0, mut x1)) = passband_x_range(state) else {
        return;
    };

    if x0 > x1 {
        std::mem::swap(&mut x0, &mut x1);
    }

    x0 = x0.max(SPECTRUM_PLOT_X0);
    x1 = x1.min(SPECTRUM_PLOT_X1.saturating_sub(1));

    if x0 >= x1 {
        return;
    }

    for y in SPECTRUM_PLOT_Y0..SPECTRUM_PLOT_Y1 {
        let row = y * width;
        for x in x0..=x1 {
            let idx = row + x;
            buffer[idx] = blend(buffer[idx], COLOR_PASSBAND);
        }
    }
}

fn passband_x_range(state: &UiState) -> Option<(usize, usize)> {
    let target = state.target_freq_hz;
    let (start_hz, end_hz) = match state.demod_mode.as_str() {
        "usb" => (target + SSB_LOW_HZ, target + SSB_HIGH_HZ),
        "lsb" => (target - SSB_HIGH_HZ, target - SSB_LOW_HZ),
        "wfm" => (target - WFM_HALF_BW_HZ, target + WFM_HALF_BW_HZ),
        _ => return None,
    };

    let x0 = freq_to_plot_x(start_hz, state)?;
    let x1 = freq_to_plot_x(end_hz, state)?;
    Some((x0, x1))
}

fn freq_to_plot_x(freq_hz: f32, state: &UiState) -> Option<usize> {
    if state.input_sample_rate_hz <= 0.0 {
        return None;
    }

    let left_hz = state.center_freq_hz - state.input_sample_rate_hz * 0.5;
    let right_hz = state.center_freq_hz + state.input_sample_rate_hz * 0.5;

    if freq_hz < left_hz || freq_hz > right_hz {
        return None;
    }

    let frac = (freq_hz - left_hz) / (right_hz - left_hz);
    let plot_width = (SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0) as f32;
    let x = SPECTRUM_PLOT_X0 as f32 + frac * plot_width;

    Some(x.round() as usize)
}

fn blend(dst: u32, src: u32) -> u32 {
    let dr = (dst >> 16) & 0xff;
    let dg = (dst >> 8) & 0xff;
    let db = dst & 0xff;

    let sr = (src >> 16) & 0xff;
    let sg = (src >> 8) & 0xff;
    let sb = src & 0xff;

    let alpha_num = 1u32;
    let alpha_den = 4u32;

    let r = (dr * (alpha_den - alpha_num) + sr * alpha_num) / alpha_den;
    let g = (dg * (alpha_den - alpha_num) + sg * alpha_num) / alpha_den;
    let b = (db * (alpha_den - alpha_num) + sb * alpha_num) / alpha_den;

    (r << 16) | (g << 8) | b
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


pub fn draw_spectrum_background(buffer: &mut [u32], width: usize, height: usize) {
    for y in 0..height {
        let row = &mut buffer[y * width..(y + 1) * width];
        row.fill(COLOR_BLACK);
    }
}


pub fn draw_spectrum_grid(
    buffer: &mut [u32],
    width: usize,
    plot_height: usize,
    db_min: f32,
    db_max: f32,
) {
    let marks = [-120.0, -100.0, -80.0, -60.0, -40.0, -20.0, 0.0];

    for &db in &marks {
        if db < db_min || db > db_max {
            continue;
        }

        let y = db_to_y(db, db_min, db_max, plot_height);
        if y >= plot_height {
            continue;
        }

        for x in 0..width {
            buffer[y * width + x] = COLOR_GRID;
        }
    }
}


pub fn draw_spectrum_trace(
    buffer: &mut [u32],
    width: usize,
    spectrum_db: &[f32],
) {
    if spectrum_db.len() < 2 || SPECTRUM_PLOT_WIDTH < 2 {
        return;
    }

    let mut prev_x = SPECTRUM_PLOT_X0 as i32;
    let mut prev_y = db_to_plot_y(spectrum_db[0]) as i32;

    for plot_x in 1..SPECTRUM_PLOT_WIDTH {
        let bin = plot_x * spectrum_db.len() / SPECTRUM_PLOT_WIDTH;
        let bin = bin.min(spectrum_db.len() - 1);

        let x = (SPECTRUM_PLOT_X0 + plot_x) as i32;
        let y = db_to_plot_y(spectrum_db[bin]) as i32;

        draw_line(buffer, width, prev_x, prev_y, x, y, COLOR_SPECTRUM);

        prev_x = x;
        prev_y = y;
    }
}


pub fn draw_spectrum_axes_and_labels(
    buffer: &mut [u32],
    width: usize,
    state: &UiState,
) {
    for y in SPECTRUM_PLOT_Y0..=SPECTRUM_PLOT_Y1 {
        buffer[y * width + SPECTRUM_PLOT_X0] = COLOR_AXIS;
    }

    for x in SPECTRUM_PLOT_X0..=SPECTRUM_PLOT_X1 {
        buffer[SPECTRUM_PLOT_Y1 * width + x] = COLOR_AXIS;
    }

    let db_ticks = [-120.0, -100.0, -80.0, -60.0, -40.0, -20.0, 0.0];
    for db in db_ticks {
        let y = db_to_plot_y(db);
        for x in SPECTRUM_PLOT_X0..SPECTRUM_PLOT_X1 {
            buffer[y * width + x] = COLOR_GRID;
        }

        let label = format!("{:.0}", db);
        let label_x = 4;
        let label_y = y.saturating_sub(3);
        draw_text(buffer, width, label_x, label_y, &label, COLOR_LABEL);
    }

    if state.input_sample_rate_hz > 0.0 {
        let left = state.center_freq_hz - 0.5 * state.input_sample_rate_hz;
        let center = state.center_freq_hz;
        let right = state.center_freq_hz + 0.5 * state.input_sample_rate_hz;

        let ticks = [
            (SPECTRUM_PLOT_X0, format_freq_label(left)),
            (SPECTRUM_PLOT_X0 + SPECTRUM_PLOT_WIDTH / 4, format_freq_label(left + 0.25 * state.input_sample_rate_hz)),
            (SPECTRUM_PLOT_X0 + SPECTRUM_PLOT_WIDTH / 2, format_freq_label(center)),
            (SPECTRUM_PLOT_X0 + 3 * SPECTRUM_PLOT_WIDTH / 4, format_freq_label(left + 0.75 * state.input_sample_rate_hz)),
            (SPECTRUM_PLOT_X1, format_freq_label(right)),
        ];

        for (x, label) in ticks {
            for y in SPECTRUM_PLOT_Y0..=SPECTRUM_PLOT_Y1 {
                if y % 4 == 0 {
                    buffer[y * width + x] = COLOR_GRID;
                }
            }

            let label_w = label.len() * 6;
            let label_x = x.saturating_sub(label_w / 2).min(width.saturating_sub(label_w));
            let label_y = SPECTRUM_PLOT_Y1 + 16;
            draw_text(buffer, width, label_x, label_y, &label, COLOR_LABEL);
        }
    }

    draw_text(buffer, width, 4, 2, "dB", COLOR_LABEL);
    draw_text(buffer, width, SPECTRUM_PLOT_X1.saturating_sub(14), 2, "Hz", COLOR_LABEL);
}


pub fn draw_frequency_overlay(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    const CF_COLOR: u32 = 0x00FFFF00;
    const TF_COLOR: u32 = 0x00FFA500;

    // 2x text metrics for the 5x7 font:
    // width = 5*2 = 10 px, spacing = 2 px, so each char advances 12 px
    const CHAR_ADVANCE_2X: usize = 12;
    const TEXT_HEIGHT_2X: usize = 14;

    // Top-left overlay for center frequency
    let cf_text = format!("CF: {}", format_freq_hz(state.center_freq_hz));
    let cf_x = SPECTRUM_PLOT_X0 + 8;
    let cf_y = SPECTRUM_PLOT_Y0 + 6;
    //draw_text_2x(buffer, fb_width, cf_x, cf_y, &cf_text, CF_COLOR);
    draw_text(buffer, fb_width, cf_x, cf_y, &cf_text, CF_COLOR);

    // Target-frequency label above the target marker location
    if let Some(tf_x_center) = freq_to_plot_x(state.target_freq_hz, state) {
        let tf_text = format!("TF: {}", format_freq_hz(state.target_freq_hz));
        let tf_width = tf_text.chars().count() * CHAR_ADVANCE_2X;

        let mut tf_x = tf_x_center.saturating_sub(tf_width / 2);
        let tf_y = SPECTRUM_PLOT_Y0 + 24;

        // Clamp label to visible spectrum area
        let min_x = SPECTRUM_PLOT_X0 + 4;
        let max_x = SPECTRUM_PLOT_X1.saturating_sub(tf_width + 4);

        if tf_x < min_x {
            tf_x = min_x;
        }
        if tf_x > max_x {
            tf_x = max_x;
        }

        draw_text(buffer, fb_width, tf_x, tf_y, &tf_text, TF_COLOR);

        // Optional small tick mark above the target x position
        let tick_top = tf_y + TEXT_HEIGHT_2X + 2;
        let tick_bottom = tick_top + 8;
        for y in tick_top..tick_bottom {
            if tf_x_center < fb_width && y < HEIGHT {
                buffer[y * fb_width + tf_x_center] = TF_COLOR;
            }
        }
    }
}


fn byte_to_relative_db(v: u8) -> f32 {
    SPECTRUM_DB_MIN + (v as f32 / 255.0) * (SPECTRUM_DB_MAX - SPECTRUM_DB_MIN)
}


fn db_to_y(db: f32, db_min: f32, db_max: f32, height: usize) -> usize {
    let t = ((db - db_min) / (db_max - db_min)).clamp(0.0, 1.0);
    (height - 1).saturating_sub((t * (height as f32 - 1.0)) as usize)
}


fn db_to_plot_y(db: f32) -> usize {
    let t = ((db - SPECTRUM_DB_MIN) / (SPECTRUM_DB_MAX - SPECTRUM_DB_MIN)).clamp(0.0, 1.0);
    SPECTRUM_PLOT_Y1 - (t * SPECTRUM_PLOT_HEIGHT as f32) as usize
}


fn draw_line(
    buffer: &mut [u32],
    fb_width: usize,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        put_pixel(buffer, fb_width, x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn format_freq_label(freq_hz: f32) -> String {
    if freq_hz.abs() >= 1_000_000.0 {
        format!("{:.3}M", freq_hz / 1_000_000.0)
    } else if freq_hz.abs() >= 1_000.0 {
        format!("{:.1}k", freq_hz / 1_000.0)
    } else {
        format!("{:.0}", freq_hz)
    }
}

fn format_freq_hz(freq_hz: f32) -> String {
    if freq_hz.abs() >= 1_000_000.0 {
        format!("{:.3} MHz", freq_hz / 1_000_000.0)
    } else if freq_hz.abs() >= 1_000.0 {
        format!("{:.3} kHz", freq_hz / 1_000.0)
    } else {
        format!("{:.0} Hz", freq_hz)
    }
}


fn put_pixel(buffer: &mut [u32], fb_width: usize, x: i32, y: i32, color: u32) {
    if x < 0 || y < 0 {
        return;
    }

    let x = x as usize;
    let y = y as usize;

    if x >= fb_width || y >= HEIGHT {
        return;
    }

    let idx = y * fb_width + x;
    if idx < buffer.len() {
        buffer[idx] = color;
    }
}

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


pub fn draw_tuning_marker(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    y_start: usize,
    state: &UiState,
) {
    if state.input_sample_rate_hz <= 0.0 || SPECTRUM_PLOT_WIDTH == 0 {
        return;
    }

    let offset_hz = state.target_freq_hz - state.center_freq_hz;
    let frac = offset_hz / state.input_sample_rate_hz + 0.5;
    let x = SPECTRUM_PLOT_X0 as f32 + frac * SPECTRUM_PLOT_WIDTH as f32;
    let x = x.round() as isize;

    if x < 0 || x >= width as isize {
        return;
    }

    let x = x as usize;
    for y in y_start..height {
        buffer[y * width + x] = 0x00FF0000;
    }
}

pub fn draw_separator(buffer: &mut [u32], width: usize, y: usize) {
    if y >= HEIGHT {
        return;
    }

    let row = &mut buffer[y * width..(y + 1) * width];
    row.fill(COLOR_SEPARATOR);
}

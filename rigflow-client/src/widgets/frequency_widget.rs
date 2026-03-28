use crate::{
    app::state::UiState,
    render::{
        color::{COLOR_TEXT, COLOR_TEXT_DIM},
        text::draw_text_2x,
    },
};

pub const FREQ_WIDGET_DIGITS: usize = 10;
pub const FREQ_WIDGET_CHAR_W: usize = 12; // draw_text_2x spacing
pub const FREQ_WIDGET_CHAR_H: usize = 14; // 7 rows * 2
pub const FREQ_WIDGET_GROUP_SEPARATORS: [usize; 3] = [1, 4, 7];

#[derive(Debug, Clone, Copy)]
pub struct FrequencyWidgetLayout {
    pub x: usize,
    pub y: usize,
}

pub fn format_center_frequency_digits(freq_hz: u64) -> [u8; FREQ_WIDGET_DIGITS] {
    let s = format!("{:010}", freq_hz.min(9_999_999_999));
    let mut out = [0u8; FREQ_WIDGET_DIGITS];
    for (i, b) in s.as_bytes().iter().enumerate() {
        out[i] = *b - b'0';
    }
    out
}

pub fn digit_place_value(digit_index: usize) -> u64 {
    10u64.pow((FREQ_WIDGET_DIGITS - 1 - digit_index) as u32)
}

pub fn hit_test_digit(
    mouse_x: f32,
    mouse_y: f32,
    layout: FrequencyWidgetLayout,
) -> Option<usize> {
    let mx = mouse_x as isize;
    let my = mouse_y as isize;

    let top = layout.y as isize;
    let bottom = top + FREQ_WIDGET_CHAR_H as isize;
    if my < top || my >= bottom {
        return None;
    }

    let mut cx = layout.x as isize;
    for digit_idx in 0..FREQ_WIDGET_DIGITS {
        let left = cx;
        let right = cx + FREQ_WIDGET_CHAR_W as isize;

        if mx >= left && mx < right {
            return Some(digit_idx);
        }

        cx += FREQ_WIDGET_CHAR_W as isize;

        if FREQ_WIDGET_GROUP_SEPARATORS.contains(&(digit_idx + 1)) {
            cx += FREQ_WIDGET_CHAR_W as isize; // comma spacing
        }
    }

    None
}

pub fn apply_digit_wheel_delta(center_freq_hz: u64, digit_index: usize, wheel_delta: f32) -> u64 {
    if digit_index >= FREQ_WIDGET_DIGITS || wheel_delta == 0.0 {
        return center_freq_hz;
    }

    let step = digit_place_value(digit_index) as i64;
    let dir = if wheel_delta > 0.0 { 1i64 } else { -1i64 };

    let next = center_freq_hz as i64 - dir * step;
    next.clamp(0, 9_999_999_999) as u64
}

pub fn draw_center_frequency_widget(
    buffer: &mut [u32],
    fb_width: usize,
    layout: FrequencyWidgetLayout,
    state: &UiState,
    hovered_digit: Option<usize>,
) {
    let digits = format_center_frequency_digits(state.center_freq_hz.max(0.0) as u64);

    let first_non_zero = digits.iter().position(|&d| d != 0).unwrap_or(FREQ_WIDGET_DIGITS - 1);

    let mut text = String::with_capacity(13);
    for (i, d) in digits.iter().enumerate() {
        text.push(char::from(b'0' + *d));
        if FREQ_WIDGET_GROUP_SEPARATORS.contains(&(i + 1)) {
            text.push(',');
        }
    }

    // Draw char-by-char so we can dim leading zeros and optionally highlight hovered digit.
    let chars: Vec<char> = text.chars().collect();

    let mut cx = layout.x;
    let mut digit_idx = 0usize;

    for ch in chars {
        let is_digit = ch.is_ascii_digit();

        let color = if is_digit {
            let is_leading_zero = digit_idx < first_non_zero && digits[digit_idx] == 0;
            let mut color = if is_leading_zero { COLOR_TEXT_DIM } else { COLOR_TEXT };

            if hovered_digit == Some(digit_idx) {
                color = 0x00ffd0ff;
            }

            digit_idx += 1;
            color
        } else {
            COLOR_TEXT
        };

        draw_text_2x(buffer, fb_width, cx, layout.y, &ch.to_string(), color);
        cx += FREQ_WIDGET_CHAR_W;

        if ch == ',' {
            // already consumed one char width, no extra needed
        }
    }
}

use crate::render::text::{draw_text, text_width_px};

use super::types::{MouseState, Rect, WidgetColors};

pub fn draw_text_input(
    frame: &mut [u32],
    frame_width: usize,
    rect: Rect,
    value: &str,
    focused: bool,
    mouse: Option<MouseState>,
    colors: WidgetColors,
) -> bool {
    fill_rect(frame, frame_width, rect, colors.bg);
    draw_rect(
        frame,
        frame_width,
        rect,
        if focused { colors.accent } else { colors.border },
    );

    draw_text(
        frame,
        frame_width,
        rect.x + 6,
        rect.y + 6,
        value,
        colors.text,
    );

    if focused {
        let cursor_x = rect.x + 6 + text_width_px(value);
        draw_rect(
            frame,
            frame_width,
            Rect {
                x: cursor_x,
                y: rect.y + 4,
                w: 1,
                h: rect.h.saturating_sub(8),
            },
            colors.text,
        );
    }

    if let Some(mouse) = mouse {
        if mouse.left_clicked && rect.contains(mouse.x, mouse.y) {
            return true;
        }
    }

    false
}

pub fn handle_text_input_char(value: &mut String, ch: char) {
    if ch.is_ascii_digit() || ch == '.' {
        value.push(ch);
    }
}

pub fn handle_text_input_backspace(value: &mut String) {
    value.pop();
}

fn fill_rect(frame: &mut [u32], frame_width: usize, rect: Rect, color: u32) {
    let frame_height = frame.len() / frame_width;
    let x0 = rect.x.min(frame_width);
    let y0 = rect.y.min(frame_height);
    let x1 = (rect.x + rect.w).min(frame_width);
    let y1 = (rect.y + rect.h).min(frame_height);

    for y in y0..y1 {
        let row_start = y * frame_width;
        for x in x0..x1 {
            frame[row_start + x] = color;
        }
    }
}

fn draw_rect(frame: &mut [u32], frame_width: usize, rect: Rect, color: u32) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }

    fill_rect(frame, frame_width, Rect { x: rect.x, y: rect.y, w: rect.w, h: 1 }, color);

    if rect.h > 1 {
        fill_rect(
            frame,
            frame_width,
            Rect { x: rect.x, y: rect.y + rect.h - 1, w: rect.w, h: 1 },
            color,
        );
    }

    fill_rect(frame, frame_width, Rect { x: rect.x, y: rect.y, w: 1, h: rect.h }, color);

    if rect.w > 1 {
        fill_rect(
            frame,
            frame_width,
            Rect { x: rect.x + rect.w - 1, y: rect.y, w: 1, h: rect.h },
            color,
        );
    }
}

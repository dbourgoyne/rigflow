use crate::render::text::draw_text;

use super::types::{MouseState, Rect, WidgetColors};

pub fn draw_collapsible_header(
    frame: &mut [u32],
    frame_width: usize,
    rect: Rect,
    title: &str,
    expanded: bool,
    mouse: Option<MouseState>,
    colors: WidgetColors,
) -> bool {
    fill_rect(frame, frame_width, rect, colors.bg);
    draw_rect(frame, frame_width, rect, colors.border);

    let arrow = if expanded { "▼" } else { "→" };

    draw_text(
        frame,
        frame_width,
        rect.x + 8,
        rect.y + 8,
        &format!("{arrow} {title}"),
        colors.text,
    );

    if let Some(mouse) = mouse {
        if mouse.left_clicked && rect.contains(mouse.x, mouse.y) {
            return true;
        }
    }

    false
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

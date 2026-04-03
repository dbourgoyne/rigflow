use crate::app::state::UiState;
use crate::app::layout::{
    LEFT_PANE_WIDTH, PANEL_PADDING, HEADER_HEIGHT,
    ROW_HEIGHT, BUTTON_HEIGHT, FIELD_HEIGHT,
    SECTION_SPACING,
};
use crate::render::text::draw_text;
use crate::net::control::ControlCommand;


#[derive(Debug, Clone)]
pub struct LeftPaneLayout {
    pub pane: Rect,
    pub rigflow_header: Rect,
    pub server_ip_field: Option<Rect>,
    pub connect_button: Option<Rect>,
}

pub fn compute_left_pane_layout(window_width: usize, window_height: usize, ui: &UiState) -> LeftPaneLayout {
    let pane = Rect {
        x: 0,
        y: 0,
        w: LEFT_PANE_WIDTH.min(window_width),
        h: window_height,
    };

    let mut y = PANEL_PADDING;

    let rigflow_header = Rect {
        x: PANEL_PADDING,
        y,
        w: pane.w - 2 * PANEL_PADDING,
        h: HEADER_HEIGHT,
    };

    y += HEADER_HEIGHT + 6;

    let mut server_ip_field = None;
    let mut connect_button = None;

    if ui.rigflow_server_menu_expanded {
        server_ip_field = Some(Rect {
            x: PANEL_PADDING,
            y: y + ROW_HEIGHT,
            w: pane.w - 2 * PANEL_PADDING,
            h: FIELD_HEIGHT,
        });

        y += ROW_HEIGHT + FIELD_HEIGHT + 8;

        connect_button = Some(Rect {
            x: PANEL_PADDING,
            y,
            w: 120,
            h: BUTTON_HEIGHT,
        });

        y += BUTTON_HEIGHT + 8;

        // status text follows visually, no click rect needed for now
        let _status_y = y;
    }

    LeftPaneLayout {
        pane,
        rigflow_header,
        server_ip_field,
        connect_button,
    }
}

pub fn draw_left_pane(
    frame: &mut [u32],
    window_width: usize,
    window_height: usize,
    ui: &UiState,
) {
    let layout = compute_left_pane_layout(window_width, window_height, ui);

    // Pane background
    fill_rect(frame, window_width, layout.pane, 0x202020);

    // Pane border
    draw_rect(frame, window_width, layout.pane, 0x505050);

    // Header
    fill_rect(frame, window_width, layout.rigflow_header, 0x303030);
    draw_rect(frame, window_width, layout.rigflow_header, 0x606060);

    let arrow = if ui.rigflow_server_menu_expanded { "▼" } else { "▶" };
    draw_text(
        frame,
        window_width,
        layout.rigflow_header.x + 8,
        layout.rigflow_header.y + 8,
        &format!("{arrow} rigflow server"),
        0xFFFFFF,
    );

    if ui.rigflow_server_menu_expanded {
        let label_x = PANEL_PADDING;
        let mut y = layout.rigflow_header.y + layout.rigflow_header.h + 6;

        draw_text(
            frame,
            window_width,
            label_x,
            y + 6,
            "rigflow server IP:",
            0xDDDDDD,
        );

        if let Some(ip_rect) = layout.server_ip_field {
            fill_rect(frame, window_width, ip_rect, 0x101010);
            draw_rect(
                frame,
                window_width,
                ip_rect,
                if ui.editing_server_ip { 0xAAAAFF } else { 0x707070 },
            );

            draw_text(
                frame,
                window_width,
                ip_rect.x + 6,
                ip_rect.y + 6,
                &ui.rigflow_server_ip,
                0xFFFFFF,
            );

            if ui.editing_server_ip {
                // optional simple cursor
                let cursor_x = ip_rect.x + 6 + (ui.rigflow_server_ip.len() * 8);
                draw_rect(
                    frame,
                    window_width,
                    Rect { x: cursor_x, y: ip_rect.y + 4, w: 1, h: ip_rect.h - 8 },
                    0xFFFFFF,
                );
            }
        }

        y += ROW_HEIGHT + FIELD_HEIGHT + 8;

        if let Some(button_rect) = layout.connect_button {
            fill_rect(frame, window_width, button_rect, 0x404040);
            draw_rect(frame, window_width, button_rect, 0x808080);

            let button_text = if ui.server_connected { "Disconnect" } else { "Connect" };

            draw_text(
                frame,
                window_width,
                button_rect.x + 10,
                button_rect.y + 8,
                button_text,
                0xFFFFFF,
            );
        }

        y += BUTTON_HEIGHT + 10;

        draw_text(
            frame,
            window_width,
            label_x,
            y,
            "Status:",
            0xDDDDDD,
        );

        draw_text(
            frame,
            window_width,
            label_x,
            y + 18,
            &ui.server_status,
            0xA0FFA0,
        );
    }
}

pub fn handle_left_pane_click(
    mouse_x: usize,
    mouse_y: usize,
    window_width: usize,
    window_height: usize,
    ui: &mut UiState,
) -> Option<ControlCommand> {
    let layout = compute_left_pane_layout(window_width, window_height, ui);

    if layout.rigflow_header.contains(mouse_x, mouse_y) {
        ui.rigflow_server_menu_expanded = !ui.rigflow_server_menu_expanded;
        return None;
    }

    if !ui.rigflow_server_menu_expanded {
        ui.editing_server_ip = false;
        return None;
    }

    if let Some(ip_rect) = layout.server_ip_field {
        if ip_rect.contains(mouse_x, mouse_y) {
            ui.editing_server_ip = true;
            return None;
        }
    }

    ui.editing_server_ip = false;

    if let Some(button_rect) = layout.connect_button {
        if button_rect.contains(mouse_x, mouse_y) {
            if ui.server_connected {
                return Some(ControlCommand::Disconnect);
            } else {
                return Some(ControlCommand::Connect {
                    server_ip: ui.rigflow_server_ip.clone(),
                });
            }
        }
    }

    None
}

pub fn handle_left_pane_text_input(ui: &mut UiState, ch: char) {
    if !ui.editing_server_ip {
        return;
    }

    if ch.is_ascii_digit() || ch == '.' {
        ui.rigflow_server_ip.push(ch);
    }
}

pub fn handle_left_pane_backspace(ui: &mut UiState) {
    if ui.editing_server_ip {
        ui.rigflow_server_ip.pop();
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn contains(&self, px: usize, py: usize) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
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

    // top
    fill_rect(
        frame,
        frame_width,
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: 1,
        },
        color,
    );

    // bottom
    if rect.h > 1 {
        fill_rect(
            frame,
            frame_width,
            Rect {
                x: rect.x,
                y: rect.y + rect.h - 1,
                w: rect.w,
                h: 1,
            },
            color,
        );
    }

    // left
    fill_rect(
        frame,
        frame_width,
        Rect {
            x: rect.x,
            y: rect.y,
            w: 1,
            h: rect.h,
        },
        color,
    );

    // right
    if rect.w > 1 {
        fill_rect(
            frame,
            frame_width,
            Rect {
                x: rect.x + rect.w - 1,
                y: rect.y,
                w: 1,
                h: rect.h,
            },
            color,
        );
    }
}

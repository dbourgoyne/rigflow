use crate::{
    app::{
        layout::{
            BUTTON_HEIGHT, FIELD_HEIGHT, HEADER_HEIGHT, HEIGHT, LEFT_PANE_WIDTH, PANEL_PADDING,
            ROW_HEIGHT, SECTION_SPACING, WIDTH,
        },
        state::UiState,
    },
    net::control::ControlCommand,
    render::{
        text::draw_text,
        widgets::{
            draw_button, draw_collapsible_header, draw_text_input, handle_text_input_backspace,
            handle_text_input_char, MouseState, Rect, WidgetColors,
        },
    },
};

#[derive(Debug, Clone)]
pub struct LeftPaneLayout {
    pub pane: Rect,
    pub rigflow_header: Rect,
    pub server_ip_field: Option<Rect>,
    pub connect_button: Option<Rect>,
}

pub fn compute_left_pane_layout(ui: &UiState) -> LeftPaneLayout {
    let pane = Rect {
        x: 0,
        y: 0,
        w: LEFT_PANE_WIDTH.min(WIDTH),
        h: HEIGHT,
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
    _window_height: usize,
    state: &UiState,
) {
    let layout = compute_left_pane_layout(state);

    let colors = WidgetColors {
        bg: 0x202020,
        border: 0x606060,
        text: 0xFFFFFF,
        accent: 0xAAAAFF,
    };

    fill_rect(frame, window_width, layout.pane, 0x202020);
    draw_rect(frame, window_width, layout.pane, 0x505050);

    let _ = draw_collapsible_header(
        frame,
        window_width,
        layout.rigflow_header,
        "rigflow server",
        state.rigflow_server_menu_expanded,
        None,
        colors,
    );

    if state.rigflow_server_menu_expanded {
        let mut y = layout.rigflow_header.y + layout.rigflow_header.h + 6;

        draw_text(
            frame,
            window_width,
            PANEL_PADDING,
            y + 6,
            "rigflow server IP:",
            0xDDDDDD,
        );

        if let Some(ip_rect) = layout.server_ip_field {
            let _ = draw_text_input(
                frame,
                window_width,
                ip_rect,
                &state.rigflow_server_ip,
                state.editing_server_ip,
                None,
                colors,
            );
        }

        y += ROW_HEIGHT + FIELD_HEIGHT + 8;

        if let Some(button_rect) = layout.connect_button {
            let label = if state.server_connected {
                "Disconnect"
            } else {
                "Connect"
            };

            let _ = draw_button(
                frame,
                window_width,
                button_rect,
                label,
                None,
                colors,
            );
        }

        y += BUTTON_HEIGHT + SECTION_SPACING;

        draw_text(frame, window_width, PANEL_PADDING, y, "Status:", 0xDDDDDD);
        draw_text(
            frame,
            window_width,
            PANEL_PADDING,
            y + 18,
            &state.server_status,
            0xA0FFA0,
        );
    }
}

pub fn handle_left_pane_click(
    mouse_x: usize,
    mouse_y: usize,
    ui: &mut UiState,
) -> Option<ControlCommand> {
    let layout = compute_left_pane_layout(ui);

    if layout.rigflow_header.contains(mouse_x, mouse_y) {
        ui.rigflow_server_menu_expanded = !ui.rigflow_server_menu_expanded;
        ui.editing_server_ip = false;
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
    if ui.editing_server_ip {
        handle_text_input_char(&mut ui.rigflow_server_ip, ch);
    }
}

pub fn handle_left_pane_backspace(ui: &mut UiState) {
    if ui.editing_server_ip {
        handle_text_input_backspace(&mut ui.rigflow_server_ip);
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

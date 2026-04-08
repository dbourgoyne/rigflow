use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

#[derive(Debug, Clone, Copy)]
pub enum DigitWheelAnchor {
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct DigitWheelSpec<'a> {
    pub label: &'a str,
    pub digit_count: usize,
    pub signed: bool,
    pub groups: &'a [usize], // e.g. &[1,3,3,3] or &[3,3]
    pub anchor: DigitWheelAnchor,
}

#[derive(Debug, Clone, Copy)]
struct DigitCell {
    rect: Rect,
    digit_index: usize, // 0 = most significant digit
}

fn pow10_u64(exp: usize) -> u64 {
    let mut v = 1u64;
    for _ in 0..exp {
        v *= 10;
    }
    v
}

fn format_abs_digits(value: u64, digit_count: usize) -> Vec<u8> {
    let s = format!("{value:0width$}", width = digit_count);
    s.into_bytes()
}

fn first_nonzero_digit(digits: &[u8]) -> Option<usize> {
    digits.iter().position(|d| *d != b'0')
}

fn color32_from_u32_with_alpha(rgb: u32, alpha: u8) -> Color32 {
    let r = ((rgb >> 16) & 0xff) as u8;
    let g = ((rgb >> 8) & 0xff) as u8;
    let b = (rgb & 0xff) as u8;
    Color32::from_rgba_premultiplied(r, g, b, alpha)
}

fn digit_step(digit_count: usize, digit_index: usize) -> i64 {
    let place_from_right = digit_count - 1 - digit_index;
    pow10_u64(place_from_right) as i64
}

fn total_widget_width(
    label_w: f32,
    label_gap: f32,
    digit_w: f32,
    digit_gap: f32,
    sep_w: f32,
    sign_w: f32,
    spec: &DigitWheelSpec<'_>,
) -> f32 {
    let digit_area = digit_w * spec.digit_count as f32
        + digit_gap * (spec.digit_count.saturating_sub(1)) as f32
        + sep_w * spec.groups.len().saturating_sub(1) as f32;

    let sign_area = if spec.signed { sign_w + digit_gap } else { 0.0 };

    label_w + label_gap + sign_area + digit_area
}

/// Draw a reusable digit-wheel widget.
/// Returns Some(new_value) when mouse wheel changes the hovered digit.
pub fn draw_digit_wheel_widget(
    ui: &mut egui::Ui,
    origin: Pos2,
    spec: &DigitWheelSpec<'_>,
    value: i64,
) -> Option<i64> {
    let font = FontId::proportional(28.0);
    let label_font = FontId::proportional(20.0);

    let active_color = Color32::from_rgb(235, 235, 235);
    let inactive_color = Color32::from_rgb(90, 90, 90);
    let hover_bg = Color32::from_rgba_premultiplied(120, 120, 120, 40);
    let label_color = Color32::from_rgb(180, 180, 180);
    let sign_color = Color32::from_rgb(210, 210, 210);

    let digit_w = 20.0;
    let digit_h = 40.0;
    let digit_gap = 2.0;
    let sep_w = 10.0;
    let sign_w = 16.0;
    let label_gap = 10.0;

    let label_w = match spec.label {
        "LO" => 28.0,
        "LO Offset" => 82.0,
        _ => 70.0,
    };

    let widget_w = total_widget_width(
        label_w,
        label_gap,
        digit_w,
        digit_gap,
        sep_w,
        sign_w,
        spec,
    );
    let widget_h = digit_h;

    let top_left = match spec.anchor {
        DigitWheelAnchor::Left => origin,
        DigitWheelAnchor::Right => Pos2::new(origin.x - widget_w, origin.y),
    };

    let total_rect = Rect::from_min_size(top_left, Vec2::new(widget_w, widget_h));
    let response = ui.allocate_rect(total_rect, Sense::hover());
    let painter = ui.painter();

    let abs_value = value.unsigned_abs();
    let digits = format_abs_digits(abs_value, spec.digit_count);
    let first_nonzero = first_nonzero_digit(&digits).unwrap_or(spec.digit_count - 1);

    painter.text(
        Pos2::new(top_left.x, top_left.y + digit_h * 0.5),
        Align2::LEFT_CENTER,
        spec.label,
        label_font,
        label_color,
    );

    let mut x = top_left.x + label_w + label_gap;

    if spec.signed {
        let sign_text = if value < 0 { "-" } else { "+" };
        painter.text(
            Pos2::new(x + sign_w * 0.5, top_left.y + digit_h * 0.5),
            Align2::CENTER_CENTER,
            sign_text,
            font.clone(),
            sign_color,
        );
        x += sign_w + digit_gap;
    }

    let mut digit_cells = Vec::with_capacity(spec.digit_count);
    let mut digit_i = 0usize;

    for (group_idx, group_len) in spec.groups.iter().enumerate() {
        for _ in 0..*group_len {
            let rect = Rect::from_min_size(Pos2::new(x, top_left.y), Vec2::new(digit_w, digit_h));
            digit_cells.push(DigitCell {
                rect,
                digit_index: digit_i,
            });

            x += digit_w;
            if digit_i < spec.digit_count - 1 {
                x += digit_gap;
            }
            digit_i += 1;
        }

        if group_idx < spec.groups.len() - 1 {
            painter.text(
                Pos2::new(x + sep_w * 0.5, top_left.y + digit_h * 0.52),
                Align2::CENTER_CENTER,
                ".",
                font.clone(),
                active_color,
            );
            x += sep_w;
        }
    }

    let hovered_digit = response.hover_pos().and_then(|pos| {
        digit_cells
            .iter()
            .find(|c| c.rect.contains(pos))
            .map(|c| c.digit_index)
    });

    for cell in &digit_cells {
        if hovered_digit == Some(cell.digit_index) {
            painter.rect_filled(cell.rect, 3.0, hover_bg);
        }

        let d = digits[cell.digit_index] as char;
        let color = if cell.digit_index < first_nonzero {
            inactive_color
        } else {
            active_color
        };

        painter.text(
            Pos2::new(cell.rect.center().x, cell.rect.center().y),
            Align2::CENTER_CENTER,
            d,
            font.clone(),
            color,
        );
    }

    painter.rect_stroke(
        total_rect.expand(2.0),
        4.0,
        Stroke::new(1.0, color32_from_u32_with_alpha(0xffffff, 24)),
        egui::StrokeKind::Inside,
    );

    if let Some(idx) = hovered_digit {
        let scroll_y = ui.ctx().input(|i| i.raw_scroll_delta.y);
        if scroll_y.abs() > 0.0 {
            let step = digit_step(spec.digit_count, idx);
            let delta = if scroll_y > 0.0 { step } else { -step };

            let next = if spec.signed {
                value.saturating_add(delta)
            } else {
                value.saturating_add(delta).max(0)
            };

            return Some(next);
        }
    }

    None
}

pub fn draw_lo_widget(
    ui: &mut egui::Ui,
    top_left: Pos2,
    center_freq_hz: u64,
) -> Option<u64> {
    let spec = DigitWheelSpec {
        label: "LO",
        digit_count: 10,
        signed: false,
        groups: &[1, 3, 3, 3],
        anchor: DigitWheelAnchor::Left,
    };

    draw_digit_wheel_widget(ui, top_left, &spec, center_freq_hz as i64)
        .map(|v| v.max(0) as u64)
}

pub fn draw_lo_offset_widget(
    ui: &mut egui::Ui,
    top_right: Pos2,
    offset_hz: i64,
) -> Option<i64> {
    let spec = DigitWheelSpec {
        label: "LO Offset",
        digit_count: 6,
        signed: true,
        groups: &[3, 3],
        anchor: DigitWheelAnchor::Right,
    };

    draw_digit_wheel_widget(ui, top_right, &spec, offset_hz)
}

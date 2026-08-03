use std::hash::Hash;

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Key, Modifiers, Pos2, Rect, Sense, Vec2,
};

const DRAG_POINTS_PER_STEP: f32 = 7.0;

/// Controls how the digit widget is anchored relative to the provided origin.
#[derive(Debug, Clone, Copy)]
pub enum DigitWheelAnchor {
    Left,
    Right,
}

/// Specification for a digit-wheel widget.
#[derive(Debug, Clone)]
pub struct DigitWheelSpec<'a> {
    pub label: &'a str,
    pub digit_count: usize,
    pub signed: bool,
    pub groups: &'a [usize],
    pub anchor: DigitWheelAnchor,
}

#[derive(Debug, Clone, Copy)]
struct DigitCell {
    rect: Rect,
    digit_index: usize,
}

#[derive(Debug, Clone, Copy)]
struct DragState {
    digit_index: usize,
    start_value: i64,
    accumulated_points: f32,
}

#[derive(Debug, Clone)]
struct DigitWheelState {
    editing: bool,
    focus_editor: bool,
    draft: String,
    edit_error: bool,
    drag: Option<DragState>,
    point_wheel_accumulator: f32,
    point_wheel_direction: i8,
    point_wheel_digit: Option<usize>,
    last_point_wheel_at: f64,
    last_point_step_at: f64,
}

impl Default for DigitWheelState {
    fn default() -> Self {
        Self {
            editing: false,
            focus_editor: false,
            draft: String::new(),
            edit_error: false,
            drag: None,
            point_wheel_accumulator: 0.0,
            point_wheel_direction: 0,
            point_wheel_digit: None,
            last_point_wheel_at: f64::NEG_INFINITY,
            last_point_step_at: f64::NEG_INFINITY,
        }
    }
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

fn format_editor_value(value: i64, spec: &DigitWheelSpec<'_>) -> String {
    let digits = format!("{:0width$}", value.unsigned_abs(), width = spec.digit_count);
    let grouped_width: usize = spec.groups.iter().sum();
    debug_assert_eq!(grouped_width, spec.digit_count);

    let mut out = String::with_capacity(digits.len() + spec.groups.len());
    if spec.signed {
        out.push(if value < 0 { '-' } else { '+' });
    }

    // If a caller supplies a value wider than the nominal digit count, retain
    // the extra leading digits in the first group rather than truncating them.
    let mut offset = 0;
    let overflow = digits.len().saturating_sub(grouped_width);
    for (group_index, group_width) in spec.groups.iter().copied().enumerate() {
        if group_index != 0 {
            out.push('.');
        }
        let width = group_width + usize::from(group_index == 0) * overflow;
        out.push_str(&digits[offset..offset + width]);
        offset += width;
    }
    out
}

fn editor_character_distance(current: char, next: char) -> Option<f32> {
    match (current, next) {
        (c, n) if c.is_ascii_digit() && n.is_ascii_digit() => Some(14.0),
        (c, '.') if c.is_ascii_digit() => Some(11.0),
        ('.', n) if n.is_ascii_digit() => Some(10.0),
        ('+' | '-', n) if n.is_ascii_digit() => Some(13.5),
        _ => None,
    }
}

fn layout_editor_text(
    ui: &egui::Ui,
    text: &str,
    _wrap_width: f32,
    font: &FontId,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let chars: Vec<char> = text.chars().collect();
    let widths: Vec<f32> = ui.fonts(|fonts| {
        chars
            .iter()
            .map(|character| fonts.glyph_width(font, *character))
            .collect()
    });
    let mut job = egui::text::LayoutJob::default();
    // Match TextEdit's built-in single-line layouter: do not wrap at the field
    // width (or at separators), and let TextEdit horizontally clip/scroll.
    job.break_on_newline = false;

    for (index, character) in chars.iter().copied().enumerate() {
        let format = egui::TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        };
        let leading_space = if let Some(previous) = index.checked_sub(1)
            && let Some(center_distance) = editor_character_distance(chars[previous], character)
        {
            // Match the fixed-cell painter by compensating for the actual glyph
            // advances. This also remains correct if the chosen font's digits
            // are not tabular.
            center_distance - (widths[previous] + widths[index]) * 0.5
        } else {
            0.0
        };
        // Each character is a separate section so its gap can reflect the
        // fixed digit/separator geometry. `extra_letter_spacing` would have no
        // effect here because it only separates glyphs within one section.
        job.append(&character.to_string(), leading_space, format);
    }

    ui.fonts(|fonts| fonts.layout_job(job))
}

fn first_nonzero_digit(digits: &[u8]) -> Option<usize> {
    digits.iter().position(|d| *d != b'0')
}

fn digit_step(digit_count: usize, digit_index: usize) -> i64 {
    let place_from_right = digit_count - 1 - digit_index;
    pow10_u64(place_from_right) as i64
}

fn adjusted_value(value: i64, delta: i64, signed: bool) -> i64 {
    let next = value.saturating_add(delta);
    if signed { next } else { next.max(0) }
}

fn dragged_value(
    drag: &mut DragState,
    frame_delta_y: f32,
    digit_count: usize,
    signed: bool,
) -> i64 {
    // `Response::drag_delta` is the movement in this frame, not the total
    // displacement since the gesture started. Keep the total here so a still
    // frame does not snap back to `start_value`, and retain sub-step movement.
    drag.accumulated_points -= frame_delta_y;
    let increments = (drag.accumulated_points / DRAG_POINTS_PER_STEP).trunc() as i64;
    let delta = digit_step(digit_count, drag.digit_index).saturating_mul(increments);
    adjusted_value(drag.start_value, delta, signed)
}

fn parse_display_prefix(number: &str, spec: &DigitWheelSpec<'_>) -> Option<i64> {
    let (sign, unsigned) = if let Some(unsigned) = number.strip_prefix('-') {
        if !spec.signed {
            return None;
        }
        ("-", unsigned)
    } else if let Some(unsigned) = number.strip_prefix('+') {
        ("+", unsigned)
    } else {
        ("", number)
    };

    // Without a display separator, bare input retains its documented literal-Hz
    // meaning. A dotted value matching the widget's leading groups is instead
    // treated as a possibly incomplete copy of the displayed value.
    if !unsigned.contains('.') {
        return None;
    }
    let entered_groups: Vec<&str> = unsigned.split('.').collect();
    if entered_groups.len() > spec.groups.len() {
        return None;
    }

    let mut digits = String::with_capacity(spec.digit_count);
    for (index, width) in spec.groups.iter().copied().enumerate() {
        let entered = entered_groups.get(index).copied().unwrap_or("");
        if entered.len() > width || !entered.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        digits.push_str(entered);
        digits.extend(std::iter::repeat_n('0', width - entered.len()));
    }

    format!("{sign}{digits}").parse().ok()
}

/// Parse an editor value. Bare values are Hz (grouping dots/commas are allowed);
/// a left-to-right prefix of the dotted widget display is padded with trailing
/// zeroes, and an explicit Hz/kHz/MHz suffix enables decimal unit input.
fn parse_frequency(text: &str, spec: &DigitWheelSpec<'_>) -> Option<i64> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = compact.to_ascii_lowercase();
    let (number, multiplier, has_unit) = if let Some(n) = lower.strip_suffix("mhz") {
        (n, 1_000_000.0, true)
    } else if let Some(n) = lower.strip_suffix("khz") {
        (n, 1_000.0, true)
    } else if let Some(n) = lower.strip_suffix("hz") {
        (n, 1.0, true)
    } else {
        (lower.as_str(), 1.0, false)
    };

    let parsed = if has_unit {
        let n = number.replace(',', "");
        let hz = n.parse::<f64>().ok()? * multiplier;
        if !hz.is_finite() || hz < i64::MIN as f64 || hz > i64::MAX as f64 {
            return None;
        }
        hz.round() as i64
    } else if let Some(parsed) = parse_display_prefix(number, spec) {
        parsed
    } else {
        number.replace(['.', ',', '_'], "").parse::<i64>().ok()?
    };

    (spec.signed || parsed >= 0).then_some(parsed)
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

/// Normalize wheel input into one digit step.
///
/// A traditional wheel reports line events, whose magnitude is ignored. Windows
/// precision wheels and touchpads report a stream of point events; those are
/// accumulated to a small threshold and rate-limited so one gesture cannot tune
/// once per rendered frame.
fn wheel_direction(ui: &egui::Ui, state: &mut DigitWheelState, digit_index: usize) -> i64 {
    const POINTS_PER_STEP: f32 = 32.0;
    const POINT_STEP_INTERVAL: f64 = 0.075;
    const GESTURE_GAP: f64 = 0.18;

    let now = ui.input(|i| i.time);
    let (line_y, point_y) = ui.input(|i| {
        let mut line_y = 0.0;
        let mut point_y = 0.0;
        for event in &i.events {
            if let egui::Event::MouseWheel { unit, delta, .. } = event {
                match unit {
                    egui::MouseWheelUnit::Point => point_y += delta.y,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => line_y += delta.y,
                }
            }
        }
        (line_y, point_y)
    });

    if line_y != 0.0 {
        return line_y.signum() as i64;
    }
    if point_y == 0.0 {
        return 0;
    }

    let direction = point_y.signum() as i8;
    if now - state.last_point_wheel_at > GESTURE_GAP
        || state.point_wheel_direction != direction
        || state.point_wheel_digit != Some(digit_index)
    {
        state.point_wheel_accumulator = 0.0;
    }
    state.point_wheel_direction = direction;
    state.point_wheel_digit = Some(digit_index);
    state.last_point_wheel_at = now;
    // Do not queue a long tail of future steps after one large precision-wheel
    // event. At most one additional threshold is carried into the next event.
    state.point_wheel_accumulator = (state.point_wheel_accumulator + point_y)
        .clamp(-POINTS_PER_STEP * 2.0, POINTS_PER_STEP * 2.0);

    if state.point_wheel_accumulator.abs() >= POINTS_PER_STEP
        && now - state.last_point_step_at >= POINT_STEP_INTERVAL
    {
        state.point_wheel_accumulator -= direction as f32 * POINTS_PER_STEP;
        state.last_point_step_at = now;
        direction as i64
    } else {
        0
    }
}

/// Draw a reusable digit wheel with per-digit wheel, drag and arrow adjustment.
/// Clicking without dragging replaces the digits with a whole-value text editor.
pub fn draw_digit_wheel_widget(
    ui: &mut egui::Ui,
    id_salt: impl Hash,
    origin: Pos2,
    spec: &DigitWheelSpec<'_>,
    value: i64,
    enabled: bool,
) -> Option<i64> {
    let font = FontId::proportional(17.0);
    let label_font = FontId::proportional(12.0);
    let active_color = Color32::from_rgb(235, 235, 235);
    let inactive_color = Color32::from_rgb(90, 90, 90);
    let hover_bg = Color32::from_rgba_premultiplied(120, 120, 120, 40);
    let error_color = Color32::from_rgb(240, 90, 90);
    let label_color = Color32::from_rgb(180, 180, 180);
    let sign_color = Color32::from_rgb(210, 210, 210);

    const DIGIT_W: f32 = 13.0;
    const DIGIT_H: f32 = 24.0;
    const DIGIT_GAP: f32 = 1.0;
    const SEP_W: f32 = 7.0;
    const SIGN_W: f32 = 12.0;
    const LABEL_GAP: f32 = 8.0;
    let label_w = match spec.label {
        "LO" => 18.0,
        "LO Offset" => 54.0,
        _ => 46.0,
    };
    let widget_w = total_widget_width(label_w, LABEL_GAP, DIGIT_W, DIGIT_GAP, SEP_W, SIGN_W, spec);
    let top_left = match spec.anchor {
        DigitWheelAnchor::Left => origin,
        DigitWheelAnchor::Right => Pos2::new(origin.x - widget_w, origin.y),
    };
    let total_rect = Rect::from_min_size(top_left, Vec2::new(widget_w, DIGIT_H));
    ui.allocate_rect(total_rect, Sense::hover());

    let widget_id = ui.make_persistent_id(("digit_wheel", id_salt));
    let editor_id = widget_id.with("editor");
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<DigitWheelState>(widget_id))
        .unwrap_or_default();
    if !enabled {
        state.editing = false;
        state.drag = None;
    }

    let painter = ui.painter();
    painter.text(
        Pos2::new(top_left.x, top_left.y + DIGIT_H * 0.5),
        Align2::LEFT_CENTER,
        spec.label,
        label_font,
        if state.edit_error {
            error_color
        } else {
            label_color
        },
    );

    if state.editing && enabled {
        let edit_rect = Rect::from_min_max(
            Pos2::new(top_left.x + label_w + LABEL_GAP, top_left.y),
            total_rect.right_bottom(),
        );
        let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
            layout_editor_text(ui, text, wrap_width, &font, active_color)
        };
        let response = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut state.draft)
                .id(editor_id)
                .font(font.clone())
                .text_color(active_color)
                .margin(egui::Margin::symmetric(2, 2))
                .layouter(&mut layouter)
                .desired_width(edit_rect.width()),
        );
        if state.edit_error {
            response
                .clone()
                .on_hover_text("Enter a frequency in Hz, or use an explicit kHz/MHz suffix");
        }
        if state.focus_editor {
            response.request_focus();
            let mut editor_state =
                egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
            editor_state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::default(),
                    egui::text::CCursor::new(state.draft.chars().count()),
                )));
            editor_state.store(ui.ctx(), editor_id);
            state.focus_editor = false;
        }

        let escape = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let mut result = None;
        if escape {
            state.editing = false;
            state.edit_error = false;
            response.surrender_focus();
        } else if enter || response.lost_focus() {
            if let Some(parsed) = parse_frequency(&state.draft, spec) {
                state.editing = false;
                state.edit_error = false;
                result = (parsed != value).then_some(parsed);
            } else {
                state.edit_error = true;
                state.focus_editor = true;
            }
        }
        ui.ctx().data_mut(|d| d.insert_temp(widget_id, state));
        return result;
    }

    let digits = format_abs_digits(value.unsigned_abs(), spec.digit_count);
    let first_nonzero = first_nonzero_digit(&digits).unwrap_or(spec.digit_count - 1);
    let mut x = top_left.x + label_w + LABEL_GAP;
    if spec.signed {
        painter.text(
            Pos2::new(x + SIGN_W * 0.5, top_left.y + DIGIT_H * 0.5),
            Align2::CENTER_CENTER,
            if value < 0 { "-" } else { "+" },
            font.clone(),
            sign_color,
        );
        x += SIGN_W + DIGIT_GAP;
    }

    let mut digit_cells = Vec::with_capacity(spec.digit_count);
    let mut digit_i = 0;
    for (group_idx, group_len) in spec.groups.iter().enumerate() {
        for _ in 0..*group_len {
            let rect = Rect::from_min_size(Pos2::new(x, top_left.y), Vec2::new(DIGIT_W, DIGIT_H));
            digit_cells.push(DigitCell {
                rect,
                digit_index: digit_i,
            });
            x += DIGIT_W;
            if digit_i < spec.digit_count - 1 {
                x += DIGIT_GAP;
            }
            digit_i += 1;
        }
        if group_idx < spec.groups.len() - 1 {
            painter.text(
                Pos2::new(x + SEP_W * 0.5, top_left.y + DIGIT_H * 0.52),
                Align2::CENTER_CENTER,
                ".",
                font.clone(),
                active_color,
            );
            x += SEP_W;
        }
    }

    let mut result = None;
    let mut hovered_digit = None;
    for cell in &digit_cells {
        let response = ui
            .interact(
                cell.rect,
                widget_id.with(cell.digit_index),
                if enabled {
                    Sense::click_and_drag()
                } else {
                    Sense::hover()
                },
            )
            .on_hover_text("Scroll, drag vertically, or use ↑/↓; click to type a value");
        if enabled && response.hovered() {
            hovered_digit = Some(cell.digit_index);
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if enabled && response.drag_started() {
            state.drag = Some(DragState {
                digit_index: cell.digit_index,
                start_value: value,
                accumulated_points: 0.0,
            });
        }
        if enabled && response.dragged() {
            if let Some(drag) = state
                .drag
                .as_mut()
                .filter(|d| d.digit_index == cell.digit_index)
            {
                let next =
                    dragged_value(drag, response.drag_delta().y, spec.digit_count, spec.signed);
                result = (next != value).then_some(next);
            }
        }
        if response.drag_stopped() {
            state.drag = None;
        }
        if enabled && response.clicked() {
            state.editing = true;
            state.focus_editor = true;
            state.edit_error = false;
            state.draft = format_editor_value(value, spec);
        }

        if enabled && response.hovered() {
            painter.rect_filled(cell.rect, 3.0, hover_bg);
        }
        let color = if cell.digit_index < first_nonzero {
            inactive_color
        } else {
            active_color
        };
        painter.text(
            cell.rect.center(),
            Align2::CENTER_CENTER,
            digits[cell.digit_index] as char,
            font.clone(),
            color,
        );
    }

    if let Some(idx) = hovered_digit {
        let wheel_dir = wheel_direction(ui, &mut state, idx);
        if wheel_dir != 0 {
            let delta = digit_step(spec.digit_count, idx).saturating_mul(wheel_dir);
            let next = adjusted_value(value, delta, spec.signed);
            result = (next != value).then_some(next);
        }

        let (up, down) = ui.input_mut(|i| {
            (
                i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp),
                i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown),
            )
        });
        let key_steps = up as i64 - down as i64;
        if key_steps != 0 {
            let delta = digit_step(spec.digit_count, idx).saturating_mul(key_steps);
            let next = adjusted_value(value, delta, spec.signed);
            result = (next != value).then_some(next);
        }

        // Prevent this wheel gesture from leaking to another control in the same UI.
        ui.ctx().input_mut(|i| {
            i.raw_scroll_delta = Vec2::ZERO;
            i.smooth_scroll_delta = Vec2::ZERO;
        });
    }

    ui.ctx().data_mut(|d| d.insert_temp(widget_id, state));
    result
}

pub fn draw_lo_widget(
    ui: &mut egui::Ui,
    id_salt: impl Hash,
    top_left: Pos2,
    center_freq_hz: u64,
    enabled: bool,
) -> Option<u64> {
    let spec = DigitWheelSpec {
        label: "LO",
        digit_count: 10,
        signed: false,
        groups: &[1, 3, 3, 3],
        anchor: DigitWheelAnchor::Left,
    };
    draw_digit_wheel_widget(ui, id_salt, top_left, &spec, center_freq_hz as i64, enabled)
        .map(|v| v.max(0) as u64)
}

pub fn draw_lo_offset_widget(
    ui: &mut egui::Ui,
    id_salt: impl Hash,
    top_right: Pos2,
    offset_hz: i64,
    enabled: bool,
) -> Option<i64> {
    let spec = DigitWheelSpec {
        label: "LO Offset",
        digit_count: 6,
        signed: true,
        groups: &[3, 3],
        anchor: DigitWheelAnchor::Right,
    };
    draw_digit_wheel_widget(ui, id_salt, top_right, &spec, offset_hz, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lo_spec() -> DigitWheelSpec<'static> {
        DigitWheelSpec {
            label: "LO",
            digit_count: 10,
            signed: false,
            groups: &[1, 3, 3, 3],
            anchor: DigitWheelAnchor::Left,
        }
    }

    fn offset_spec() -> DigitWheelSpec<'static> {
        DigitWheelSpec {
            label: "LO Offset",
            digit_count: 6,
            signed: true,
            groups: &[3, 3],
            anchor: DigitWheelAnchor::Right,
        }
    }

    #[test]
    fn parses_bare_grouped_hz() {
        let spec = lo_spec();
        assert_eq!(parse_frequency("14.074.000", &spec), Some(14_074_000));
        assert_eq!(parse_frequency("145,500,000", &spec), Some(145_500_000));
        assert_eq!(parse_frequency("98000123", &spec), Some(98_000_123));
    }

    #[test]
    fn pads_incomplete_display_groups_with_trailing_zeroes() {
        let spec = lo_spec();
        assert_eq!(parse_frequency("0.098.000.", &spec), Some(98_000_000));
        assert_eq!(parse_frequency("0.098.000", &spec), Some(98_000_000));
        assert_eq!(parse_frequency("0.098.000.1", &spec), Some(98_000_100));
        assert_eq!(parse_frequency("0.098.000.12", &spec), Some(98_000_120));
    }

    #[test]
    fn parses_explicit_units() {
        let spec = lo_spec();
        assert_eq!(parse_frequency("14.074 MHz", &spec), Some(14_074_000));
        assert_eq!(parse_frequency("14074 kHz", &spec), Some(14_074_000));
        assert_eq!(parse_frequency("700 Hz", &spec), Some(700));
    }

    #[test]
    fn signedness_is_enforced() {
        assert_eq!(parse_frequency("-1500", &offset_spec()), Some(-1_500));
        assert_eq!(parse_frequency("-1500", &lo_spec()), None);
    }

    #[test]
    fn digit_places_match_display_positions() {
        assert_eq!(digit_step(10, 0), 1_000_000_000);
        assert_eq!(digit_step(10, 9), 1);
        assert_eq!(digit_step(6, 2), 1_000);
    }

    #[test]
    fn editor_value_matches_the_grouped_digit_display() {
        let lo = lo_spec();
        let offset = offset_spec();

        assert_eq!(format_editor_value(96_300_000, &lo), "0.096.300.000");
        assert_eq!(format_editor_value(1_500, &offset), "+001.500");
        assert_eq!(format_editor_value(-1_500, &offset), "-001.500");
    }

    #[test]
    fn editor_spacing_matches_the_fixed_digit_cells() {
        assert_eq!(editor_character_distance('1', '2'), Some(14.0));
        assert_eq!(editor_character_distance('1', '.'), Some(11.0));
        assert_eq!(editor_character_distance('.', '2'), Some(10.0));
        assert_eq!(editor_character_distance('+', '2'), Some(13.5));
        assert_eq!(editor_character_distance('M', 'H'), None);
    }

    #[test]
    fn drag_accumulates_frame_deltas_without_snapping_back() {
        let mut drag = DragState {
            digit_index: 7,
            start_value: 14_074_000,
            accumulated_points: 0.0,
        };

        // Two sub-step upward movements combine into one 100 Hz step.
        assert_eq!(dragged_value(&mut drag, -3.0, 10, false), 14_074_000);
        assert_eq!(dragged_value(&mut drag, -4.0, 10, false), 14_074_100);

        // A stationary frame retains the accumulated displacement and value.
        assert_eq!(dragged_value(&mut drag, 0.0, 10, false), 14_074_100);

        // Moving back to the gesture origin restores the original value.
        assert_eq!(dragged_value(&mut drag, 7.0, 10, false), 14_074_000);
    }
}

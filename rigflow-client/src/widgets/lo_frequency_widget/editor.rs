use eframe::egui::{self, Color32, FontId, Key, Modifiers, Rect};

use super::{DIGIT_GAP, DIGIT_W, DigitWheelSpec, DigitWheelState, SEP_W, SIGN_W};

pub(super) fn format_value(value: i64, spec: &DigitWheelSpec<'_>) -> String {
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

fn character_distance(current: char, next: char) -> Option<f32> {
    match (current, next) {
        (c, n) if c.is_ascii_digit() && n.is_ascii_digit() => Some(DIGIT_W + DIGIT_GAP),
        (c, '.') if c.is_ascii_digit() => Some(DIGIT_W * 0.5 + DIGIT_GAP + SEP_W * 0.5),
        ('.', n) if n.is_ascii_digit() => Some(SEP_W * 0.5 + DIGIT_W * 0.5),
        ('+' | '-', n) if n.is_ascii_digit() => Some(SIGN_W * 0.5 + DIGIT_GAP + DIGIT_W * 0.5),
        _ => None,
    }
}

fn layout_text(
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
            && let Some(center_distance) = character_distance(chars[previous], character)
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

pub(super) fn draw(
    ui: &mut egui::Ui,
    editor_id: egui::Id,
    rect: Rect,
    state: &mut DigitWheelState,
    spec: &DigitWheelSpec<'_>,
    value: i64,
    font: &FontId,
    color: Color32,
) -> Option<i64> {
    let mut layouter =
        |ui: &egui::Ui, text: &str, wrap_width: f32| layout_text(ui, text, wrap_width, font, color);
    let response = ui.put(
        rect,
        egui::TextEdit::singleline(&mut state.draft)
            .id(editor_id)
            .font(font.clone())
            .text_color(color)
            .margin(egui::Margin::symmetric(2, 2))
            .layouter(&mut layouter)
            .desired_width(rect.width()),
    );
    if state.edit_error {
        response
            .clone()
            .on_hover_text("Enter a frequency in Hz, or use an explicit kHz/MHz suffix");
    }
    if state.focus_editor {
        response.request_focus();
        let mut editor_state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
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
    if escape {
        state.editing = false;
        state.edit_error = false;
        response.surrender_focus();
        None
    } else if enter || response.lost_focus() {
        if let Some(parsed) = parse_frequency(&state.draft, spec) {
            state.editing = false;
            state.edit_error = false;
            (parsed != value).then_some(parsed)
        } else {
            state.edit_error = true;
            state.focus_editor = true;
            None
        }
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::super::{DigitWheelAnchor, DigitWheelSpec};
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
    fn value_matches_the_grouped_digit_display() {
        let lo = lo_spec();
        let offset = offset_spec();

        assert_eq!(format_value(96_300_000, &lo), "0.096.300.000");
        assert_eq!(format_value(1_500, &offset), "+001.500");
        assert_eq!(format_value(-1_500, &offset), "-001.500");
    }

    #[test]
    fn spacing_matches_the_fixed_digit_cells() {
        assert_eq!(character_distance('1', '2'), Some(14.0));
        assert_eq!(character_distance('1', '.'), Some(11.0));
        assert_eq!(character_distance('.', '2'), Some(10.0));
        assert_eq!(character_distance('+', '2'), Some(13.5));
        assert_eq!(character_distance('M', 'H'), None);
    }
}

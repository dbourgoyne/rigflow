use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};
use rigflow_core::dsp::modes::DemodMode;

use crate::ui::{
    bands::visible_radio_bands,
    frequency_view::{visible_left_hz, visible_right_hz, visible_span_hz},
    layout::{BOTTOM_GUTTER, LEFT_GUTTER, RIGHT_GUTTER, TOP_GUTTER},
    om_bands::{
        COLOR_OM_CW_ONLY, COLOR_OM_FIXED_DIGITAL, COLOR_OM_PHONE_IMAGE, COLOR_OM_RTTY_DATA,
        COLOR_OM_SSB_PHONE, COLOR_OM_USB_PHONE_CW_RTTY_DATA, LicenseClass, OmKind,
        visible_om_segments,
    },
    spectrum_utils::zoom_window,
    state::UiState,
    view_interaction::ViewMouseResult,
};

pub struct SpectrumInteraction {
    /// A bookmark overlay was clicked (takes precedence over tune/recenter).
    pub clicked_bookmark_id: Option<String>,
    /// Shared mouse interaction (tune / recenter / wheel-tune / zoom).  The
    /// caller applies it identically for the spectrum and the waterfall.
    pub mouse: ViewMouseResult,
}

pub fn draw_spectrum_plot(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    spectrum_db: &[f32],
    db_min: f32,
    db_max: f32,
    state: &UiState,
) -> SpectrumInteraction {
    let size = egui::vec2(size.x.max(300.0), size.y.max(180.0));

    let (outer_rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(outer_rect);

    painter.rect_filled(outer_rect, 4.0, Color32::from_rgb(20, 20, 24));

    let plot_rect = Rect::from_min_max(
        Pos2::new(
            outer_rect.left() + LEFT_GUTTER,
            outer_rect.top() + TOP_GUTTER,
        ),
        Pos2::new(
            outer_rect.right() - RIGHT_GUTTER,
            outer_rect.bottom() - BOTTOM_GUTTER,
        ),
    );

    if plot_rect.width() <= 1.0 || plot_rect.height() <= 1.0 {
        return empty_interaction();
    }

    let spectrum_len = spectrum_db.len();
    let pointer_pos = response.hover_pos();
    let pointer_clicked = response.clicked();

    draw_grid_and_y_axis(&painter, plot_rect, outer_rect, db_min, db_max);
    draw_x_axis(&painter, plot_rect, outer_rect, spectrum_len, state);
    let band_tooltip = draw_band_overlays(&painter, plot_rect, spectrum_len, state, pointer_pos);
    let om_tooltip = draw_om_overlays(&painter, plot_rect, spectrum_len, state, pointer_pos);
    draw_passband_overlay(&painter, plot_rect, spectrum_len, state);
    draw_trace(&painter, plot_rect, spectrum_db, db_min, db_max, state);

    let clicked_bookmark_id = draw_bookmark_overlays(
        &painter,
        plot_rect,
        spectrum_len,
        state,
        pointer_pos,
        pointer_clicked,
    );

    draw_frequency_markers(&painter, plot_rect, spectrum_len, state);

    painter.rect_stroke(
        plot_rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(110)),
        egui::StrokeKind::Inside,
    );

    // Band-strip hover tooltips are drawn last, so hover is always the topmost
    // element — above both band strips, the trace, and the plot border. (The two
    // hover zones are vertically disjoint, so at most one is Some.)
    if let Some(p) = pointer_pos {
        if let Some(lines) = &band_tooltip {
            draw_tooltip_bubble(&painter, plot_rect, lines, p);
        }
        if let Some(lines) = &om_tooltip {
            draw_tooltip_bubble(&painter, plot_rect, lines, p);
        }
    }

    // Shared mouse interaction (identical to the waterfall): wheel fine-tune /
    // zoom and click/double-click tuning.  The spectrum maps screen-x →
    // frequency via the currently-visible (zoomed) range across `plot_rect`.
    let visible_range = zoomed_visible_freq_range_hz(spectrum_len, state);
    let mut mouse = if let Some((left_hz, right_hz)) = visible_range {
        crate::ui::view_interaction::handle_view_mouse(ui, &response, plot_rect, |x| {
            let frac = ((x - plot_rect.left()) / plot_rect.width()).clamp(0.0, 1.0);
            left_hz + frac * (right_hz - left_hz)
        })
    } else {
        // No visible span: still allow wheel zoom, but no click→frequency.
        crate::ui::view_interaction::handle_view_mouse(ui, &response, plot_rect, |_| 0.0)
    };

    // Suppress click→frequency when there is no valid mapping (no visible
    // span) or when a bookmark overlay was clicked; wheel / zoom / center still
    // apply (they don't depend on the click position).
    if visible_range.is_none() || clicked_bookmark_id.is_some() {
        mouse.tune_to_hz = None;
    }

    SpectrumInteraction {
        clicked_bookmark_id,
        mouse,
    }
}

fn draw_grid_and_y_axis(
    painter: &egui::Painter,
    plot_rect: Rect,
    outer_rect: Rect,
    db_min: f32,
    db_max: f32,
) {
    let grid_color = Color32::from_gray(55);
    let text_color = Color32::from_gray(180);

    let steps = 6;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let y = egui::lerp(plot_rect.bottom()..=plot_rect.top(), t);

        painter.line_segment(
            [
                Pos2::new(plot_rect.left(), y),
                Pos2::new(plot_rect.right(), y),
            ],
            Stroke::new(1.0, grid_color),
        );

        let db = egui::lerp(db_min..=db_max, t);

        painter.text(
            Pos2::new(plot_rect.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{db:.0}"),
            FontId::monospace(11.0),
            text_color,
        );
    }

    painter.text(
        Pos2::new(outer_rect.left() + 6.0, plot_rect.top()),
        Align2::LEFT_TOP,
        "dB",
        FontId::monospace(11.0),
        text_color,
    );
}

fn draw_x_axis(
    painter: &egui::Painter,
    plot_rect: Rect,
    outer_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
) {
    let grid_color = Color32::from_gray(55);
    let text_color = Color32::from_gray(180);

    let Some((left_hz, right_hz)) = zoomed_visible_freq_range_hz(spectrum_len, state) else {
        return;
    };

    let span_hz = right_hz - left_hz;
    if span_hz <= 0.0 {
        return;
    }

    let steps = 6;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = egui::lerp(plot_rect.left()..=plot_rect.right(), t);

        painter.line_segment(
            [
                Pos2::new(x, plot_rect.top()),
                Pos2::new(x, plot_rect.bottom()),
            ],
            Stroke::new(1.0, grid_color),
        );

        let freq_hz = egui::lerp(left_hz..=right_hz, t);
        let label = format_freq(freq_hz);

        let align = if i == 0 {
            Align2::LEFT_TOP
        } else if i == steps {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };

        painter.text(
            Pos2::new(x, plot_rect.bottom() + 6.0),
            align,
            label,
            FontId::monospace(11.0),
            text_color,
        );
    }

    painter.text(
        Pos2::new(plot_rect.center().x, outer_rect.bottom() - 4.0),
        Align2::CENTER_BOTTOM,
        "Frequency",
        FontId::monospace(11.0),
        text_color,
    );
}

fn draw_trace(
    painter: &egui::Painter,
    plot_rect: Rect,
    spectrum_db: &[f32],
    db_min: f32,
    db_max: f32,
    state: &UiState,
) {
    if spectrum_db.len() < 2 || db_max <= db_min {
        return;
    }

    let (start, end) = zoom_window(spectrum_db.len(), state.display_zoom);
    let visible_len = end - start;

    if visible_len < 2 {
        return;
    }

    let mut points = Vec::with_capacity(visible_len);

    for i in 0..visible_len {
        let db = spectrum_db[start + i];
        let x_t = i as f32 / (visible_len - 1) as f32;
        let x = plot_rect.left() + x_t * plot_rect.width();

        let clamped = db.clamp(db_min, db_max);
        let y_t = (clamped - db_min) / (db_max - db_min);
        let y = plot_rect.bottom() - y_t * plot_rect.height();

        points.push(Pos2::new(x, y));
    }

    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5, Color32::LIGHT_GREEN),
    ));
}

fn format_freq(freq_hz: f32) -> String {
    if freq_hz.abs() >= 1_000_000.0 {
        format!("{:.3}", freq_hz / 1_000_000.0)
    } else if freq_hz.abs() >= 1_000.0 {
        format!("{:.1}k", freq_hz / 1_000.0)
    } else {
        format!("{:.0}", freq_hz)
    }
}

pub(crate) fn zoomed_visible_freq_range_hz(
    spectrum_len: usize,
    state: &UiState,
) -> Option<(f32, f32)> {
    if spectrum_len == 0 {
        return None;
    }

    let left_hz = visible_left_hz(state);
    let right_hz = visible_right_hz(state);
    let span_hz = right_hz - left_hz;

    if span_hz <= 0.0 {
        return None;
    }

    let (start, end) = zoom_window(spectrum_len, state.display_zoom);

    let start_frac = start as f32 / spectrum_len as f32;
    let end_frac = end as f32 / spectrum_len as f32;

    let zoom_left_hz = left_hz + start_frac * span_hz;
    let zoom_right_hz = left_hz + end_frac * span_hz;

    Some((zoom_left_hz, zoom_right_hz))
}

fn freq_to_plot_x_egui(
    freq_hz: f32,
    plot_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
) -> Option<f32> {
    let (left_hz, right_hz) = zoomed_visible_freq_range_hz(spectrum_len, state)?;

    if right_hz <= left_hz || freq_hz < left_hz || freq_hz > right_hz {
        return None;
    }

    let frac = (freq_hz - left_hz) / (right_hz - left_hz);
    Some(plot_rect.left() + frac * plot_rect.width())
}

fn draw_frequency_markers(
    painter: &egui::Painter,
    plot_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
) {
    let Some((left_hz, right_hz)) = zoomed_visible_freq_range_hz(spectrum_len, state) else {
        return;
    };

    if right_hz <= left_hz {
        return;
    }

    if let Some(target_x) =
        freq_to_plot_x_egui(state.target_freq_hz, plot_rect, spectrum_len, state)
    {
        painter.line_segment(
            [
                Pos2::new(target_x, plot_rect.top()),
                Pos2::new(target_x, plot_rect.bottom()),
            ],
            Stroke::new(1.5, Color32::from_rgb(255, 220, 80)),
        );

        let label = format!("T: {} MHz", format_mhz(state.target_freq_hz));
        let plot_center_x = plot_rect.center().x;

        let (label_pos, label_align) = if target_x > plot_center_x {
            (
                Pos2::new(target_x - 4.0, plot_rect.top() + 18.0),
                Align2::RIGHT_TOP,
            )
        } else {
            (
                Pos2::new(target_x + 4.0, plot_rect.top() + 18.0),
                Align2::LEFT_TOP,
            )
        };

        painter.text(
            label_pos,
            label_align,
            label,
            FontId::monospace(10.0),
            Color32::from_rgb(255, 220, 80),
        );
    }
}

fn draw_passband_overlay(
    painter: &egui::Painter,
    plot_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
) {
    let Some((left_hz, right_hz)) = zoomed_visible_freq_range_hz(spectrum_len, state) else {
        return;
    };

    if right_hz <= left_hz {
        return;
    }

    let target_freq_hz = state.target_freq_hz;

    // CW passband: centered at the dial ± CW pitch (CWU above, CWL below) with
    // a width set by Filter BW — NOT by the pitch.  `state.pitch_hz` holds the
    // CW pitch in CW modes; `state.filter_bandwidth_hz` is the filter width.
    //   center_offset = ±cw_pitch ; low/high = center_offset ∓ filter_bw/2
    let (pb_left_hz, pb_right_hz) = match state.demod_mode {
        DemodMode::Wfm => (target_freq_hz - 75_000.0, target_freq_hz + 75_000.0),
        DemodMode::Nfm => (target_freq_hz - 6_000.0, target_freq_hz + 6_000.0),
        DemodMode::Usb | DemodMode::DgtU => (target_freq_hz, target_freq_hz + 3_000.0),
        DemodMode::Lsb => (target_freq_hz - 3_000.0, target_freq_hz),
        DemodMode::Am => (target_freq_hz - 5_000.0, target_freq_hz + 5_000.0),
        DemodMode::Cwu => {
            let center = target_freq_hz + state.pitch_hz;
            let half = state.filter_bandwidth_hz / 2.0;
            (center - half, center + half)
        }
        DemodMode::Cwl => {
            let center = target_freq_hz - state.pitch_hz;
            let half = state.filter_bandwidth_hz / 2.0;
            (center - half, center + half)
        }
    };

    let visible_pb_left_hz = pb_left_hz.max(left_hz);
    let visible_pb_right_hz = pb_right_hz.min(right_hz);

    if visible_pb_right_hz <= visible_pb_left_hz {
        return;
    }

    let Some(x0) = freq_to_plot_x_egui(visible_pb_left_hz, plot_rect, spectrum_len, state) else {
        return;
    };
    let Some(x1) = freq_to_plot_x_egui(visible_pb_right_hz, plot_rect, spectrum_len, state) else {
        return;
    };

    let pb_rect = Rect::from_min_max(
        Pos2::new(x0, plot_rect.top()),
        Pos2::new(x1, plot_rect.bottom()),
    );

    painter.rect_filled(
        pb_rect,
        0.0,
        Color32::from_rgba_premultiplied(100, 140, 255, 40),
    );

    painter.line_segment(
        [
            Pos2::new(x0, plot_rect.top()),
            Pos2::new(x0, plot_rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgb(120, 160, 255)),
    );

    painter.line_segment(
        [
            Pos2::new(x1, plot_rect.top()),
            Pos2::new(x1, plot_rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgb(120, 160, 255)),
    );
}

pub fn x_frac_to_frequency_hz(frac: f32, state: &UiState) -> f32 {
    let frac = frac.clamp(0.0, 1.0);
    let left_hz = visible_left_hz(state);
    left_hz + frac * visible_span_hz(state)
}

/// Draw the radio-band strip. Returns the tooltip lines for the band under the
/// pointer (if any) so the caller can draw it last, on top of everything.
fn draw_band_overlays(
    painter: &egui::Painter,
    plot_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
    pointer_pos: Option<Pos2>,
) -> Option<Vec<String>> {
    let Some((left_hz, right_hz)) = zoomed_visible_freq_range_hz(spectrum_len, state) else {
        return None;
    };

    if right_hz <= left_hz {
        return None;
    }

    let visible_bands = visible_radio_bands(left_hz, right_hz);
    if visible_bands.is_empty() {
        return None;
    }

    let band_strip_height = 14.0;
    let y0 = plot_rect.bottom() - band_strip_height - 2.0;
    let y1 = plot_rect.bottom() - 2.0;

    // Track the band under the pointer for the hover tooltip.
    let mut hovered = None;

    for band in visible_bands {
        let Some(x0) = freq_to_plot_x_egui(band.start_hz, plot_rect, spectrum_len, state) else {
            continue;
        };
        let Some(x1) = freq_to_plot_x_egui(band.end_hz, plot_rect, spectrum_len, state) else {
            continue;
        };

        if x1 <= x0 {
            continue;
        }

        let color = color32_from_u32_with_alpha(band.color, 72);

        let band_rect = Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));

        painter.rect_filled(band_rect, 0.0, color);

        painter.rect_stroke(
            band_rect,
            0.0,
            Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 24)),
            egui::StrokeKind::Inside,
        );

        if (x1 - x0) >= 48.0 {
            painter.text(
                Pos2::new((x0 + x1) * 0.5, y0 + 1.0),
                Align2::CENTER_TOP,
                band.name,
                FontId::monospace(10.0),
                Color32::from_rgba_premultiplied(235, 235, 235, 180),
            );
        }

        // The strip is 14px tall, so it's easy to hover as-is; pad only downward
        // (never upward into the license-band strip's hover zone just above).
        if let Some(p) = pointer_pos {
            let hover_rect = Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1 + 2.0));
            if hover_rect.contains(p) {
                hovered = Some(band);
            }
        }
    }

    hovered.map(|band| {
        vec![
            band.name.to_string(),
            format!(
                "{}–{} MHz",
                format_mhz(band.band_start_hz),
                format_mhz(band.band_end_hz),
            ),
        ]
    })
}

fn color32_from_u32_with_alpha(rgb: u32, alpha: u8) -> Color32 {
    let r = ((rgb >> 16) & 0xff) as u8;
    let g = ((rgb >> 8) & 0xff) as u8;
    let b = (rgb & 0xff) as u8;
    Color32::from_rgba_premultiplied(r, g, b, alpha)
}

fn om_kind_color(kind: OmKind) -> u32 {
    match kind {
        OmKind::RttyData => COLOR_OM_RTTY_DATA,
        OmKind::PhoneImage => COLOR_OM_PHONE_IMAGE,
        OmKind::CwOnly => COLOR_OM_CW_ONLY,
        OmKind::SsbPhone => COLOR_OM_SSB_PHONE,
        OmKind::UsbPhoneCwRttyData => COLOR_OM_USB_PHONE_CW_RTTY_DATA,
        OmKind::FixedDigitalMessages => COLOR_OM_FIXED_DIGITAL,
    }
}

fn om_kind_label(kind: OmKind) -> &'static str {
    match kind {
        OmKind::RttyData => "RTTY/DATA",
        OmKind::PhoneImage => "PHONE",
        OmKind::CwOnly => "CW",
        OmKind::SsbPhone => "SSB",
        OmKind::UsbPhoneCwRttyData => "USB/CW/DATA",
        OmKind::FixedDigitalMessages => "DIGITAL",
    }
}

/// Full meaning of an OM segment kind, shown in the hover tooltip.
fn om_kind_full_label(kind: OmKind) -> &'static str {
    match kind {
        OmKind::RttyData => "RTTY and data",
        OmKind::PhoneImage => "Phone and image",
        OmKind::CwOnly => "CW only",
        OmKind::SsbPhone => "SSB phone",
        OmKind::UsbPhoneCwRttyData => "USB phone, CW, RTTY, and data",
        OmKind::FixedDigitalMessages => "Fixed digital message forwarding system only",
    }
}

/// Human-readable license class name for the hover tooltip.
fn license_class_name(license: LicenseClass) -> &'static str {
    match license {
        LicenseClass::AmateurExtra => "Amateur Extra",
        LicenseClass::Advanced => "Advanced",
        LicenseClass::General => "General",
        LicenseClass::Technician => "Technician",
        LicenseClass::Novice => "Novice",
    }
}

/// Draw the license (ARRL band plan) strip. Returns the tooltip lines for the
/// segment under the pointer (if any) so the caller can draw it last, on top.
fn draw_om_overlays(
    painter: &egui::Painter,
    plot_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
    pointer_pos: Option<Pos2>,
) -> Option<Vec<String>> {
    let license = state.selected_license?;

    let Some((left_hz, right_hz)) = zoomed_visible_freq_range_hz(spectrum_len, state) else {
        return None;
    };

    if right_hz <= left_hz {
        return None;
    }

    let visible_segments = visible_om_segments(left_hz, right_hz, license);
    if visible_segments.is_empty() {
        return None;
    }

    let band_strip_height = 14.0;
    let om_strip_height = band_strip_height / 3.0;
    let band_y0 = plot_rect.bottom() - band_strip_height - 2.0;
    let om_y1 = band_y0 - 1.0;
    let om_y0 = om_y1 - om_strip_height;

    // Track the segment under the pointer (last match wins, matching draw order
    // so the topmost-drawn bar of an overlap is reported) for the hover tooltip.
    let mut hovered = None;

    for seg in visible_segments {
        let Some(x0) = freq_to_plot_x_egui(seg.start_hz, plot_rect, spectrum_len, state) else {
            continue;
        };
        let Some(x1) = freq_to_plot_x_egui(seg.end_hz, plot_rect, spectrum_len, state) else {
            continue;
        };

        if x1 <= x0 {
            continue;
        }

        let color = color32_from_u32_with_alpha(om_kind_color(seg.kind), 150);

        let seg_rect = Rect::from_min_max(Pos2::new(x0, om_y0), Pos2::new(x1, om_y1));

        painter.rect_filled(seg_rect, 0.0, color);

        painter.rect_stroke(
            seg_rect,
            0.0,
            Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 32)),
            egui::StrokeKind::Inside,
        );

        if (x1 - x0) >= 40.0 {
            painter.text(
                Pos2::new((x0 + x1) * 0.5, om_y0 - 1.0),
                Align2::CENTER_BOTTOM,
                om_kind_label(seg.kind),
                FontId::monospace(9.0),
                Color32::from_rgba_premultiplied(235, 235, 235, 170),
            );
        }

        // The strip is only a few pixels tall; extend the hit area *upward*
        // (over the label) so the bar is easy to hover — but not downward, to
        // avoid overlapping the radio-band strip's hover zone just below.
        if let Some(p) = pointer_pos {
            let hover_rect = Rect::from_min_max(Pos2::new(x0, om_y0 - 8.0), Pos2::new(x1, om_y1));
            if hover_rect.contains(p) {
                hovered = Some(seg);
            }
        }
    }

    hovered.map(|seg| {
        vec![
            om_kind_full_label(seg.kind).to_string(),
            format!(
                "{}–{} MHz · {}",
                format_mhz(seg.band_start_hz),
                format_mhz(seg.band_end_hz),
                license_class_name(license),
            ),
        ]
    })
}

fn draw_bookmark_overlays(
    painter: &egui::Painter,
    plot_rect: Rect,
    spectrum_len: usize,
    state: &UiState,
    pointer_pos: Option<Pos2>,
    pointer_clicked: bool,
) -> Option<String> {
    if state.bookmarks.is_empty() {
        return None;
    }

    let Some((left_hz, right_hz)) = zoomed_visible_freq_range_hz(spectrum_len, state) else {
        return None;
    };

    if right_hz <= left_hz {
        return None;
    }

    // Position bookmark labels just above the OM strip.
    let mut clicked_bookmark_id: Option<String> = None;
    let band_strip_height = 14.0;
    let om_strip_height = band_strip_height / 3.0;
    let band_y0 = plot_rect.bottom() - band_strip_height - 2.0;
    let om_y1 = band_y0 - 1.0;
    let om_y0 = om_y1 - om_strip_height;

    let bookmark_y1 = om_y0 - 4.0;
    let bookmark_height = 18.0;
    let bookmark_y0 = bookmark_y1 - bookmark_height;

    let font_id = FontId::monospace(10.0);
    let text_color = Color32::from_rgb(255, 220, 80);
    let border_color = Color32::from_rgb(255, 220, 80);
    let fill_color = Color32::from_rgba_premultiplied(0, 0, 0, 0);

    let mut hovered_bookmark: Option<(&crate::persistence::BookmarkFile, Rect)> = None;

    for bookmark in &state.bookmarks {
        let Some(center_x) =
            freq_to_plot_x_egui(bookmark.frequency_hz, plot_rect, spectrum_len, state)
        else {
            continue;
        };

        if !center_x.is_finite() {
            continue;
        }

        let full_title = bookmark.name.trim();

        let title = if full_title.chars().count() > 16 {
            let truncated: String = full_title.chars().take(16).collect();
            format!("{truncated}...")
        } else {
            full_title.to_string()
        };

        if title.is_empty() {
            continue;
        }

        let galley = painter.layout_no_wrap(title.to_string(), font_id.clone(), text_color);

        let padding_x = 6.0;
        let rect_width = galley.size().x + padding_x * 2.0;

        let mut x0 = center_x - rect_width * 0.5;
        let mut x1 = center_x + rect_width * 0.5;

        // Clamp horizontally into the plot region.
        if x0 < plot_rect.left() {
            let delta = plot_rect.left() - x0;
            x0 += delta;
            x1 += delta;
        }
        if x1 > plot_rect.right() {
            let delta = x1 - plot_rect.right();
            x0 -= delta;
            x1 -= delta;
        }

        let rect = Rect::from_min_max(Pos2::new(x0, bookmark_y0), Pos2::new(x1, bookmark_y1));

        painter.rect_filled(rect, 4.0, fill_color);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, border_color),
            egui::StrokeKind::Inside,
        );

        painter.galley(
            Pos2::new(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            text_color,
        );

        if let Some(pointer) = pointer_pos {
            if rect.contains(pointer) {
                hovered_bookmark = Some((bookmark, rect));

                if pointer_clicked {
                    clicked_bookmark_id = Some(bookmark.id.clone());
                }
            }
        }
    }

    if let Some((bookmark, _rect)) = hovered_bookmark {
        draw_bookmark_tooltip(painter, plot_rect, bookmark, pointer_pos);
    }

    clicked_bookmark_id
}

fn draw_bookmark_tooltip(
    painter: &egui::Painter,
    plot_rect: Rect,
    bookmark: &crate::persistence::BookmarkFile,
    pointer_pos: Option<Pos2>,
) {
    let Some(pointer_pos) = pointer_pos else {
        return;
    };

    let mut lines = vec![
        bookmark.name.clone(),
        format!("{} MHz", format_mhz(bookmark.frequency_hz)),
    ];

    if let Some(notes) = &bookmark.notes {
        let trimmed = notes.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }

    draw_tooltip_bubble(painter, plot_rect, &lines, pointer_pos);
}

/// Draw a small floating tooltip bubble of text lines near the pointer, kept
/// inside `plot_rect`. Shared by the bookmark and license-band overlays.
fn draw_tooltip_bubble(
    painter: &egui::Painter,
    plot_rect: Rect,
    lines: &[String],
    pointer_pos: Pos2,
) {
    if lines.is_empty() {
        return;
    }

    let font_id = FontId::monospace(10.0);
    let text_color = Color32::from_rgb(235, 235, 235);
    let border_color = Color32::from_rgba_premultiplied(255, 255, 255, 80);
    let fill_color = Color32::from_rgba_premultiplied(28, 28, 32, 235);

    let padding = egui::vec2(8.0, 6.0);
    let line_spacing = 2.0;

    let galleys: Vec<_> = lines
        .iter()
        .map(|line| painter.layout_no_wrap(line.clone(), font_id.clone(), text_color))
        .collect();

    let max_width = galleys.iter().map(|g| g.size().x).fold(0.0_f32, f32::max);

    let total_text_height = galleys.iter().map(|g| g.size().y).sum::<f32>()
        + line_spacing * (galleys.len().saturating_sub(1) as f32);

    let bubble_size = egui::vec2(
        max_width + padding.x * 2.0,
        total_text_height + padding.y * 2.0,
    );

    let margin = 12.0;

    let preferred_x = if pointer_pos.x > plot_rect.center().x {
        // Mouse is in the right half: place bubble to the left of the pointer.
        pointer_pos.x - margin - bubble_size.x
    } else {
        // Mouse is in the left half: place bubble to the right of the pointer.
        pointer_pos.x + margin
    };

    let preferred_y = pointer_pos.y + margin;

    // Keep the bubble inside the plot when possible, but avoid invalid clamp ranges.
    let min_x = plot_rect.left() + 4.0;
    let max_x = plot_rect.right() - bubble_size.x - 4.0;
    let bubble_x = if max_x >= min_x {
        preferred_x.clamp(min_x, max_x)
    } else {
        min_x
    };

    let min_y = plot_rect.top() + 4.0;
    let max_y = plot_rect.bottom() - bubble_size.y - 4.0;
    let bubble_y = if max_y >= min_y {
        preferred_y.clamp(min_y, max_y)
    } else {
        min_y
    };

    let bubble_rect = Rect::from_min_size(Pos2::new(bubble_x, bubble_y), bubble_size);

    painter.rect_filled(bubble_rect, 6.0, fill_color);
    painter.rect_stroke(
        bubble_rect,
        6.0,
        Stroke::new(1.0, border_color),
        egui::StrokeKind::Inside,
    );

    let mut y = bubble_rect.top() + padding.y;
    for galley in galleys {
        painter.galley(
            Pos2::new(bubble_rect.left() + padding.x, y),
            galley.clone(),
            text_color,
        );
        y += galley.size().y + line_spacing;
    }
}

fn empty_interaction() -> SpectrumInteraction {
    SpectrumInteraction {
        clicked_bookmark_id: None,
        mouse: ViewMouseResult::default(),
    }
}

fn format_mhz(freq_hz: f32) -> String {
    let mhz = freq_hz / 1_000_000.0;
    format!("{mhz:.3}")
}

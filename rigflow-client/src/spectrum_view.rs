use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke};

const LEFT_GUTTER: f32 = 64.0;
const RIGHT_GUTTER: f32 = 12.0;
const TOP_GUTTER: f32 = 6.0;
const BOTTOM_GUTTER: f32 = 34.0;

pub fn draw_spectrum_plot(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    spectrum_db: &[f32],
    db_min: f32,
    db_max: f32,
    center_freq_hz: f32,
    target_freq_hz: f32,
    sample_rate_hz: f32,
) {
    let size = egui::vec2(size.x.max(300.0), size.y.max(180.0));
    let (outer_rect, _response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(outer_rect);

    // Background
    painter.rect_filled(outer_rect, 4.0, Color32::from_rgb(20, 20, 24));

    let plot_rect = Rect::from_min_max(
        Pos2::new(outer_rect.left() + LEFT_GUTTER, outer_rect.top() + TOP_GUTTER),
        Pos2::new(outer_rect.right() - RIGHT_GUTTER, outer_rect.bottom() - BOTTOM_GUTTER),
    );

    if plot_rect.width() <= 1.0 || plot_rect.height() <= 1.0 {
        return;
    }

    // Debug center line if needed
    // painter.line_segment(
    //     [Pos2::new(plot_rect.center().x, plot_rect.top()), Pos2::new(plot_rect.center().x, plot_rect.bottom())],
    //     Stroke::new(1.0, Color32::RED),
    // );

    draw_grid_and_y_axis(&painter, plot_rect, outer_rect, db_min, db_max);
    draw_x_axis(&painter, plot_rect, outer_rect, center_freq_hz, sample_rate_hz);
    draw_trace(&painter, plot_rect, spectrum_db, db_min, db_max);
    draw_frequency_markers(&painter, plot_rect, center_freq_hz, target_freq_hz, sample_rate_hz);

    // Draw only the plot border, and draw it INSIDE so it won't clip
    painter.rect_stroke(
        plot_rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(110)),
        egui::StrokeKind::Inside,
    );
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
            [Pos2::new(plot_rect.left(), y), Pos2::new(plot_rect.right(), y)],
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
    center_freq_hz: f32,
    sample_rate_hz: f32,
) {
    let grid_color = Color32::from_gray(55);
    let text_color = Color32::from_gray(180);

    if sample_rate_hz <= 0.0 {
        return;
    }

    let left_hz = center_freq_hz - sample_rate_hz * 0.5;
    let right_hz = center_freq_hz + sample_rate_hz * 0.5;

    let steps = 6;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = egui::lerp(plot_rect.left()..=plot_rect.right(), t);

        painter.line_segment(
            [Pos2::new(x, plot_rect.top()), Pos2::new(x, plot_rect.bottom())],
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
) {
    if spectrum_db.len() < 2 || db_max <= db_min {
        return;
    }

    let n = spectrum_db.len();
    let mut points = Vec::with_capacity(n);

    for (i, db) in spectrum_db.iter().enumerate() {
        let x_t = i as f32 / (n - 1) as f32;
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

fn draw_frequency_markers(
    painter: &egui::Painter,
    plot_rect: Rect,
    center_freq_hz: f32,
    target_freq_hz: f32,
    sample_rate_hz: f32,
) {
    if sample_rate_hz <= 0.0 {
        return;
    }

    let left_hz = center_freq_hz - sample_rate_hz * 0.5;
    let right_hz = center_freq_hz + sample_rate_hz * 0.5;

    // Center marker is exactly in the middle of the visible plot
    let center_x = plot_rect.center().x;

    painter.line_segment(
        [
            Pos2::new(center_x, plot_rect.top()),
            Pos2::new(center_x, plot_rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgb(120, 160, 255)),
    );

    painter.text(
        Pos2::new(center_x + 4.0, plot_rect.top() + 4.0),
        Align2::LEFT_TOP,
        "CF",
        FontId::monospace(10.0),
        Color32::from_rgb(120, 160, 255),
    );

    // Target marker is placed according to its frequency within visible bandwidth
    if target_freq_hz >= left_hz && target_freq_hz <= right_hz {
        let frac = (target_freq_hz - left_hz) / (right_hz - left_hz);
        let target_x = plot_rect.left() + frac * plot_rect.width();

        painter.line_segment(
            [
                Pos2::new(target_x, plot_rect.top()),
                Pos2::new(target_x, plot_rect.bottom()),
            ],
            Stroke::new(1.5, Color32::from_rgb(255, 220, 80)),
        );

        painter.text(
            Pos2::new(target_x + 4.0, plot_rect.top() + 18.0),
            Align2::LEFT_TOP,
            "T",
            FontId::monospace(10.0),
            Color32::from_rgb(255, 220, 80),
        );

	let tri = vec![
	    Pos2::new(target_x, plot_rect.top() + 2.0),
	    Pos2::new(target_x - 5.0, plot_rect.top() + 10.0),
	    Pos2::new(target_x + 5.0, plot_rect.top() + 10.0),
	];

	painter.add(egui::Shape::convex_polygon(
	    tri,
	    Color32::from_rgb(255, 220, 80),
	    Stroke::NONE,
	));
    }
}

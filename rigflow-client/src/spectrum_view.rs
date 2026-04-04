use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

const Y_AXIS_WIDTH: f32 = 72.0;
const X_AXIS_HEIGHT: f32 = 28.0;
const PLOT_PAD_TOP: f32 = 10.0;
const PLOT_PAD_RIGHT: f32 = 10.0;

pub fn draw_spectrum_plot(
    ui: &mut egui::Ui,
    spectrum_db: &[f32],
    db_min: f32,
    db_max: f32,
    center_freq_hz: f32,
    sample_rate_hz: f32,
) {
    let desired_size = ui.available_size();
    let desired_size = Vec2::new(desired_size.x.max(300.0), desired_size.y.max(180.0));

    let (rect, _response) = ui.allocate_exact_size(desired_size, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 20, 24));

    let plot_rect = Rect::from_min_max(
        Pos2::new(rect.left() + Y_AXIS_WIDTH, rect.top() + PLOT_PAD_TOP),
        Pos2::new(rect.right() - PLOT_PAD_RIGHT, rect.bottom() - X_AXIS_HEIGHT),
    );

    // begin debug
    painter.rect_stroke(
	rect,
	0.0,
	Stroke::new(1.0, Color32::RED),
	egui::StrokeKind::Middle,
    );

    painter.rect_stroke(
	plot_rect,
	0.0,
	Stroke::new(1.0, Color32::YELLOW),
	egui::StrokeKind::Middle,
    );
    // end debug

    if plot_rect.width() <= 1.0 || plot_rect.height() <= 1.0 {
        return;
    }

    draw_db_axis_and_grid(&painter, rect, plot_rect, db_min, db_max);
    draw_freq_axis_and_grid(&painter, rect, plot_rect, center_freq_hz, sample_rate_hz);
    draw_plot_border(&painter, plot_rect);
    draw_trace(&painter, plot_rect, spectrum_db, db_min, db_max);
}

fn draw_plot_border(painter: &egui::Painter, plot_rect: Rect) {
    painter.rect_stroke(
        plot_rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Middle,
    );
}

fn draw_db_axis_and_grid(
    painter: &egui::Painter,
    full_rect: Rect,
    plot_rect: Rect,
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
	    format!("{db:.0} dB"),
	    FontId::monospace(12.0),
	    text_color,
	);
    }

    painter.text(
        Pos2::new(full_rect.left() + 8.0, plot_rect.top() - 2.0),
        Align2::LEFT_TOP,
        "dB",
        FontId::monospace(11.0),
        text_color,
    );
}

fn draw_freq_axis_and_grid(
    painter: &egui::Painter,
    _full_rect: Rect,
    plot_rect: Rect,
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

        painter.text(
            Pos2::new(x, plot_rect.bottom() + 6.0),
            Align2::CENTER_TOP,
            format_freq(freq_hz),
            FontId::monospace(11.0),
            text_color,
        );
    }

    painter.text(
        Pos2::new(plot_rect.center().x, plot_rect.bottom() + 20.0),
        Align2::CENTER_TOP,
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
        format!("{:.3} MHz", freq_hz / 1_000_000.0)
    } else if freq_hz.abs() >= 1_000.0 {
        format!("{:.1} kHz", freq_hz / 1_000.0)
    } else {
        format!("{:.0} Hz", freq_hz)
    }
}

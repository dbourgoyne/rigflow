use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

const Y_AXIS_WIDTH: f32 = 72.0;
const X_AXIS_HEIGHT: f32 = 28.0;
const PLOT_PAD_TOP: f32 = 10.0;
const PLOT_PAD_RIGHT: f32 = 10.0;

pub fn draw_spectrum_plot(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    spectrum_db: &[f32],
    db_min: f32,
    db_max: f32,
    center_freq_hz: f32,
    sample_rate_hz: f32,
) {
    let size = egui::vec2(size.x.max(300.0), size.y.max(180.0));

    let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter_at(rect);

    // Full widget background
    painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 20, 24));

    // NEW: create inner content rect so the plot is not flush to the edges
    let content_rect = rect.shrink2(egui::vec2(12.0, 8.0));

    let plot_rect = egui::Rect::from_min_max(
        egui::Pos2::new(content_rect.left() + Y_AXIS_WIDTH, content_rect.top() + PLOT_PAD_TOP),
        egui::Pos2::new(content_rect.right() - PLOT_PAD_RIGHT, content_rect.bottom() - X_AXIS_HEIGHT),
    );

    // DEBUG: draw only these
    painter.rect_stroke(
        content_rect,
        0.0,
        egui::Stroke::new(1.0, egui::Color32::RED),
        egui::StrokeKind::Middle,
    );

    painter.rect_stroke(
        plot_rect,
        0.0,
        egui::Stroke::new(1.0, egui::Color32::YELLOW),
        egui::StrokeKind::Middle,
    );

    if plot_rect.width() <= 1.0 || plot_rect.height() <= 1.0 {
        return;
    }

    draw_db_axis_and_grid(&painter, content_rect, plot_rect, db_min, db_max);
    draw_freq_axis_and_grid(&painter, content_rect, plot_rect, center_freq_hz, sample_rate_hz);
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
    content_rect: egui::Rect,
    plot_rect: egui::Rect,
    db_min: f32,
    db_max: f32,
) {
    let grid_color = egui::Color32::from_gray(55);
    let text_color = egui::Color32::from_gray(180);

    let steps = 6;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let y = egui::lerp(plot_rect.bottom()..=plot_rect.top(), t);

        painter.line_segment(
            [egui::Pos2::new(plot_rect.left(), y), egui::Pos2::new(plot_rect.right(), y)],
            egui::Stroke::new(1.0, grid_color),
        );

        let db = egui::lerp(db_min..=db_max, t);

        painter.text(
            egui::Pos2::new(plot_rect.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{db:.0}"),
            egui::FontId::monospace(12.0),
            text_color,
        );
    }

    painter.text(
        egui::Pos2::new(content_rect.left() + 4.0, plot_rect.top()),
        egui::Align2::LEFT_TOP,
        "dB",
        egui::FontId::monospace(11.0),
        text_color,
    );
}

fn draw_freq_axis_and_grid(
    painter: &egui::Painter,
    _content_rect: egui::Rect,
    plot_rect: egui::Rect,
    center_freq_hz: f32,
    sample_rate_hz: f32,
) {
    let grid_color = egui::Color32::from_gray(55);
    let text_color = egui::Color32::from_gray(180);

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
            [egui::Pos2::new(x, plot_rect.top()), egui::Pos2::new(x, plot_rect.bottom())],
            egui::Stroke::new(1.0, grid_color),
        );

        let freq_hz = egui::lerp(left_hz..=right_hz, t);

        painter.text(
            egui::Pos2::new(x, plot_rect.bottom() + 6.0),
            egui::Align2::CENTER_TOP,
            format_freq(freq_hz),
            egui::FontId::monospace(11.0),
            text_color,
        );
    }

    painter.text(
        egui::Pos2::new(plot_rect.center().x, plot_rect.bottom() + 22.0),
        egui::Align2::CENTER_TOP,
        "Frequency",
        egui::FontId::monospace(11.0),
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

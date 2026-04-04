use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};

pub fn draw_spectrum_trace(
    ui: &mut egui::Ui,
    spectrum_db: &[f32],
    db_min: f32,
    db_max: f32,
) {
    let desired_size = ui.available_size();
    let desired_size = Vec2::new(desired_size.x.max(200.0), desired_size.y.max(120.0));

    let (rect, _response) = ui.allocate_exact_size(desired_size, Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 4.0, Color32::from_rgb(20, 20, 24));

    if spectrum_db.len() < 2 || db_max <= db_min {
        return;
    }

    let width = rect.width().max(1.0);
    let height = rect.height().max(1.0);
    let n = spectrum_db.len();

    let mut points = Vec::with_capacity(n);

    for (i, db) in spectrum_db.iter().enumerate() {
        let x_t = i as f32 / (n - 1) as f32;
        let x = rect.left() + x_t * width;

        let clamped = db.clamp(db_min, db_max);
        let y_t = (clamped - db_min) / (db_max - db_min);
        let y = rect.bottom() - y_t * height;

        points.push(Pos2::new(x, y));
    }

    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5, Color32::LIGHT_GREEN),
    ));

    draw_db_grid(&painter, rect, db_min, db_max);
}

fn draw_db_grid(painter: &egui::Painter, rect: Rect, db_min: f32, db_max: f32) {
    let grid_color = Color32::from_gray(60);
    let text_color = Color32::from_gray(160);

    let steps = 6;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let y = egui::lerp(rect.bottom()..=rect.top(), t);

        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, grid_color),
        );

        let db = egui::lerp(db_min..=db_max, t);
        painter.text(
            Pos2::new(rect.left() + 6.0, y - 8.0),
            egui::Align2::LEFT_TOP,
            format!("{db:.0} dB"),
            egui::FontId::monospace(11.0),
            text_color,
        );
    }
}

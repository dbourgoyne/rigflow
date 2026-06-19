use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;

/// Styling for a **top-level** left-panel section header (Radio Operator,
/// Radios, Radio Control, …).  Larger + the operator-name accent colour so the
/// main menus stand out from their sub-sections (which keep the default style).
pub(crate) fn panel_header(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .size(16.0)
        .strong()
        .color(egui::Color32::from_rgb(90, 200, 255))
}

/// Soft-amber, body-size styling for explanatory note / help captions — the short
/// sentences that explain a control or panel. Readable and visually distinct from
/// white control labels, replacing the old hard-to-read `.small().weak()` styling.
pub(crate) fn note_text(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(egui::Color32::from_rgb(230, 200, 120))
}

/// Let the mouse wheel adjust a slider while the pointer is over it: scroll up
/// increases, scroll down decreases, by `step` (clamped to `min..=max`). Marks the
/// response changed so the caller's existing `.changed()` handling runs. Returns
/// `true` if the wheel changed the value (useful when the caller persists on
/// `drag_stopped()` rather than `changed()`). Call right after
/// `ui.add(Slider::new(&mut value, min..=max)…)`.
///
/// While the pointer is over the slider this **swallows all wheel scroll** so the
/// surrounding panel `ScrollArea` never scrolls and the slider stays under the
/// cursor. The step is taken from `raw_scroll_delta` (one notch per frame), but
/// both the raw and the multi-frame `smooth_scroll_delta` (which the `ScrollArea`
/// actually consumes) are zeroed every hovered frame — otherwise the smoothed tail
/// of a notch keeps scrolling the panel on the frames after the step.
pub(crate) fn slider_scroll<Num: egui::emath::Numeric>(
    ui: &egui::Ui,
    response: &mut egui::Response,
    value: &mut Num,
    min: f64,
    max: f64,
    step: f64,
) -> bool {
    if !response.hovered() {
        return false;
    }

    // Step the value once per wheel notch (raw delta is per-frame).
    let raw_y = ui.input(|i| i.raw_scroll_delta.y);
    let mut changed = false;
    if raw_y != 0.0 {
        let (lo, hi) = (min.min(max), min.max(max));
        let cur = (*value).to_f64();
        let next = (cur + step * raw_y.signum() as f64).clamp(lo, hi);
        if next != cur {
            *value = Num::from_f64(next);
            response.mark_changed();
            changed = true;
        }
    }

    // Swallow wheel scroll for as long as the slider is hovered — including the
    // smoothed tail on frames where `raw_scroll_delta` is already zero — so the
    // panel ScrollArea (which reads `smooth_scroll_delta`) never moves.
    ui.ctx().input_mut(|i| {
        i.raw_scroll_delta = egui::Vec2::ZERO;
        i.smooth_scroll_delta = egui::Vec2::ZERO;
    });

    changed
}

mod bookmarks;
mod latency;
mod operator;
mod problems;
mod radio_control;
mod radios;
mod sections;

/// Shared S-meter label formatter (also used by the top status bar).
pub(crate) use radio_control::s_meter_label;
mod server;
mod source_control;
mod source_status;
mod tx_tune_test;
mod waterfall_control;

impl RigflowApp {
    pub(crate) fn draw_left_panel(
        &mut self,
        ctx: &egui::Context,
        snapshot: &UiState,
        config_mode: bool,
    ) {
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(260.0)
            .show(ctx, |ui| {
                ui.heading("rigflow");
                ui.separator();

                // Status console docked at the bottom of the side panel (fixed
                // height, internally scrollable).  Reserved here, before the
                // settings scroll area, so that area fills the space above it.
                let row_h = ui.text_style_height(&egui::TextStyle::Body);
                egui::TopBottomPanel::bottom("status_console")
                    .resizable(false)
                    .exact_height(row_h * 5.0 + 10.0)
                    .show_inside(ui, |ui| self.draw_status_console(ui, snapshot));

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // First-run / not-connected cue: point a new user at the
                        // two steps to get on the air (the panels below are open
                        // pre-connect).  Hidden once connected.
                        if !snapshot.server_connected {
                            let cue = if snapshot.known_operator_ids.is_empty() {
                                "Getting started: add an operator below, then enter your server IP and Connect."
                            } else {
                                "Not connected — enter your server IP and click Connect."
                            };
                            ui.colored_label(egui::Color32::from_rgb(255, 190, 70), cue);
                            ui.separator();
                        }

                        self.draw_operator_panel(ui, snapshot, config_mode);
                        ui.separator();
                        self.draw_server_panel(ui, snapshot, config_mode);
                        ui.separator();
                        self.draw_radios_panel(ui, snapshot);
                        self.draw_radio_control_panel(ui, snapshot);
                        ui.separator();
                        self.draw_source_control_panel(ui, snapshot);
                        ui.separator();
                        self.draw_waterfall_control_panel(ui);
                        ui.separator();
                        self.draw_latency_panel(ui, snapshot);
                        ui.separator();
                        self.draw_bookmarks_panel(ui);
                        ui.separator();
                    });
            });
    }
}

use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;

mod bookmarks;
mod operator;
mod problems;
mod radio_control;
mod radios;

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
                        self.draw_bookmarks_panel(ui);
                        ui.separator();
                    });
            });
    }
}

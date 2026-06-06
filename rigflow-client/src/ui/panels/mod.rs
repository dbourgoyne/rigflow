use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;

mod bookmarks;
mod operator;
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

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
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

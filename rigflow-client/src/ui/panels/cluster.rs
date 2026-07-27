use eframe::egui;

use crate::ui::app::RigflowApp;
use crate::ui::app_cluster::cluster_status_display;
use crate::ui::state::UiState;

impl RigflowApp {
    /// Compact left-panel section: at-a-glance DX-cluster status + a button to
    /// open the full config window. The spots themselves render on the
    /// spectrum/waterfall, not here.
    pub(crate) fn draw_cluster_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        ui.collapsing(super::panel_header("DX Cluster"), |ui| {
            ui.horizontal(|ui| {
                ui.label("Status:");
                let (txt, col) = cluster_status_display(&snapshot.dx_cluster_status);
                ui.label(egui::RichText::new(txt).color(col));
            });
            if ui.button("Configure…").clicked() {
                self.open_cluster(&snapshot.operator_id);
            }
        });
    }
}

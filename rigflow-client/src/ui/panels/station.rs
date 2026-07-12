use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;

impl RigflowApp {
    /// The **Station** panel: the physical station's location, shared across all
    /// operators (one rig, one grid). The callsign is the operator id, not here.
    /// These values feed the `MY_*` fields snapshotted onto each logged QSO.
    ///
    /// Not gated by the connected/config lock: it's global config (in
    /// `app_state.json`), sends nothing to the server, and the operator may need
    /// to set their grid at any time.
    pub(crate) fn draw_station_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        egui::CollapsingHeader::new(super::panel_header("Station"))
            .default_open(false)
            .show(ui, |ui| {
                ui.label("Shared station location — applies to all operators.");
                ui.add_space(4.0);

                let mut p = snapshot.station_profile.clone();
                let mut changed = false;
                let mut committed = false;
                let mut field = |ui: &mut egui::Ui, label: &str, val: &mut String| {
                    ui.label(label);
                    let r = ui.text_edit_singleline(val);
                    changed |= r.changed();
                    committed |= r.lost_focus();
                    ui.end_row();
                };

                egui::Grid::new("station_profile_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        field(ui, "Grid square", &mut p.gridsquare);
                        field(ui, "State", &mut p.state);
                        field(ui, "County", &mut p.county);
                        field(ui, "CQ zone", &mut p.cq_zone);
                        field(ui, "ITU zone", &mut p.itu_zone);
                    });

                // Mirror edits into UiState live; only persist to disk when a
                // field loses focus (so we don't rewrite app_state.json per key).
                if changed || committed {
                    if let Ok(mut s) = self.state.lock() {
                        s.station_profile = p;
                        if committed {
                            s.pending_save_station_profile = true;
                        }
                    }
                }

                ui.add_space(6.0);
                let call = snapshot.operator_id.trim();
                if call.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(255, 190, 70),
                        "Set an operator — its ID is your logging callsign.",
                    );
                } else {
                    ui.label(format!("Logging callsign: {}", call.to_uppercase()));
                }
            });
    }
}

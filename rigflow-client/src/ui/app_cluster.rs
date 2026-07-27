//! DX-cluster app glue (Phase 3): per-operator config load/save, applying the
//! config to the telnet thread (connect/disconnect), and producing the filtered
//! spot list the waterfall/spectrum renderer draws (Phase 4). The config window
//! itself is Phase 5.

use eframe::egui::{self, Color32};
use rigflow_core::radio::ham_band::band_from_frequency;

use crate::cluster::DxSpot;
use crate::cluster::client::DxClusterStatus;
use crate::cluster::config;
use crate::ui::app::RigflowApp;
use crate::ui::panels::note_text;

/// Common modes offered as filter toggles in the config window.
const FILTER_MODES: [&str; 6] = ["CW", "SSB", "FT8", "FT4", "RTTY", "PSK"];

/// Status text + colour for the connection state (colour, not size — no glyphs).
pub(crate) fn cluster_status_display(status: &DxClusterStatus) -> (String, Color32) {
    match status {
        DxClusterStatus::Disconnected => ("disconnected".to_string(), Color32::from_gray(160)),
        DxClusterStatus::Connecting => ("connecting…".to_string(), Color32::from_rgb(235, 200, 90)),
        DxClusterStatus::Connected => ("connected".to_string(), Color32::from_rgb(120, 205, 140)),
        DxClusterStatus::Error(e) => (format!("{e} — retrying"), Color32::from_rgb(230, 130, 120)),
    }
}

impl RigflowApp {
    /// Load this operator's cluster config, once per operator.
    pub(crate) fn load_cluster_config(&mut self, operator_id: &str) {
        if self.dx_cluster_loaded_for.as_deref() == Some(operator_id) {
            return;
        }
        let dir = self.operator_dir(operator_id);
        self.dx_cluster_config = config::load_config(&dir);
        self.dx_cluster_loaded_for = Some(operator_id.to_string());
    }

    /// Persist this operator's cluster config.
    pub(crate) fn save_cluster_config(&mut self, operator_id: &str) {
        let dir = self.operator_dir(operator_id);
        if let Err(e) = config::save_config(&dir, &self.dx_cluster_config) {
            self.set_log_status(format!("cluster config save failed: {e}"));
        }
    }

    /// Drive the telnet thread from the current config: connect when enabled
    /// (with a resolvable login callsign), disconnect otherwise. Idempotent —
    /// safe to call after any config change or on an operator switch.
    pub(crate) fn apply_cluster_connection(&mut self, operator_id: &str) {
        let call = self.dx_cluster_config.login_call(operator_id);
        if self.dx_cluster_config.enabled && !call.is_empty() {
            self.dx_cluster.connect(
                self.dx_cluster_config.host.clone(),
                self.dx_cluster_config.port,
                call,
            );
        } else {
            self.dx_cluster.disconnect();
        }
    }

    /// The spots the renderer should draw: the shared book filtered by the
    /// config (current-band / mode) against the currently tuned band. Cloned
    /// once per frame — filtered to the current band this is a small set.
    pub(crate) fn visible_spots(&self) -> Vec<DxSpot> {
        let current_band = {
            let s = self.state.lock().unwrap();
            band_from_frequency(s.target_freq_hz.max(0.0) as u64)
        };
        let cfg = &self.dx_cluster_config;
        let Ok(spots) = self.dx_cluster.spots.lock() else {
            return Vec::new();
        };
        spots
            .iter()
            .filter(|s| cfg.show(s, current_band))
            .cloned()
            .collect()
    }

    /// Open the DX-cluster config window for this operator.
    pub(crate) fn open_cluster(&mut self, operator_id: &str) {
        self.show_cluster = true;
        self.load_cluster_config(operator_id);
    }

    /// On an operator switch (or first frame), load that operator's cluster
    /// config and apply it — auto-connecting when they've enabled the feature,
    /// disconnecting otherwise. A no-op while the operator is unchanged.
    pub(crate) fn sync_cluster_state(&mut self, operator_id: &str) {
        if self.dx_cluster_synced_for.as_deref() == Some(operator_id) {
            return;
        }
        if operator_id.trim().is_empty() {
            self.dx_cluster.disconnect();
        } else {
            self.load_cluster_config(operator_id);
            self.apply_cluster_connection(operator_id);
        }
        self.dx_cluster_synced_for = Some(operator_id.to_string());
    }

    /// The DX-cluster configuration window: node, login callsign, display
    /// filter, and connect/disconnect. Editing is locked while connected to a
    /// rigflow server (operator-settings rule).
    pub(crate) fn draw_cluster_window(&mut self, ctx: &egui::Context, operator_id: &str) {
        if !self.show_cluster {
            return;
        }
        if operator_id.trim().is_empty() {
            self.show_cluster = false;
            return;
        }
        self.load_cluster_config(operator_id);

        let (locked, status) = {
            let s = self.state.lock().unwrap();
            (s.config_locked, s.dx_cluster_status.clone())
        };

        let mut open = true;
        let mut save = false;
        let mut disconnect = false;

        egui::Window::new("DX Cluster")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label(note_text(
                    "Connect to a DX-cluster node and overlay live spots on the spectrum and \
                     waterfall. Click a spot to tune to it.",
                ));
                ui.separator();

                ui.horizontal(|ui| {
                    ui.strong("Status:");
                    let (txt, col) = cluster_status_display(&status);
                    ui.label(egui::RichText::new(txt).color(col));
                });
                ui.separator();

                if locked {
                    ui.label(note_text(
                        "Locked while connected to a rigflow server — disconnect from the \
                         server to change cluster settings.",
                    ));
                }

                ui.add_enabled_ui(!locked, |ui| {
                    ui.checkbox(&mut self.dx_cluster_config.enabled, "Enabled");

                    let current = self.dx_cluster_config.matching_node().unwrap_or("Custom");
                    egui::ComboBox::from_label("Node")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for n in config::NODES {
                                if ui.selectable_label(current == n.name, n.name).clicked() {
                                    self.dx_cluster_config.host = n.host.to_string();
                                    self.dx_cluster_config.port = n.port;
                                }
                            }
                            // "Custom" leaves host/port as-is for manual editing.
                            let _ =
                                ui.selectable_label(current == "Custom", "Custom (edit host/port)");
                        });

                    egui::Grid::new("cluster_fields")
                        .num_columns(2)
                        .spacing([8.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Host");
                            ui.text_edit_singleline(&mut self.dx_cluster_config.host);
                            ui.end_row();
                            ui.label("Port");
                            let mut port_str = self.dx_cluster_config.port.to_string();
                            if ui.text_edit_singleline(&mut port_str).changed()
                                && let Ok(p) = port_str.trim().parse::<u16>()
                            {
                                self.dx_cluster_config.port = p;
                            }
                            ui.end_row();
                            ui.label("Callsign");
                            ui.text_edit_singleline(&mut self.dx_cluster_config.call);
                            ui.end_row();
                        });
                    ui.label(note_text(
                        "Callsign is your login — public nodes have no password. Leave blank to \
                         use your operator callsign.",
                    ));

                    ui.separator();
                    ui.checkbox(
                        &mut self.dx_cluster_config.current_band_only,
                        "Show only the currently tuned band",
                    );
                    ui.horizontal(|ui| {
                        ui.label("Modes:");
                        for m in FILTER_MODES {
                            let mut on = self.dx_cluster_config.modes.iter().any(|x| x == m);
                            if ui.checkbox(&mut on, m).changed() {
                                self.dx_cluster_config.modes.retain(|x| x != m);
                                if on {
                                    self.dx_cluster_config.modes.push(m.to_string());
                                }
                            }
                        }
                    });
                    ui.label(note_text("No modes selected = all modes."));
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!locked, egui::Button::new("Save & apply"))
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add_enabled(!locked, egui::Button::new("Disconnect"))
                        .clicked()
                    {
                        disconnect = true;
                    }
                });
            });

        if save {
            self.save_cluster_config(operator_id);
            self.apply_cluster_connection(operator_id);
        }
        if disconnect {
            self.dx_cluster_config.enabled = false;
            self.save_cluster_config(operator_id);
            self.dx_cluster.disconnect();
        }
        if !open {
            self.show_cluster = false;
        }
    }
}

use std::time::{Duration, Instant};

use eframe::egui;

use crate::ui::app::RigflowApp;
use crate::ui::app_cluster::cluster_status_display;
use crate::ui::state::UiState;

impl RigflowApp {
    /// Left-panel "DX Cluster" section: status, a receive/shown count, and a
    /// scrollable **spot list** (band map). The list is the primary way to see
    /// spots — the spectrum/waterfall markers only cover the narrow visible
    /// span, whereas the list shows every spot passing the filter and each row
    /// clicks to tune (recentering on it).
    pub(crate) fn draw_cluster_panel(&mut self, ui: &mut egui::Ui, snapshot: &UiState) {
        ui.collapsing(super::panel_header("DX Cluster"), |ui| {
            ui.horizontal(|ui| {
                ui.label("Status:");
                let (txt, col) = cluster_status_display(&snapshot.dx_cluster_status);
                ui.label(egui::RichText::new(txt).color(col));
            });

            // Received vs. shown, so "connected but nothing on screen" is
            // diagnosable: 0 received = not arriving/parsing; received > shown =
            // hidden by the band/mode filter.
            let total = self.total_spots();
            let mut spots = self.visible_spots();
            ui.label(format!("Spots: {total} received · {} shown", spots.len()));

            if total == 0 {
                ui.label(super::note_text(
                    "No spots yet — they arrive as stations are spotted. Check your \
                     connection if this stays at zero on a busy band.",
                ));
                if ui.button("Configure…").clicked() {
                    self.open_cluster(&snapshot.operator_id);
                }
                return;
            }

            if spots.is_empty() {
                ui.label(super::note_text(
                    "Spots received but none pass the current filter — turn off \
                     \"current band only\" or clear the mode filter in Configure.",
                ));
            } else {
                // Sort by frequency so the list reads like a band map.
                spots.sort_by_key(|s| s.freq_hz);
                let now = Instant::now();
                // Inset the list so its scrollbar sits clear of the left panel's
                // own scrollbar at the pane edge (otherwise the two merge and the
                // list looks like it doesn't scroll).
                let list_width = (ui.available_width() - 18.0).max(120.0);
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .max_width(list_width)
                    .auto_shrink([false, true])
                    // Always show the scrollbar so it's obvious the list scrolls
                    // (the default floating bar only appears on hover).
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        for spot in &spots {
                            let mhz = spot.freq_hz as f64 / 1_000_000.0;
                            let age = fmt_age(now.saturating_duration_since(spot.received_at));
                            let mode = spot
                                .mode_hint
                                .as_deref()
                                .map(|m| format!("  {m}"))
                                .unwrap_or_default();
                            let label = format!("{mhz:9.3}  {:<10}{mode}   {age}", spot.dx_call);
                            if ui
                                .add(
                                    egui::Button::new(egui::RichText::new(label).monospace())
                                        .frame(false),
                                )
                                .on_hover_text(format!(
                                    "de {} · {}",
                                    spot.spotter,
                                    if spot.comment.is_empty() {
                                        "—"
                                    } else {
                                        &spot.comment
                                    }
                                ))
                                .clicked()
                            {
                                self.tune_to_cluster_spot(spot.freq_hz);
                            }
                        }
                    });
            }

            if ui.button("Configure…").clicked() {
                self.open_cluster(&snapshot.operator_id);
            }
        });
    }
}

/// Compact age like `12s`, `4m`, `1h`.
fn fmt_age(age: Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

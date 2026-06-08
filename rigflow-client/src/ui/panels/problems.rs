//! "Status" console — a fixed, always-present list of active subsystem failures
//! (rigctl bind, digital/PipeWire unavailability, amplifier serial open, server
//! connection, radio not responding, …), docked at the bottom of the left side
//! panel so failures surface on screen instead of only in the log.  The list is
//! derived from the `UiState` snapshot by [`collect_problems`], the same source
//! the top-bar status LED uses.

use crate::UiState;
use crate::ui::app::RigflowApp;
use crate::ui::state::{ProblemSeverity, collect_problems};
use eframe::egui;

/// Red for errors, amber for warnings, green for the all-clear line.
const COLOR_ERROR: egui::Color32 = egui::Color32::from_rgb(210, 130, 130);
const COLOR_WARNING: egui::Color32 = egui::Color32::from_rgb(255, 160, 40);
const COLOR_OK: egui::Color32 = egui::Color32::from_rgb(100, 200, 100);

impl RigflowApp {
    /// Draw the status console body (a heading + a scrollable problem list) to
    /// fill its container.  The container (a fixed-height bottom panel) decides
    /// the size; the inner scroll area lets a long list be read in full.
    pub(crate) fn draw_status_console(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        let problems = collect_problems(snapshot);

        ui.label(egui::RichText::new("Status").strong());

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if problems.is_empty() {
                    ui.colored_label(COLOR_OK, "All subsystems OK");
                    return;
                }
                for p in &problems {
                    let color = match p.severity {
                        ProblemSeverity::Error => COLOR_ERROR,
                        ProblemSeverity::Warning => COLOR_WARNING,
                    };
                    ui.colored_label(color, format!("{}: {}", p.source, p.detail));
                }
            });
    }
}

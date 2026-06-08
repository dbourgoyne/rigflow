//! "Status / Problems" panel — an always-open list of active subsystem
//! failures (rigctl bind, digital/PipeWire unavailability, amplifier serial
//! open, server connection), so failures surface on screen instead of only in
//! the log.  The list is derived from the `UiState` snapshot by
//! [`collect_problems`], the same source the status-bar badge uses.

use crate::UiState;
use crate::ui::app::RigflowApp;
use crate::ui::state::{ProblemSeverity, collect_problems};
use eframe::egui;

/// Red for errors, orange for warnings, green for the all-clear line.
const COLOR_ERROR: egui::Color32 = egui::Color32::from_rgb(210, 130, 130);
const COLOR_WARNING: egui::Color32 = egui::Color32::from_rgb(255, 160, 40);
const COLOR_OK: egui::Color32 = egui::Color32::from_rgb(100, 200, 100);

impl RigflowApp {
    pub(crate) fn draw_problems_panel(&self, ui: &mut egui::Ui, snapshot: &UiState) {
        let problems = collect_problems(snapshot);

        egui::CollapsingHeader::new("Status / Problems")
            .id_salt("status_problems")
            .default_open(true)
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
                    ui.colored_label(
                        color,
                        egui::RichText::new(format!("{}: {}", p.source, p.detail)).small(),
                    );
                }
            });
    }
}

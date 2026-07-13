//! The ADIF **import** window, opened from the contact view.
//!
//! Two phases, mirroring the library ([`rigflow_log::import`]):
//!
//! 1. **Plan** — pick a file; the worker parses, normalizes, validates and
//!    dedupes it against the log on its **read-only** connection, and reports
//!    *"1,483 records · 1,471 to import · 12 duplicates (skipped) · 3 unusable"*.
//!    Nothing has been written. The operator sees what the file would do to their
//!    log before agreeing to it — the same promise the export window makes.
//! 2. **Commit** — one transaction, one journal write, one fsync, all-or-nothing,
//!    on the UI thread's read-write `LogStore`.
//!
//! Duplicates are skipped on the same ±30-minute rule the WSJT-X path uses, which
//! makes import **idempotent**: importing the same file twice adds nothing the
//! second time. Bad records are skipped and named, never fatal — a twenty-year log
//! from another program will have cruft, and refusing 20,000 contacts over three
//! junk rows would be hostile.

use eframe::egui;

use crate::logging::export::ExportJob;
use crate::ui::app::RigflowApp;
use crate::ui::app_export::{NO_PICKER_HINT, file_picker_available};
use crate::ui::panels::note_text;

impl RigflowApp {
    /// Open the import window (empty — the operator picks a file next).
    pub(crate) fn open_import(&mut self) {
        self.import_file = None;
        self.import_plan = None;
        self.import_planning = false;
        self.import_status.clear();
        self.show_import = true;
    }

    /// Hand a chosen file to the worker to plan.
    fn plan_import(&mut self, operator_id: &str, file: std::path::PathBuf) {
        self.import_plan = None;
        self.import_planning = true;
        self.import_status.clear();
        self.import_file = Some(file.clone());
        let _ = self.export_tx.send(ExportJob::PlanImport {
            db_path: self.persistence_store.qso_log_db_path(operator_id),
            file,
        });
    }

    /// Commit the planned contacts.
    ///
    /// Runs on the UI thread, on the store this thread owns. It is a single
    /// transaction with one fsync — fast enough that even a large log lands in
    /// well under a second — so it does not need the worker, and putting it there
    /// would mean a second writer on the same database for no gain.
    fn commit_import(&mut self) {
        let Some(plan) = self.import_plan.take() else {
            return;
        };
        let (op, name, profile) = {
            let s = self.state.lock().unwrap();
            (
                s.operator_id.clone(),
                s.operator_name.clone(),
                s.station_profile.clone(),
            )
        };
        let station = profile.to_log_station(&op, &name);

        let Some(store) = self.log.as_mut() else {
            self.import_status = "no operator selected — nothing imported".to_string();
            return;
        };

        match store.commit_import(&plan.importable, &station) {
            Ok(outcome) => {
                // The worked-before index and the contact list both describe a log
                // that just changed underneath them.
                self.worked_before = store.load_worked_before().unwrap_or_default();
                self.contacts_cache_dirty = true;

                let mut msg = format!(
                    "imported {} contact{}",
                    outcome.imported,
                    if outcome.imported == 1 { "" } else { "s" }
                );
                if plan.duplicates > 0 {
                    msg.push_str(&format!(" · {} duplicate(s) skipped", plan.duplicates));
                }
                if !plan.unusable.is_empty() {
                    msg.push_str(&format!(" · {} unusable", plan.unusable.len()));
                }
                if !outcome.journal_appended {
                    msg.push_str(" (journal not written)");
                }
                self.import_status = msg.clone();
                self.set_log_status(msg);
                self.import_file = None;
            }
            // Atomic: nothing was written, so the log is exactly as it was.
            Err(e) => self.import_status = format!("import failed, nothing written: {e}"),
        }
    }

    pub(crate) fn draw_import_window(
        &mut self,
        ctx: &egui::Context,
        snapshot: &crate::ui::state::UiState,
    ) {
        if !self.show_import {
            return;
        }
        let operator_id = snapshot.operator_id.clone();
        if operator_id.trim().is_empty() || self.log.is_none() {
            self.show_import = false;
            return;
        }

        let mut open = true;
        let mut pick = false;
        let mut do_import = false;

        egui::Window::new("Import ADIF")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("File");
                    let name = self
                        .import_file
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "— none chosen —".to_string());
                    ui.label(name);
                });
                ui.horizontal(|ui| {
                    let picker = file_picker_available();
                    if ui
                        .add_enabled(picker, egui::Button::new("Choose file…"))
                        .on_disabled_hover_text(NO_PICKER_HINT)
                        .clicked()
                    {
                        pick = true;
                    }
                });
                if !file_picker_available() {
                    // Import has no typed-path fallback the way export does — a
                    // file has to exist to be read — so say what to install.
                    ui.label(note_text(NO_PICKER_HINT));
                }

                ui.separator();

                if self.import_planning {
                    ui.label("reading the file…");
                } else if let Some(plan) = &self.import_plan {
                    // ── the preview: what this file would do to the log ──
                    ui.label(egui::RichText::new(plan.summary()).strong());

                    if plan.duplicates > 0 {
                        ui.label(note_text(
                            "Duplicates are contacts your log already has (same call, band \
                             and mode, within 30 minutes). They are skipped, so importing \
                             the same file twice is safe.",
                        ));
                    }

                    if !plan.unusable.is_empty() {
                        ui.separator();
                        ui.label(note_text(
                            "These records could not be imported. Everything else still will be:",
                        ));
                        egui::ScrollArea::vertical()
                            .max_height(120.0)
                            .id_salt("import_problems")
                            .show(ui, |ui| {
                                for p in &plan.unusable {
                                    ui.label(p.to_string());
                                }
                            });
                    }

                    ui.separator();
                    ui.horizontal(|ui| {
                        let can = !plan.is_empty();
                        if ui
                            .add_enabled(
                                can,
                                egui::Button::new(format!(
                                    "Import {} contact{}",
                                    plan.importable.len(),
                                    if plan.importable.len() == 1 { "" } else { "s" }
                                )),
                            )
                            .on_disabled_hover_text("Nothing in this file is new to your log.")
                            .clicked()
                        {
                            do_import = true;
                        }
                        if !can {
                            ui.label("nothing to import");
                        }
                    });
                } else if self.import_file.is_some() && self.import_status.is_empty() {
                    ui.label("reading the file…");
                } else if self.import_file.is_none() {
                    ui.label(note_text(
                        "Choose an ADIF file (.adi) to see what importing it would do. \
                         Nothing is written until you confirm.",
                    ));
                }

                if !self.import_status.is_empty() {
                    ui.separator();
                    ui.label(&self.import_status);
                }
            });

        // Native picker: synchronous, so it blocks the UI thread while open. The
        // media runtime and audio live on other threads and keep running.
        if pick {
            let mut dlg = rfd::FileDialog::new().add_filter("ADIF", &["adi", "adif"]);
            if let Some(dir) = self
                .persistence_store
                .qso_log_db_path(&operator_id)
                .parent()
                .filter(|p| p.is_dir())
            {
                dlg = dlg.set_directory(dir);
            }
            if let Some(path) = dlg.pick_file() {
                self.plan_import(&operator_id, path);
            }
        }

        if do_import {
            self.commit_import();
        }

        if !open {
            self.show_import = false;
        }
    }
}

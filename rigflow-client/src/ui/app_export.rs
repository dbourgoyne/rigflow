//! The ADIF export window, opened from the contact view (`V`).
//!
//! A floating `egui::Window` (non-blocking, like the log-entry window) with the
//! filters grouped into collapsible sections, the output options, and a **live
//! match count** so the operator sees "1,483 QSOs match" before committing to a
//! file. That count is a real dry run through the same filter the export will
//! use, debounced by [`COUNT_DEBOUNCE`].

use eframe::egui;

use crate::logging::export::{COUNT_DEBOUNCE, ExportDraft, ExportEvent, ExportJob, ProfileChoice};
use rigflow_log::export::GridPrecision;
use rigflow_log::normalize::ModeClass;

impl crate::ui::app::RigflowApp {
    /// Open the export window, seeding the draft with a dated default path in
    /// the current operator's data directory.
    pub(crate) fn open_export(&mut self, operator_id: &str) {
        let (date, _) = rigflow_log::now_utc_adif();
        let default_path = self
            .persistence_store
            .qso_log_db_path(operator_id)
            .with_file_name(format!("export-{date}.adi"));
        self.export_draft = ExportDraft::new(default_path);
        self.export_count = None;
        self.export_status.clear();
        self.export_count_due = Some(std::time::Instant::now());
        self.show_export = true;
    }

    /// Drain the export worker's replies. Called at the top of `update()`.
    ///
    /// This is where an incremental export's bookmark advances — on the UI
    /// thread, on the read-write `LogStore`, and only after the worker reports a
    /// **successful write** of an export that actually was incremental. The
    /// worker itself runs on a read-only connection and cannot do it.
    pub(crate) fn drain_export_events(&mut self, ctx: &egui::Context) {
        let mut got = false;
        while let Ok(evt) = self.export_rx.try_recv() {
            got = true;
            match evt {
                ExportEvent::Count(Ok(n)) => {
                    self.export_count = Some(n);
                    self.export_status.clear();
                }
                ExportEvent::Count(Err(e)) => {
                    self.export_count = None;
                    self.export_status = e;
                }
                ExportEvent::Done(Ok(summary)) => {
                    self.export_busy = false;

                    // Advance the incremental bookmark — and ONLY for an
                    // incremental export. An ad-hoc "export my 20m QSOs" must
                    // never move an operator's incremental position, or the next
                    // incremental run silently skips every unexported QSO that
                    // fell outside that filter.
                    //
                    // A failed advance is reported, not swallowed: the file is on
                    // disk but the position didn't move, so the next incremental
                    // run would re-export these contacts. The operator has to know.
                    let mut warning = String::new();
                    if let (Some(profile), Some(max_id)) = (
                        summary.filter.incremental_profile().map(str::to_string),
                        summary.max_qso_id,
                    ) && let Some(store) = self.log.as_mut()
                        && let Err(e) = store.advance_export_bookmark(&profile, max_id)
                    {
                        warning = format!(" — but the bookmark did not advance: {e}");
                    }

                    self.export_status = format!(
                        "exported {} contact{} to {}{}",
                        summary.count,
                        if summary.count == 1 { "" } else { "s" },
                        summary.path.display(),
                        warning,
                    );
                    self.set_log_status(self.export_status.clone());
                }
                ExportEvent::Done(Err(e)) => {
                    self.export_busy = false;
                    self.export_status = format!("export failed: {e}");
                }
            }
        }
        if got {
            ctx.request_repaint();
        }
    }

    /// Run the debounced dry-run count when the draft has settled.
    fn maybe_request_count(&mut self, operator_id: &str, ctx: &egui::Context) {
        // Any edit re-arms the timer.
        if self.export_draft_last.as_ref() != Some(&self.export_draft) {
            self.export_draft_last = Some(self.export_draft.clone());
            self.export_count_due = Some(std::time::Instant::now() + COUNT_DEBOUNCE);
            self.export_count = None;
        }
        let Some(due) = self.export_count_due else {
            return;
        };
        if std::time::Instant::now() < due {
            // Make sure we come back to fire it even if nothing else repaints.
            ctx.request_repaint_after(COUNT_DEBOUNCE);
            return;
        }
        self.export_count_due = None;

        match self.export_draft.to_filter() {
            Ok(filter) => {
                let db_path = self.persistence_store.qso_log_db_path(operator_id);
                let _ = self.export_tx.send(ExportJob::Count {
                    db_path,
                    filter: Box::new(filter),
                });
            }
            // A malformed filter is shown, not run.
            Err(e) => self.export_status = e.to_string(),
        }
    }

    pub(crate) fn draw_export_window(
        &mut self,
        ctx: &egui::Context,
        snapshot: &crate::ui::state::UiState,
    ) {
        if !self.show_export {
            return;
        }
        let operator_id = snapshot.operator_id.clone();
        if operator_id.trim().is_empty() || self.log.is_none() {
            self.show_export = false;
            return;
        }

        self.maybe_request_count(&operator_id, ctx);

        let mut open = true;
        let mut do_export = false;
        let mut pick_path = false;

        egui::Window::new("Export ADIF")
            .open(&mut open)
            .default_width(460.0)
            .show(ctx, |ui| {
                let d = &mut self.export_draft;

                egui::CollapsingHeader::new("Date")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("From");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.date_from)
                                    .hint_text("YYYYMMDD")
                                    .desired_width(90.0),
                            );
                            ui.label("To");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.date_to)
                                    .hint_text("YYYYMMDD")
                                    .desired_width(90.0),
                            );
                        });
                    });

                egui::CollapsingHeader::new("Band & mode")
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label("Bands (none = all)");
                        egui::Grid::new("export_bands")
                            .num_columns(6)
                            .show(ui, |ui| {
                                for (i, band) in ExportDraft::all_bands().into_iter().enumerate() {
                                    let mut on = d.bands.contains(band);
                                    if ui.checkbox(&mut on, band).changed() {
                                        if on {
                                            d.bands.insert(band.to_string());
                                        } else {
                                            d.bands.remove(band);
                                        }
                                    }
                                    if i % 6 == 5 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.add_enabled(
                            !d.bands.is_empty(),
                            egui::Checkbox::new(
                                &mut d.match_either_band,
                                "also match the RX band (split across bands)",
                            ),
                        );

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label("Mode class");
                            for c in ModeClass::ALL {
                                let mut on = d.mode_classes.contains(c);
                                if ui.checkbox(&mut on, c.as_str()).changed() {
                                    if on {
                                        d.mode_classes.insert(*c);
                                    } else {
                                        d.mode_classes.remove(c);
                                    }
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Modes");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.modes)
                                    .hint_text("SSB, CW, FT8")
                                    .desired_width(150.0),
                            );
                            ui.label("Submodes");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.submodes)
                                    .hint_text("FT4")
                                    .desired_width(100.0),
                            );
                        });
                    });

                egui::CollapsingHeader::new("Station").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Call");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.call_pattern)
                                .hint_text("W1* or K?ZD")
                                .desired_width(120.0),
                        );
                        ui.label("DXCC");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.dxcc)
                                .hint_text("291, 339")
                                .desired_width(90.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("Their grid");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.gridsquare)
                                .hint_text("EM12")
                                .desired_width(80.0),
                        );
                        egui::ComboBox::from_id_salt("grid_precision")
                            .selected_text(match d.grid_precision {
                                GridPrecision::Field => "field (2)",
                                GridPrecision::Square => "square (4)",
                                GridPrecision::Full => "exact",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut d.grid_precision,
                                    GridPrecision::Field,
                                    "field (2)",
                                );
                                ui.selectable_value(
                                    &mut d.grid_precision,
                                    GridPrecision::Square,
                                    "square (4)",
                                );
                                ui.selectable_value(
                                    &mut d.grid_precision,
                                    GridPrecision::Full,
                                    "exact",
                                );
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("My grid");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.my_gridsquare)
                                .hint_text("the grid you worked FROM")
                                .desired_width(160.0),
                        );
                    })
                    .response
                    .on_hover_text(
                        "Matches the grid recorded on each contact, so QSOs made \
                             from a previous QTH are still found after you move.",
                    );
                    ui.horizontal(|ui| {
                        ui.label("Contest");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.contest_id)
                                .hint_text("CQ-WW-SSB")
                                .desired_width(140.0),
                        );
                    });
                });

                egui::CollapsingHeader::new("Confirmation").show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Online-service state isn't tracked yet, so these match \
                                 nothing (or everything) until service sync lands.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Not uploaded to");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.not_uploaded_to)
                                .hint_text("lotw")
                                .desired_width(80.0),
                        );
                        ui.label("Confirmed by");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.confirmed_by)
                                .hint_text("lotw")
                                .desired_width(80.0),
                        );
                    });
                    ui.checkbox(&mut d.qsl_rcvd_yes, "QSL received (QSL_RCVD = Y)");
                });

                egui::CollapsingHeader::new("Incremental").show(ui, |ui| {
                    ui.checkbox(&mut d.incremental, "Only contacts new since the last run");
                    ui.add_enabled_ui(d.incremental, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Stream");
                            ui.add(
                                egui::TextEdit::singleline(&mut d.profile)
                                    .hint_text("default")
                                    .desired_width(120.0),
                            );
                        });
                    });
                    ui.label(
                        egui::RichText::new(
                            "Each named stream keeps its own position. Only an \
                                 incremental export moves it — an ordinary filtered \
                                 export never does.",
                        )
                        .small()
                        .weak(),
                    );
                });

                ui.separator();

                // ── output ──
                ui.horizontal(|ui| {
                    ui.label("File");
                    ui.add(egui::TextEdit::singleline(&mut d.output_path).desired_width(260.0));
                    if ui.button("Browse…").clicked() {
                        pick_path = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Fields");
                    ui.selectable_value(&mut d.profile_choice, ProfileChoice::Full, "Full");
                    ui.selectable_value(&mut d.profile_choice, ProfileChoice::Core, "Core");
                    ui.selectable_value(&mut d.profile_choice, ProfileChoice::Custom, "Custom");
                    if d.profile_choice == ProfileChoice::Full {
                        ui.checkbox(&mut d.include_extra, "include extra fields");
                    }
                });
                if d.profile_choice == ProfileChoice::Custom {
                    ui.horizontal(|ui| {
                        ui.label("Custom fields");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.custom_fields)
                                .hint_text("CALL, QSO_DATE, BAND, MODE")
                                .desired_width(280.0),
                        );
                    });
                }
                ui.checkbox(&mut d.sort_reverse, "newest first");

                ui.separator();

                // ── the live count + commit ──
                ui.horizontal(|ui| {
                    let ready =
                        !self.export_busy && !self.export_draft.output_path.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new("Export")).clicked() {
                        do_export = true;
                    }
                    match self.export_count {
                        Some(n) => ui.label(format!(
                            "{n} contact{} match",
                            if n == 1 { "" } else { "es" }
                        )),
                        None => ui.label(
                            egui::RichText::new(if self.export_busy {
                                "exporting…"
                            } else {
                                "counting…"
                            })
                            .weak(),
                        ),
                    };
                });
                if !self.export_status.is_empty() {
                    ui.label(&self.export_status);
                }
            });

        // The native picker is synchronous, so it blocks the UI thread while it's
        // open. That's the usual native-dialog trade (and correct on macOS, where
        // AppKit demands the main thread); audio and the media runtime live on
        // other threads and keep running.
        if pick_path {
            let start = std::path::PathBuf::from(self.export_draft.output_path.trim());
            let mut dlg = rfd::FileDialog::new().add_filter("ADIF", &["adi", "adif"]);
            if let Some(dir) = start.parent().filter(|p| p.is_dir()) {
                dlg = dlg.set_directory(dir);
            }
            if let Some(name) = start.file_name().and_then(|n| n.to_str()) {
                dlg = dlg.set_file_name(name);
            }
            if let Some(path) = dlg.save_file() {
                self.export_draft.output_path = path.to_string_lossy().into_owned();
            }
        }

        if do_export {
            match (
                self.export_draft.to_filter(),
                self.export_draft.to_options(),
            ) {
                (Ok(filter), Ok(options)) => {
                    let db_path = self.persistence_store.qso_log_db_path(&operator_id);
                    self.export_busy = true;
                    self.export_status.clear();
                    let _ = self.export_tx.send(ExportJob::Write {
                        db_path,
                        filter: Box::new(filter),
                        options: Box::new(options),
                    });
                }
                (Err(e), _) | (_, Err(e)) => self.export_status = e.to_string(),
            }
        }

        if !open {
            self.show_export = false;
        }
    }
}

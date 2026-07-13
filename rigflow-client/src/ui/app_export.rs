//! The contact-view's query driver, the worker drain, and the **export window**.
//!
//! The export window is *output only* — File, Fields, sort, incremental, and the
//! Export button. **Which** contacts get written is decided by the shared filter
//! in the `Filter…` window (`app_filter.rs`) — the same filter the contact view
//! lists — so the operator sees what they are exporting.
//!
//! The one thing that can make the written set differ from the visible list is
//! **incremental** ("only what's new since the last run"), which is a fact about
//! export progress, not about a contact. So the window spells out the arithmetic
//! — *"1,483 match your filter · 12 new since the last export · exporting 12"* —
//! rather than quietly writing a different set than the one on screen.

use eframe::egui;

use crate::logging::export::{ExportDraft, ExportEvent, ExportJob, ProfileChoice, QUERY_DEBOUNCE};
use crate::ui::panels::note_text;

impl crate::ui::app::RigflowApp {
    /// Open the export window, seeding a dated default path in the operator's
    /// data directory.
    pub(crate) fn open_export(&mut self, operator_id: &str) {
        let (date, _) = rigflow_log::now_utc_adif();
        let default_path = self
            .persistence_store
            .qso_log_db_path(operator_id)
            .with_file_name(format!("export-{date}.adi"));
        self.export_draft = ExportDraft::new(default_path);
        self.export_draft_last = None;
        self.export_count = None;
        self.export_status.clear();
        self.show_export = true;
    }

    /// Re-run the contact-view query when the filter (or the log) changes.
    ///
    /// Debounced: the filters that read the `extra` JSON blob are unindexed full
    /// scans, so re-querying on every keystroke would stutter a large log. The
    /// query runs on the worker's read-only connection, never on the UI thread
    /// that owns the `LogStore`.
    pub(crate) fn maybe_requery_contacts(&mut self, operator_id: &str, ctx: &egui::Context) {
        if self.log.is_none() {
            return;
        }

        // An edit to the shared filter re-arms the debounce and stales the
        // export's count (which is derived from the same filter).
        if self.qso_filter_last.as_ref() != Some(&self.qso_filter) {
            self.qso_filter_last = Some(self.qso_filter.clone());
            self.contacts_query_due = Some(std::time::Instant::now() + QUERY_DEBOUNCE);
            self.export_count = None;
        }
        // A newly logged contact (or Refresh) invalidates immediately — there is
        // nothing to settle, so no debounce.
        if self.contacts_cache_dirty {
            self.contacts_cache_dirty = false;
            self.contacts_query_due = Some(std::time::Instant::now());
        }

        let now = std::time::Instant::now();

        if let Some(due) = self.contacts_query_due {
            if now < due {
                ctx.request_repaint_after(QUERY_DEBOUNCE);
            } else {
                self.contacts_query_due = None;
                self.dispatch_contacts_query(operator_id);
            }
        }

        // The call lookup has its own debounce and its own (filter-independent)
        // query.
        if let Some(due) = self.call_lookup_due {
            if now < due {
                ctx.request_repaint_after(QUERY_DEBOUNCE);
            } else {
                self.call_lookup_due = None;
                self.dispatch_call_lookup(operator_id);
            }
        }
    }

    /// Send the contact-view page query — and, when an incremental export is
    /// being set up, its separate count.
    fn dispatch_contacts_query(&mut self, operator_id: &str) {
        let db_path = self.persistence_store.qso_log_db_path(operator_id);

        // The view always asks for the plain shared filter — never incremental.
        match self.qso_filter.to_filter(None) {
            Ok(filter) => {
                self.filter_error.clear();
                self.contacts_query_seq += 1;
                let _ = self.export_tx.send(ExportJob::Query {
                    db_path: db_path.clone(),
                    filter: Box::new(filter),
                    seq: self.contacts_query_seq,
                });
            }
            // A malformed filter is shown, not run — and the existing list is
            // left alone rather than silently emptied.
            Err(e) => {
                self.filter_error = e.to_string();
                return;
            }
        }

        // An incremental export writes a *subset* of the visible list, so it
        // needs its own count for the window to show the arithmetic honestly.
        if self.show_export
            && let Some(profile) = self.export_draft.incremental_profile()
            && let Ok(filter) = self.qso_filter.to_filter(Some(profile))
        {
            let _ = self.export_tx.send(ExportJob::Count {
                db_path,
                filter: Box::new(filter),
                seq: self.contacts_query_seq,
            });
        }
    }

    fn dispatch_call_lookup(&mut self, operator_id: &str) {
        let call = self.call_lookup.trim().to_string();
        if call.is_empty() {
            self.call_lookup_hits = None;
            return;
        }
        let Ok(filter) = crate::logging::export::call_lookup_filter(&call) else {
            return;
        };
        self.call_lookup_seq += 1;
        let _ = self.export_tx.send(ExportJob::CallLookup {
            db_path: self.persistence_store.qso_log_db_path(operator_id),
            call,
            filter: Box::new(filter),
            seq: self.call_lookup_seq,
        });
    }

    /// Drain the worker's replies.
    ///
    /// This is also where an incremental export's bookmark advances — on the UI
    /// thread, on the read-write `LogStore`, and only after the worker reports a
    /// successful write of an export that actually *was* incremental. The worker
    /// runs on a read-only connection and cannot do it itself.
    pub(crate) fn drain_export_events(&mut self, ctx: &egui::Context) {
        let mut got = false;
        while let Ok(evt) = self.export_rx.try_recv() {
            got = true;
            match evt {
                // Replies for a filter the operator has already typed past are
                // dropped — otherwise the list flashes stale rows.
                ExportEvent::Contacts { seq, result } => {
                    if seq != self.contacts_query_seq {
                        continue;
                    }
                    match result {
                        Ok(page) => {
                            self.contacts_cache = page.rows;
                            self.contacts_total = page.total;
                            self.filter_error.clear();
                        }
                        Err(e) => self.filter_error = e,
                    }
                }

                ExportEvent::Count { seq, result } => {
                    if seq != self.contacts_query_seq {
                        continue;
                    }
                    self.export_count = result.ok();
                }

                ExportEvent::CallMatches { seq, call, result } => {
                    // Two guards, because showing one station's contacts under
                    // another station's name would be worse than showing none:
                    // the sequence catches a reply the operator has typed past,
                    // and the echoed callsign catches any way the two could still
                    // disagree.
                    if seq != self.call_lookup_seq
                        || !call.eq_ignore_ascii_case(self.call_lookup.trim())
                    {
                        continue;
                    }
                    match result {
                        Ok(page) => self.call_lookup_hits = Some(*page),
                        Err(e) => self.set_log_status(format!("call lookup: {e}")),
                    }
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
                    // run would re-export these contacts. The operator must know.
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

                    // The bookmark just moved, so "what's still unexported" has
                    // changed: re-count.
                    self.export_count = None;
                    self.contacts_query_due = Some(std::time::Instant::now());
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

    /// The export window: **output options only**.
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

        // Toggling incremental changes what gets written — re-run the counts.
        if self.export_draft_last.as_ref() != Some(&self.export_draft) {
            self.export_draft_last = Some(self.export_draft.clone());
            self.export_count = None;
            self.contacts_query_due = Some(std::time::Instant::now());
        }

        let mut open = true;
        let mut do_export = false;
        let mut pick_path = false;
        let mut open_filter = false;

        egui::Window::new("Export ADIF")
            .open(&mut open)
            .default_width(470.0)
            .show(ctx, |ui| {
                // ── what will be written: an echo of the shared filter ──
                // Without this, opening Export from a filtered view and getting a
                // filtered file would be a surprise.
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Contacts:").strong());
                    if self.qso_filter.is_active() {
                        ui.label(self.qso_filter.summary());
                    } else {
                        ui.label("the whole log (no filter)");
                    }
                    if ui.small_button("Filter…").clicked() {
                        open_filter = true;
                    }
                });

                ui.separator();

                let d = &mut self.export_draft;

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

                // ── incremental ──
                ui.checkbox(&mut d.incremental, "Only contacts new since the last run");
                if d.incremental {
                    ui.horizontal(|ui| {
                        ui.label("Stream");
                        ui.add(
                            egui::TextEdit::singleline(&mut d.profile)
                                .hint_text("default")
                                .desired_width(120.0),
                        );
                    });
                    ui.label(note_text(
                        "Each named stream keeps its own position, and only an incremental \
                         export moves it — an ordinary filtered export never does.",
                    ));
                }

                ui.separator();

                // ── the arithmetic, then the button ──
                // With incremental on, the written set is a SUBSET of the visible
                // list. Say so in numbers, rather than letting the operator find
                // out by opening the file.
                let matching = self.contacts_total;
                if self.export_draft.incremental {
                    match self.export_count {
                        Some(n) => ui.label(format!(
                            "{matching} match your filter · {n} new since the last export \
                             · exporting {n}"
                        )),
                        None => ui.label("counting…"),
                    };
                } else {
                    ui.label(format!(
                        "exporting {matching} contact{}",
                        if matching == 1 { "" } else { "s" }
                    ));
                }

                ui.horizontal(|ui| {
                    let ready =
                        !self.export_busy && !self.export_draft.output_path.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new("Export")).clicked() {
                        do_export = true;
                    }
                    if self.export_busy {
                        ui.label("exporting…");
                    }
                });
                if !self.export_status.is_empty() {
                    ui.label(&self.export_status);
                }
                if !self.filter_error.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.filter_error);
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

        if open_filter {
            self.show_filter = true;
        }

        if do_export {
            let incremental = self.export_draft.incremental_profile();
            match (
                self.qso_filter.to_filter(incremental),
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

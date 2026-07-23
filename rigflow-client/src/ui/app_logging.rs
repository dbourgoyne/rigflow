//! The logging UI surfaces: the manual entry window (`L`) and the contact-view
//! window (`V`).
//!
//! Both are floating `egui::Window`s (non-blocking, so the operator can keep
//! tuning while logging). The entry window freezes the radio snapshot the
//! instant it opens — time, TX/RX frequency, and mode — so the logged values
//! reflect when contact was made, not when the operator finally saves.

use std::collections::BTreeMap;

use eframe::egui;

use crate::logging::LogEntryDraft;
use crate::logging::capture;
use crate::ui::app::RigflowApp;
use crate::ui::panels::note_text;
use crate::ui::state::UiState;
use rigflow_log::display;

/// The editable state of one contact open in the edit window. Holds the original
/// [`rigflow_log::Qso`] so `extra` (the MY_* snapshot, imported fields, QSL data)
/// and any column the form doesn't expose survive the edit untouched.
pub(crate) struct ContactEdit {
    pub id: i64,
    original: rigflow_log::Qso,
    call: String,
    qso_date: String,
    time_on: String,
    mode: String,
    freq_mhz: String,
    rst_sent: String,
    rst_rcvd: String,
    gridsquare: String,
    /// Last validation / save error, shown under the form. Empty = none.
    error: String,
}

impl ContactEdit {
    fn from_logged(row: &rigflow_log::store::LoggedQso) -> Self {
        let q = &row.qso;
        ContactEdit {
            id: row.id,
            original: q.clone(),
            call: q.call.clone(),
            qso_date: q.qso_date.clone(),
            time_on: q.time_on.clone(),
            mode: q.mode.clone(),
            freq_mhz: q
                .freq_hz
                .map(rigflow_log::adif::hz_to_mhz_string)
                .unwrap_or_default(),
            rst_sent: q.rst_sent.clone().unwrap_or_default(),
            rst_rcvd: q.rst_rcvd.clone().unwrap_or_default(),
            gridsquare: q.gridsquare.clone().unwrap_or_default(),
            error: String::new(),
        }
    }

    /// Build the edited QSO from the form, preserving the original's `extra` and
    /// unedited columns. `Err` is a message to show; don't save. Frequency drives
    /// band: a parseable MHz value updates the TX freq and lets `normalize`
    /// re-derive the band; a blank value keeps whatever was there.
    fn to_qso(&self) -> Result<rigflow_log::Qso, String> {
        let opt = |s: &str| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };
        let call = self.call.trim().to_ascii_uppercase();
        if call.is_empty() {
            return Err("Call is required.".into());
        }
        let date = self.qso_date.trim().to_string();
        if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Date must be 8 digits, YYYYMMDD.".into());
        }
        let time = self.time_on.trim().to_string();
        if !matches!(time.len(), 4 | 6) || !time.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Time must be HHMM or HHMMSS.".into());
        }
        if self.mode.trim().is_empty() {
            return Err("Mode is required.".into());
        }

        let mut q = self.original.clone();
        q.call = call;
        q.qso_date = date;
        q.time_on = time;
        q.mode = self.mode.trim().to_ascii_uppercase();
        q.rst_sent = opt(&self.rst_sent);
        q.rst_rcvd = opt(&self.rst_rcvd);
        q.gridsquare = opt(&self.gridsquare);

        let f = self.freq_mhz.trim();
        if !f.is_empty() {
            match rigflow_log::adif::mhz_string_to_hz(f) {
                Some(hz) => {
                    q.freq_hz = Some(hz);
                    q.band = String::new(); // re-derived from freq by normalize()
                }
                None => return Err("Frequency must be in MHz, e.g. 14.074.".into()),
            }
        }
        q.normalize();
        Ok(q)
    }
}

impl RigflowApp {
    /// Open the manual entry window, freezing the current radio state into the
    /// draft. Called from the `L` hotkey.
    pub(crate) fn open_log_entry(&mut self, snapshot: &UiState) {
        let captured = capture::capture_radio_state(snapshot);
        let (qso_date, time_on) = rigflow_log::now_utc_adif();
        let derived_rx = captured.derive_freq_rx();
        let mode = capture::effective_tx_mode(snapshot);

        let draft = LogEntryDraft {
            call: String::new(),
            rst_sent: capture::default_rst(mode).to_string(),
            rst_rcvd: capture::default_rst(mode).to_string(),
            name: String::new(),
            comment: String::new(),
            gridsquare: String::new(),
            mode: captured.tx_mode.clone(),
            freq_rx_hz_str: derived_rx.map(|h| h.to_string()).unwrap_or_default(),
            qso_date,
            time_on,
            tx_freq_hz: captured.tx_freq_hz,
            split_active: captured.split_active,
            derived_freq_rx_hz: derived_rx,
        };

        if let Ok(mut s) = self.state.lock() {
            s.log_entry_draft = draft;
            s.show_log_entry = true;
            s.log_entry_focus_pending = true;
            s.log_status.clear();
        }
    }

    /// Draw the manual entry window. Enter saves, Esc closes.
    pub(crate) fn draw_log_entry_window(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        if !snapshot.show_log_entry {
            return;
        }

        let mut save = false;
        let mut cancel = false;
        // Live-edited copy of the draft; written back after the window closure.
        let mut draft = snapshot.log_entry_draft.clone();
        let focus_pending = snapshot.log_entry_focus_pending;
        let mut focus_consumed = false;

        egui::Window::new("Log Contact")
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                // Frozen capture — read-only.
                let tx_mhz = draft.tx_freq_hz as f64 / 1_000_000.0;
                if draft.split_active {
                    let rx_txt = draft
                        .derived_freq_rx_hz
                        .map(|h| format!("{:.4}", h as f64 / 1_000_000.0))
                        .unwrap_or_else(|| "—".to_string());
                    ui.label(format!("{tx_mhz:.4} ↑ / {rx_txt} ↓ MHz  ({})", draft.mode));
                } else {
                    ui.label(format!("{tx_mhz:.4} MHz  ({})", draft.mode));
                }
                // Seconds shown here (unlike the list): this is the frozen instant
                // the contact will be logged at, so the operator is checking it.
                // Formatted via the helpers rather than sliced — `&time_on[..4]`
                // would panic on any value shorter than 4 chars.
                ui.label(format!(
                    "{} {}Z UTC",
                    display::date(&draft.qso_date),
                    display::time_hhmmss(&draft.time_on)
                ));
                ui.separator();

                egui::Grid::new("log_entry_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Call");
                        let call_resp = ui.text_edit_singleline(&mut draft.call);
                        if focus_pending && !focus_consumed {
                            call_resp.request_focus();
                            focus_consumed = true;
                        }
                        ui.end_row();

                        ui.label("RST sent");
                        ui.text_edit_singleline(&mut draft.rst_sent);
                        ui.end_row();
                        ui.label("RST rcvd");
                        ui.text_edit_singleline(&mut draft.rst_rcvd);
                        ui.end_row();
                        ui.label("Mode");
                        ui.text_edit_singleline(&mut draft.mode);
                        ui.end_row();
                        if draft.split_active {
                            ui.label("FREQ_RX (Hz)");
                            ui.text_edit_singleline(&mut draft.freq_rx_hz_str);
                            ui.end_row();
                        }
                        ui.label("Name");
                        ui.text_edit_singleline(&mut draft.name);
                        ui.end_row();
                        ui.label("Grid");
                        ui.text_edit_singleline(&mut draft.gridsquare);
                        ui.end_row();
                        ui.label("Comment");
                        ui.text_edit_singleline(&mut draft.comment);
                        ui.end_row();
                    });

                // Live "worked before?" hints from the in-memory index.
                self.show_worked_before_hints(ui, &draft);

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    let can_save = !draft.call.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Log (Enter)"))
                        .clicked()
                    {
                        save = true;
                    }
                });

                if !snapshot.log_status.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, &snapshot.log_status);
                }
            });

        // Keyboard: Enter saves (if a call is present), Esc closes.
        let (enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if enter && !draft.call.trim().is_empty() {
            save = true;
        }
        if esc {
            cancel = true;
        }

        // Persist the live edits + focus consumption back into UiState.
        if let Ok(mut s) = self.state.lock() {
            s.log_entry_draft = draft;
            if focus_consumed {
                s.log_entry_focus_pending = false;
            }
        }

        if cancel {
            if let Ok(mut s) = self.state.lock() {
                s.show_log_entry = false;
                s.log_entry_draft = LogEntryDraft::default();
            }
        } else if save {
            self.commit_log_entry();
        }
    }

    fn show_worked_before_hints(&self, ui: &mut egui::Ui, draft: &LogEntryDraft) {
        let call = draft.call.trim();
        if call.is_empty() {
            return;
        }
        let band = rigflow_log::normalize::band_for_freq_hz(draft.tx_freq_hz).unwrap_or("");
        if self.worked_before.is_new_call(call) {
            ui.colored_label(egui::Color32::LIGHT_GREEN, "New callsign");
        } else if self.worked_before.is_new_band(call, band) {
            ui.colored_label(
                egui::Color32::YELLOW,
                format!("Worked before — new on {band}"),
            );
        } else {
            ui.colored_label(egui::Color32::GRAY, "Worked before on this band");
        }
    }

    /// Build a `Qso` from the current draft and log it.
    fn commit_log_entry(&mut self) {
        let draft = {
            let s = self.state.lock().unwrap();
            s.log_entry_draft.clone()
        };
        if draft.call.trim().is_empty() {
            self.set_log_status("callsign required".to_string());
            return;
        }

        let mut extra = BTreeMap::new();
        for (k, v) in [
            ("NAME", draft.name.trim()),
            ("COMMENT", draft.comment.trim()),
        ] {
            if !v.is_empty() {
                extra.insert(k.to_string(), v.to_string());
            }
        }

        // FREQ_RX only for a real split QSO; the user may have cleared/edited it.
        let freq_rx_hz = if draft.split_active {
            draft.freq_rx_hz_str.trim().parse::<u64>().ok()
        } else {
            None
        };

        let opt = |s: &str| {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        };

        let qso = rigflow_log::Qso {
            call: draft.call.trim().to_ascii_uppercase(),
            qso_date: draft.qso_date.clone(),
            time_on: draft.time_on.clone(),
            band: String::new(), // derived from freq_hz by normalize()
            mode: draft.mode.clone(),
            submode: None,
            freq_hz: Some(draft.tx_freq_hz),
            freq_rx_hz,
            band_rx: None, // derived from freq_rx_hz by normalize()
            rst_sent: opt(&draft.rst_sent),
            rst_rcvd: opt(&draft.rst_rcvd),
            gridsquare: opt(&draft.gridsquare),
            dxcc: None,
            extra,
        };

        self.log_contact(qso);

        if let Ok(mut s) = self.state.lock() {
            s.show_log_entry = false;
            s.log_entry_draft = LogEntryDraft::default();
        }
    }

    /// Draw the contact-view window: the toolbar (call lookup, filter, export,
    /// refresh), the active-filter summary, and the table of matching contacts.
    ///
    /// The list is produced by the **same filter the export writes**, so the
    /// operator can see what they are about to export. The row count is capped
    /// (`VIEW_ROW_LIMIT`) but the *total* is always shown alongside — a capped
    /// list that didn't say so would quietly break that promise.
    pub(crate) fn draw_contact_view_window(&mut self, ctx: &egui::Context, snapshot: &UiState) {
        if !snapshot.show_contact_view {
            return;
        }
        let operator_id = snapshot.operator_id.clone();
        self.maybe_requery_contacts(&operator_id, ctx);

        let mut open = true;
        let mut open_export = false;
        let mut open_import = false;
        let mut open_filter = false;
        let mut clear_filter = false;
        // Row actions, applied after the window closure so the immutable borrow of
        // `contacts_cache` inside the grid doesn't collide with the &mut self they need.
        let mut open_edit: Option<ContactEdit> = None;
        let mut req_delete: Option<(i64, String)> = None;

        egui::Window::new("Contacts")
            .open(&mut open)
            .default_width(620.0)
            .default_height(440.0)
            .show(ctx, |ui| {
                if self.log.is_none() {
                    ui.label("Set an operator to view its contact log.");
                    return;
                }

                // ── toolbar ──
                ui.horizontal(|ui| {
                    ui.label("Call:");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.call_lookup)
                            .hint_text("worked before?")
                            .desired_width(110.0),
                    );
                    if r.changed() {
                        // Live as you type: mid-QSO the operator needs a yes/no
                        // now, not after finding and pressing a button.
                        self.call_lookup_due = Some(
                            std::time::Instant::now() + crate::logging::export::QUERY_DEBOUNCE,
                        );
                        if self.call_lookup.trim().is_empty() {
                            self.call_lookup_hits = None;
                        }
                    }
                    if ui.button("Filter…").clicked() {
                        open_filter = true;
                    }
                    if ui.button("Export…").clicked() {
                        open_export = true;
                    }
                    if ui.button("Import…").clicked() {
                        open_import = true;
                    }
                    if ui.button("Refresh").clicked() {
                        self.contacts_cache_dirty = true;
                    }
                });

                // ── active-filter summary ──
                // Always visible when filtering. A short list with no visible
                // reason is the single most confusing thing this window could do.
                if self.qso_filter.is_active() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("Filtered:").strong());
                        ui.label(self.qso_filter.summary());
                        if ui
                            .small_button("✕ clear")
                            .on_hover_text("Remove all filters")
                            .clicked()
                        {
                            clear_filter = true;
                        }
                    });
                }

                if !self.filter_error.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_RED, &self.filter_error);
                } else {
                    let shown = self.contacts_cache.len();
                    let total = self.contacts_total;
                    ui.label(if shown < total {
                        // The cap is load-bearing information, not a detail.
                        format!("showing {shown} of {total} matching")
                    } else {
                        format!("{total} contact{}", if total == 1 { "" } else { "s" })
                    });
                }

                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    egui::Grid::new("contacts_table")
                        .num_columns(7)
                        .striped(true)
                        .spacing([12.0, 2.0])
                        .show(ui, |ui| {
                            for h in ["Date", "Time (UTC)", "Call", "Band", "Mode", "Confirm", ""] {
                                ui.strong(h);
                            }
                            ui.end_row();
                            for row in &self.contacts_cache {
                                let q = &row.qso;
                                ui.label(display::date(&q.qso_date));
                                ui.label(display::time_hhmm(&q.time_on));
                                ui.label(&q.call);
                                ui.label(&q.band);
                                ui.label(&q.mode);
                                confirm_badge(ui, &row.confirmed);
                                ui.horizontal(|ui| {
                                    if ui.button("Edit").clicked() {
                                        open_edit = Some(ContactEdit::from_logged(row));
                                    }
                                    if ui.button("Delete").clicked() {
                                        req_delete = Some((
                                            row.id,
                                            format!("{} · {} · {}", q.call, q.band, q.mode),
                                        ));
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
            });

        self.draw_call_lookup_popup(ctx);

        if clear_filter {
            self.qso_filter = Default::default();
        }
        if open_filter {
            self.show_filter = true;
        }
        if open_export {
            self.open_export(&operator_id);
        }
        if open_import {
            self.open_import();
        }
        if let Some(edit) = open_edit {
            self.edit_contact = Some(edit);
        }
        if let Some(del) = req_delete {
            self.delete_contact = Some(del);
        }
        if !open && let Ok(mut s) = self.state.lock() {
            s.show_contact_view = false;
        }
    }

    /// The quick "have I worked this station?" popup.
    ///
    /// Leads with the **verdict**, not the data: mid-QSO the operator needs
    /// "new call" or "worked 3×" in under a second, and only then the rows. The
    /// headline comes from the in-memory worked-before index (no query at all);
    /// the rows come from the worker.
    ///
    /// It searches the **whole log** and never touches the view filter — "have I
    /// ever worked this station" is a question about the log, not about whatever
    /// the operator happens to be looking at.
    fn draw_call_lookup_popup(&mut self, ctx: &egui::Context) {
        let call = self.call_lookup.trim().to_ascii_uppercase();
        if call.is_empty() {
            return;
        }
        // Escape dismisses — a popup you can only close by aiming at a button is
        // an annoyance when you're in the middle of a contact.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.call_lookup.clear();
            self.call_lookup_hits = None;
            return;
        }

        let is_new = self.worked_before.is_new_call(&call);
        let mut dismiss = false;

        egui::Window::new(format!("Worked {call}?"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, [-16.0, 64.0])
            .show(ctx, |ui| {
                // Verdict first, from the in-memory index — instant, no DB hit.
                if is_new {
                    ui.label(
                        egui::RichText::new("NEW CALL")
                            .heading()
                            .color(egui::Color32::LIGHT_GREEN),
                    );
                } else {
                    match &self.call_lookup_hits {
                        Some(page) if page.total > 0 => {
                            let last = &page.rows[0].qso; // newest-first
                            ui.label(
                                egui::RichText::new(format!("Worked {}×", page.total))
                                    .heading()
                                    .color(egui::Color32::LIGHT_YELLOW),
                            );
                            ui.label(format!(
                                "last {} · {} {}",
                                display::date(&last.qso_date),
                                last.band,
                                last.mode
                            ));
                        }
                        // The index says worked, the rows haven't landed yet.
                        _ => {
                            ui.label(egui::RichText::new("Worked before").heading());
                            ui.label("looking up…");
                        }
                    }
                }

                if let Some(page) = &self.call_lookup_hits
                    && page.total > 0
                {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            egui::Grid::new("call_lookup_table")
                                .num_columns(4)
                                .striped(true)
                                .spacing([10.0, 2.0])
                                .show(ui, |ui| {
                                    for h in ["Date", "Time (UTC)", "Band", "Mode"] {
                                        ui.strong(h);
                                    }
                                    ui.end_row();
                                    for row in &page.rows {
                                        let q = &row.qso;
                                        ui.label(display::date(&q.qso_date));
                                        ui.label(display::time_hhmm(&q.time_on));
                                        ui.label(&q.band);
                                        ui.label(&q.mode);
                                        ui.end_row();
                                    }
                                });
                        });
                }

                ui.separator();
                if ui.button("Dismiss").clicked() {
                    dismiss = true;
                }
            });

        if dismiss {
            self.call_lookup.clear();
            self.call_lookup_hits = None;
        }
    }

    /// The edit-contact window, opened by "Edit" in the contact view. Edits the
    /// modeled columns; `extra` and untouched fields ride along via the original
    /// QSO held in [`ContactEdit`]. Save routes through `LogStore::update_qso`
    /// (DB-only; the append-only journal is untouched), then refreshes the view.
    pub(crate) fn draw_edit_contact_window(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.edit_contact.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;

        egui::Window::new("Edit Contact")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::Grid::new("edit_contact_grid")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        for (label, field) in [
                            ("Call", &mut edit.call),
                            ("Date (YYYYMMDD)", &mut edit.qso_date),
                            ("Time (HHMMSS)", &mut edit.time_on),
                            ("Mode", &mut edit.mode),
                            ("Freq (MHz)", &mut edit.freq_mhz),
                            ("RST sent", &mut edit.rst_sent),
                            ("RST rcvd", &mut edit.rst_rcvd),
                            ("Grid", &mut edit.gridsquare),
                        ] {
                            ui.label(label);
                            ui.text_edit_singleline(field);
                            ui.end_row();
                        }
                    });

                if !edit.error.is_empty() {
                    ui.colored_label(egui::Color32::from_rgb(0xff, 0x6b, 0x6b), &edit.error);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                });
            });

        // Closed via X or Cancel: drop the edit (already taken), window stays shut.
        if cancel || !open {
            return;
        }
        if save {
            match edit.to_qso() {
                Ok(q) => {
                    if let Some(store) = self.log.as_mut() {
                        match store.update_qso(edit.id, &q) {
                            Ok(()) => {
                                self.worked_before = store.load_worked_before().unwrap_or_default();
                                self.contacts_cache_dirty = true;
                                self.set_log_status(format!("updated {}", q.call));
                                return; // success → window closes
                            }
                            Err(e) => edit.error = format!("update failed: {e}"),
                        }
                    } else {
                        edit.error = "no operator selected".into();
                    }
                }
                Err(msg) => edit.error = msg,
            }
        }
        // Still editing (or a save error to show): keep the window open next frame.
        self.edit_contact = Some(edit);
    }

    /// The delete-confirmation dialog. Deleting is irreversible and drops the
    /// contact's confirmations too (ON DELETE CASCADE), so it always confirms
    /// first. Routes through `LogStore::delete_qso` (DB-only; the append-only
    /// journal keeps the original record as history).
    pub(crate) fn draw_delete_contact_confirm(&mut self, ctx: &egui::Context) {
        let Some((id, label)) = self.delete_contact.clone() else {
            return;
        };
        let mut open = true;
        let mut confirm = false;
        let mut cancel = false;

        egui::Window::new("Delete Contact")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Delete this contact?\n\n{label}"));
                ui.label(note_text(
                    "This removes it from the log, along with any confirmations, and cannot \
                     be undone. The append-only ADIF journal keeps the original record.",
                ));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("Delete").clicked() {
                        confirm = true;
                    }
                });
            });

        if confirm {
            if let Some(store) = self.log.as_mut() {
                match store.delete_qso(id) {
                    Ok(_) => {
                        self.worked_before = store.load_worked_before().unwrap_or_default();
                        self.contacts_cache_dirty = true;
                        self.set_log_status(format!("deleted {label}"));
                    }
                    Err(e) => self.set_log_status(format!("delete failed: {e}")),
                }
            }
            self.delete_contact = None;
        } else if cancel || !open {
            self.delete_contact = None;
        }
    }
}

/// The contact view's "Confirm" cell: the service names that have confirmed the
/// QSO (LoTW, eQSL, …) in green, or a plain "—" when none have. Emphasis is by
/// colour only — no leading glyph. A checkmark (U+2713) is not in egui's default
/// font and renders as a tofu box, so the green text alone carries "confirmed".
fn confirm_badge(ui: &mut egui::Ui, confirmed: &[String]) {
    if confirmed.is_empty() {
        ui.label("—");
        return;
    }
    let text = confirmed
        .iter()
        .map(|s| service_label(s))
        .collect::<Vec<_>>()
        .join(" · ");
    ui.colored_label(egui::Color32::from_rgb(0x4c, 0xaf, 0x50), text);
}

/// Display name for a `qso_service` code.
fn service_label(s: &str) -> &str {
    match s {
        "lotw" => "LoTW",
        "eqsl" => "eQSL",
        "qsl" => "QSL",
        other => other,
    }
}

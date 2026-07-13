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
use crate::ui::state::UiState;

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
                ui.label(format!("{} {}Z UTC", draft.qso_date, &draft.time_on[..4]));
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
        let mut open_filter = false;
        let mut clear_filter = false;

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
                        .num_columns(6)
                        .striped(true)
                        .spacing([12.0, 2.0])
                        .show(ui, |ui| {
                            for h in ["Date", "Time", "Call", "Band", "Mode", "Confirm"] {
                                ui.strong(h);
                            }
                            ui.end_row();
                            for row in &self.contacts_cache {
                                let q = &row.qso;
                                ui.label(&q.qso_date);
                                ui.label(q.time_on.get(..4).unwrap_or(&q.time_on));
                                ui.label(&q.call);
                                ui.label(&q.band);
                                ui.label(&q.mode);
                                // Per-service confirmation badge slots — empty in
                                // phase 1 (populated once service sync lands).
                                ui.label("—");
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
                                last.qso_date, last.band, last.mode
                            ));
                        }
                        // The index says worked, the rows haven't landed yet.
                        _ => {
                            ui.label(egui::RichText::new("Worked before").heading());
                            ui.label(egui::RichText::new("looking up…").weak());
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
                                    for h in ["Date", "Time", "Band", "Mode"] {
                                        ui.strong(h);
                                    }
                                    ui.end_row();
                                    for row in &page.rows {
                                        let q = &row.qso;
                                        ui.label(&q.qso_date);
                                        ui.label(q.time_on.get(..4).unwrap_or(&q.time_on));
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
}

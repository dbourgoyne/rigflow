//! The **Callbook** settings window — enable QRZ / HamQTH / Callook, enter their
//! credentials, and set the lookup priority order. This is where callbook
//! accounts are configured (never prompted for mid-QSO). Design:
//! `docs/release-review/callbook-lookup-design.md` §3–§4.
//!
//! Config (enable + order) persists to `callbook.json` in the operator dir;
//! credentials go to the keyring / `0600`-file store (keys `qrz-xml`, `hamqth`).

use eframe::egui;

use crate::logging::callbook::{self, Provider};
use crate::logging::credentials::{self, Credential};
use crate::ui::app::RigflowApp;
use crate::ui::panels::note_text;

impl RigflowApp {
    /// Open the callbook settings window, loading this operator's config + creds.
    pub(crate) fn open_callbook(&mut self, operator_id: &str) {
        self.show_callbook = true;
        self.callbook_status.clear();
        self.load_callbook(operator_id);
    }

    /// Load config + saved credentials into the fields, once per operator.
    pub(crate) fn load_callbook(&mut self, operator_id: &str) {
        if self.callbook_loaded_for.as_deref() == Some(operator_id) {
            return;
        }
        let dir = self.operator_dir(operator_id);
        self.callbook_config = callbook::load_config(&dir);

        let load = |p: Provider| -> (Credential, Option<credentials::Backend>) {
            match p
                .cred_key()
                .and_then(|k| credentials::load(k, operator_id, &dir))
            {
                Some((c, b)) => (c, Some(b)),
                None => (Credential::default(), None),
            }
        };
        (self.callbook_qrz, self.callbook_qrz_backend) = load(Provider::Qrz);
        (self.callbook_hamqth, self.callbook_hamqth_backend) = load(Provider::HamQTH);
        self.callbook_loaded_for = Some(operator_id.to_string());
    }

    /// Save config + any entered credentials.
    fn save_callbook(&mut self, operator_id: &str) {
        let dir = self.operator_dir(operator_id);
        let save_cred = |p: Provider, cred: &Credential| -> Option<credentials::Backend> {
            let key = p.cred_key()?;
            if cred.login.trim().is_empty() && cred.password.is_empty() {
                return None; // nothing entered — leave any existing store alone
            }
            credentials::store(key, operator_id, cred, &dir).ok()
        };
        if let Some(b) = save_cred(Provider::Qrz, &self.callbook_qrz) {
            self.callbook_qrz_backend = Some(b);
        }
        if let Some(b) = save_cred(Provider::HamQTH, &self.callbook_hamqth) {
            self.callbook_hamqth_backend = Some(b);
        }
        match callbook::save_config(&dir, &self.callbook_config) {
            Ok(()) => self.callbook_status = "saved".to_string(),
            Err(e) => self.callbook_status = format!("could not save config: {e}"),
        }
    }

    // ── lookup driving (for the log-entry `L` window) ────────────────────

    /// Per-frame callbook step for the `L` window: detect a call change (reset +
    /// instant prefix resolve + schedule online), fill the non-edited Name/Grid
    /// from the merged result, and fire the debounced online lookup when due.
    pub(crate) fn callbook_step(
        &mut self,
        draft: &mut crate::logging::LogEntryDraft,
        operator_id: &str,
        ctx: &egui::Context,
    ) {
        let call = draft.call.trim().to_ascii_uppercase();
        if call != self.cb_call {
            // New identity: drop stale callbook-filled visible fields (keep the
            // operator's own edits), reset, and resolve the offline floor now.
            if !self.cb_name_edited {
                draft.name.clear();
            }
            if !self.cb_grid_edited {
                draft.gridsquare.clear();
            }
            self.cb_name_edited = false;
            self.cb_grid_edited = false;
            self.cb_call = call.clone();
            self.cb_online = None;
            self.cb_busy = false;
            self.cb_prefix = callbook::prefix::resolve(&call);
            self.load_callbook(operator_id);
            let ready = call.len() >= 3 && !self.callbook_config.active().is_empty();
            self.cb_due =
                ready.then(|| std::time::Instant::now() + crate::logging::export::QUERY_DEBOUNCE);
        }

        // Fill non-edited visible fields from the effective (online-over-prefix)
        // result. Online arriving later overwrites the prefix value automatically.
        if let Some(eff) = callbook::merge(self.cb_prefix.clone(), self.cb_online.clone()) {
            if !self.cb_name_edited
                && let Some(n) = &eff.name
            {
                draft.name = n.clone();
            }
            if !self.cb_grid_edited
                && let Some(g) = &eff.grid
            {
                draft.gridsquare = g.clone();
            }
        }

        // Fire the debounced online lookup.
        if let Some(due) = self.cb_due {
            if std::time::Instant::now() < due {
                ctx.request_repaint_after(crate::logging::export::QUERY_DEBOUNCE);
            } else {
                self.cb_due = None;
                self.dispatch_callbook_lookup(operator_id);
            }
        }
    }

    fn dispatch_callbook_lookup(&mut self, operator_id: &str) {
        use crate::logging::callbook::CallbookCreds;
        self.load_callbook(operator_id);
        let order = self.callbook_config.active();
        if order.is_empty() {
            return;
        }
        let present = |c: &Credential| {
            (!c.login.trim().is_empty() || !c.password.is_empty()).then(|| c.clone())
        };
        let creds = CallbookCreds {
            qrz: self
                .callbook_config
                .is_enabled(Provider::Qrz)
                .then(|| present(&self.callbook_qrz))
                .flatten(),
            hamqth: self
                .callbook_config
                .is_enabled(Provider::HamQTH)
                .then(|| present(&self.callbook_hamqth))
                .flatten(),
        };
        self.cb_seq += 1;
        self.cb_busy = true;
        let _ = self
            .export_tx
            .send(crate::logging::export::ExportJob::CallbookLookup {
                order,
                creds: Box::new(creds),
                call: self.cb_call.clone(),
                seq: self.cb_seq,
            });
    }

    /// A short status line for the `L` window: "looking up…" or "via callbook ·
    /// United States (291)". `None` when there's nothing to show.
    pub(crate) fn callbook_note(&self) -> Option<String> {
        if self.cb_call.is_empty() {
            return None;
        }
        if self.cb_busy {
            return Some("looking up…".to_string());
        }
        let eff = callbook::merge(self.cb_prefix.clone(), self.cb_online.clone())?;
        // Name the actual source: "via QRZ" / "via HamQTH" / "via Callook" once an
        // online provider answers, else "via prefix" for the offline baseline.
        let mut s = format!("via {}", eff.source.label());
        if let Some(c) = &eff.country {
            let d = eff.dxcc.map(|d| format!(" · DXCC {d}")).unwrap_or_default();
            s.push_str(&format!(" · {c}{d}"));
        }
        Some(s)
    }

    /// Merge the callbook result into a QSO at save time: set `dxcc` (if unset)
    /// and add the `extra` ADIF fields (QTH/STATE/…) the visible form doesn't
    /// carry. Only applies when the result is for this QSO's call.
    pub(crate) fn callbook_apply_to_qso(&self, qso: &mut rigflow_log::Qso) {
        if self.cb_call != qso.call {
            return;
        }
        let Some(eff) = callbook::merge(self.cb_prefix.clone(), self.cb_online.clone()) else {
            return;
        };
        if qso.dxcc.is_none() {
            qso.dxcc = eff.dxcc;
        }
        for (k, v) in eff.extra_fields() {
            qso.extra.entry(k.to_string()).or_insert(v);
        }
    }

    pub(crate) fn draw_callbook_window(&mut self, ctx: &egui::Context, operator_id: &str) {
        if !self.show_callbook {
            return;
        }
        if operator_id.trim().is_empty() {
            self.show_callbook = false;
            return;
        }
        self.load_callbook(operator_id);

        let mut open = true;
        let mut save = false;
        let mut toggle: Option<(Provider, bool)> = None;
        let mut reorder: Option<(Provider, bool)> = None; // (provider, move_up?)
        let mut forget: Option<Provider> = None;

        egui::Window::new("Callbook")
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(note_text(
                    "Fill a contact's name / QTH / grid / DXCC from an online callbook as \
                     you log. Providers are tried in priority order; the first with a match \
                     wins. An offline prefix baseline always fills the DXCC entity and zones.",
                ));
                ui.separator();

                let order = self.callbook_config.ordered();
                let n = order.len();
                for (idx, p) in order.into_iter().enumerate() {
                    ui.horizontal(|ui| {
                        // Priority reorder. Plain-text labels — arrow glyphs
                        // (↑/↓) aren't in egui's default font and render as tofu.
                        if ui.add_enabled(idx > 0, egui::Button::new("Up")).clicked() {
                            reorder = Some((p, true));
                        }
                        if ui
                            .add_enabled(idx + 1 < n, egui::Button::new("Down"))
                            .clicked()
                        {
                            reorder = Some((p, false));
                        }
                        let mut en = self.callbook_config.is_enabled(p);
                        if ui.checkbox(&mut en, "").changed() {
                            toggle = Some((p, en));
                        }
                        ui.strong(p.name());
                        if p == Provider::Callook {
                            ui.label(note_text("US only · no account needed"));
                        }
                    });

                    // Credential fields for the auth'd providers.
                    let cred = match p {
                        Provider::Qrz => Some(&mut self.callbook_qrz),
                        Provider::HamQTH => Some(&mut self.callbook_hamqth),
                        Provider::Callook => None,
                    };
                    if let Some(cred) = cred {
                        if p == Provider::Qrz {
                            ui.label(note_text(
                                "Uses your qrz.com login (XML API) — NOT the QRZ Logbook API \
                                 key used for QSO sync. Requires an XML Logbook Data subscription.",
                            ));
                        }
                        egui::Grid::new(format!("cb_creds_{}", p.key()))
                            .num_columns(2)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Username");
                                ui.text_edit_singleline(&mut cred.login);
                                ui.end_row();
                                ui.label("Password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut cred.password).password(true),
                                );
                                ui.end_row();
                            });
                        let backend = match p {
                            Provider::Qrz => self.callbook_qrz_backend,
                            _ => self.callbook_hamqth_backend,
                        };
                        ui.horizontal(|ui| {
                            if let Some(b) = backend {
                                ui.label(note_text(b.note()));
                            }
                            if ui.button("Forget").clicked() {
                                forget = Some(p);
                            }
                        });
                    }
                    ui.separator();
                }

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if !self.callbook_status.is_empty() {
                        ui.label(&self.callbook_status);
                    }
                });
            });

        if let Some((p, on)) = toggle {
            self.callbook_config.set_enabled(p, on);
        }
        if let Some((p, up)) = reorder {
            if up {
                self.callbook_config.move_up(p);
            } else {
                self.callbook_config.move_down(p);
            }
        }
        if let Some(p) = forget {
            if let Some(key) = p.cred_key() {
                credentials::clear(key, operator_id, &self.operator_dir(operator_id));
            }
            match p {
                Provider::Qrz => {
                    self.callbook_qrz = Credential::default();
                    self.callbook_qrz_backend = None;
                }
                Provider::HamQTH => {
                    self.callbook_hamqth = Credential::default();
                    self.callbook_hamqth_backend = None;
                }
                Provider::Callook => {}
            }
            self.callbook_status = format!("{} credentials forgotten", p.name());
        }
        if save {
            self.save_callbook(operator_id);
        }
        if !open {
            self.show_callbook = false;
        }
    }
}

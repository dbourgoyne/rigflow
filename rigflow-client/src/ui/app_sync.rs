//! The **LoTW sync** window: download QSL confirmations from Logbook of the World
//! and apply them to the log.
//!
//! Download-only (upload needs TQSL/a certificate — out of scope). The heavy
//! lifting is elsewhere: the HTTPS GET + import plan run on the export worker
//! (off the UI thread); applying the plan is the same `commit_import` the manual
//! import uses. A sync **auto-commits** — confirmations only touch `qso_service`,
//! never insert contacts — so there's no per-run confirmation step, just a result.
//!
//! Credentials live in the OS keyring where available, else a per-operator
//! `0600` file (see [`crate::logging::credentials`]). The password field is only
//! re-saved when the operator edits it.

use eframe::egui;

use crate::logging::credentials::{self, Credential};
use crate::logging::export::ExportJob;
use crate::ui::app::RigflowApp;
use crate::ui::panels::note_text;

/// The ham service key for the credential store and `sync_state`.
const SERVICE: &str = "lotw";

impl RigflowApp {
    /// Open the sync window and load this operator's saved credentials once.
    pub(crate) fn open_sync(&mut self, operator_id: &str) {
        self.show_sync = true;
        self.sync_status.clear();
        self.load_sync_credentials(operator_id);
    }

    /// The per-operator directory that backs the credential file fallback.
    fn operator_dir(&self, operator_id: &str) -> std::path::PathBuf {
        self.persistence_store
            .qso_log_db_path(operator_id)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
    }

    /// Load saved LoTW credentials into the fields, once per operator.
    fn load_sync_credentials(&mut self, operator_id: &str) {
        if self.sync_loaded_for.as_deref() == Some(operator_id) {
            return;
        }
        let dir = self.operator_dir(operator_id);
        match credentials::load(SERVICE, operator_id, &dir) {
            Some((cred, backend)) => {
                self.sync_login = cred.login;
                self.sync_password = cred.password;
                self.sync_backend = Some(backend);
            }
            None => {
                self.sync_login.clear();
                self.sync_password.clear();
                self.sync_backend = None;
            }
        }
        self.sync_loaded_for = Some(operator_id.to_string());
    }

    /// Kick off a download on the worker: save the credentials, read the
    /// incremental marker, and send the job.
    fn start_lotw_sync(&mut self, operator_id: &str) {
        let dir = self.operator_dir(operator_id);
        let cred = Credential {
            login: self.sync_login.trim().to_string(),
            password: self.sync_password.clone(),
        };
        match credentials::store(SERVICE, operator_id, &cred, &dir) {
            Ok(backend) => self.sync_backend = Some(backend),
            Err(e) => {
                self.sync_status = format!("could not save credentials: {e}");
                return;
            }
        }

        let since = self
            .log
            .as_ref()
            .and_then(|s| s.sync_marker(SERVICE).ok().flatten());

        self.sync_busy = true;
        self.sync_status = "contacting LoTW…".to_string();
        let _ = self.export_tx.send(ExportJob::LotwSync {
            db_path: self.persistence_store.qso_log_db_path(operator_id),
            login: cred.login,
            password: cred.password,
            since,
        });
    }

    /// Apply a completed LoTW download: auto-commit the confirmations and advance
    /// the marker. Called from the export-event drain.
    pub(crate) fn apply_lotw_sync(
        &mut self,
        plan: rigflow_log::import::ImportPlan,
        marker: Option<String>,
    ) {
        self.sync_busy = false;

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
            self.sync_status = "no operator selected — nothing applied".to_string();
            return;
        };

        match store.commit_import(&plan.importable, &plan.confirmations, &station) {
            Ok(outcome) => {
                // Only advance the marker on a successful apply, so a failure
                // re-fetches the same window next time rather than skipping it.
                if let Err(e) = store.set_sync_marker(SERVICE, marker.as_deref()) {
                    eprintln!("rigflow: LoTW sync marker not saved: {e}");
                }
                self.worked_before = store.load_worked_before().unwrap_or_default();
                self.contacts_cache_dirty = true;

                let mut msg = format!(
                    "LoTW: {} new confirmation{}",
                    outcome.confirmed,
                    if outcome.confirmed == 1 { "" } else { "s" }
                );
                if plan.already_confirmed > 0 {
                    msg.push_str(&format!(" · {} already confirmed", plan.already_confirmed));
                }
                if plan.unmatched_confirmations > 0 {
                    msg.push_str(&format!(
                        " · {} for QSOs not in your log",
                        plan.unmatched_confirmations
                    ));
                }
                self.sync_status = msg.clone();
                self.set_log_status(msg);
            }
            Err(e) => self.sync_status = format!("LoTW sync: apply failed, nothing changed: {e}"),
        }
    }

    pub(crate) fn draw_sync_window(&mut self, ctx: &egui::Context, operator_id: &str) {
        if !self.show_sync {
            return;
        }
        if operator_id.trim().is_empty() || self.log.is_none() {
            self.show_sync = false;
            return;
        }

        // "Last synced" line, straight from the store.
        let last_run = self
            .log
            .as_ref()
            .and_then(|s| s.sync_last_run(SERVICE).ok().flatten());

        let mut open = true;
        let mut download = false;
        let mut forget = false;

        egui::Window::new("LoTW Sync")
            .open(&mut open)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(note_text(
                    "Download your QSL confirmations from Logbook of the World and apply \
                     them to matching contacts. (Uploading to LoTW is not done here.)",
                ));
                ui.separator();

                egui::Grid::new("lotw_creds")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("LoTW username");
                        ui.text_edit_singleline(&mut self.sync_login);
                        ui.end_row();
                        ui.label("Password");
                        ui.add(egui::TextEdit::singleline(&mut self.sync_password).password(true));
                        ui.end_row();
                    });

                if let Some(b) = self.sync_backend {
                    ui.label(note_text(b.note()));
                }

                ui.separator();
                if let Some(run) = &last_run {
                    // Stored RFC3339 (…"2026-07-24T09:00:00+00:00"); show the
                    // readable "YYYY-MM-DD HH:MM:SS" prefix, in UTC as stored.
                    let shown: String = run.replace('T', " ").chars().take(19).collect();
                    ui.label(format!("Last synced: {shown} UTC"));
                } else {
                    ui.label("Never synced.");
                }

                ui.horizontal(|ui| {
                    let ready = !self.sync_login.trim().is_empty()
                        && !self.sync_password.is_empty()
                        && !self.sync_busy;
                    if ui
                        .add_enabled(ready, egui::Button::new("Download confirmations"))
                        .clicked()
                    {
                        download = true;
                    }
                    if self.sync_busy {
                        ui.spinner();
                    }
                    if ui.button("Forget saved password").clicked() {
                        forget = true;
                    }
                });

                if !self.sync_status.is_empty() {
                    ui.separator();
                    ui.label(&self.sync_status);
                }
            });

        if download {
            self.start_lotw_sync(operator_id);
        }
        if forget {
            credentials::clear(SERVICE, operator_id, &self.operator_dir(operator_id));
            self.sync_password.clear();
            self.sync_backend = None;
            self.sync_status = "saved password forgotten".to_string();
        }
        if !open {
            self.show_sync = false;
        }
    }
}

//! The **online-sync** window: download QSL confirmations and upload QSOs to
//! LoTW, eQSL, and QRZ Logbook.
//!
//! The heavy lifting is elsewhere: the HTTP + read-only DB work run on the export
//! worker (off the UI thread); a download applies through the same `commit_import`
//! the manual import uses, an upload stamps `uploaded_at`. A **download
//! auto-commits** — confirmations only touch `qso_service`, never insert contacts.
//!
//! Per service: LoTW is download-only (upload needs TQSL); eQSL and QRZ do both.
//! Credentials are a username+password (LoTW, eQSL) or an API key (QRZ), stored in
//! the OS keyring where available, else a per-operator `0600` file. LoTW is the
//! only path exercised live so far — see [`crate::logging::services`].

use eframe::egui;

use crate::logging::credentials::{self, Credential};
use crate::logging::export::{ExportJob, SyncOutcome};
use crate::logging::services::{CredKind, Direction, Service};
use crate::ui::app::RigflowApp;
use crate::ui::panels::note_text;

impl RigflowApp {
    /// Open the sync window and load the current service's saved credentials.
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

    /// Load the current `(operator, service)`'s saved credentials into the
    /// fields, once — re-runs when the operator or the selected service changes.
    fn load_sync_credentials(&mut self, operator_id: &str) {
        let want = (operator_id.to_string(), self.sync_service);
        if self.sync_loaded_for.as_ref() == Some(&want) {
            return;
        }
        let dir = self.operator_dir(operator_id);
        match credentials::load(self.sync_service.key(), operator_id, &dir) {
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
        self.sync_loaded_for = Some(want);
    }

    /// Kick off a sync on the worker: save the credentials, read the marker (for a
    /// download), and send the job.
    fn start_sync(&mut self, operator_id: &str, direction: Direction) {
        let service = self.sync_service;
        let dir = self.operator_dir(operator_id);
        let cred = Credential {
            login: self.sync_login.trim().to_string(),
            password: self.sync_password.clone(),
        };
        match credentials::store(service.key(), operator_id, &cred, &dir) {
            Ok(backend) => self.sync_backend = Some(backend),
            Err(e) => {
                self.sync_status = format!("could not save credentials: {e}");
                return;
            }
        }

        let since = if direction == Direction::Download {
            self.log
                .as_ref()
                .and_then(|s| s.sync_marker(service.key()).ok().flatten())
        } else {
            None
        };

        // Force applies only to an upload; a download always uses its marker.
        let force = direction == Direction::Upload && self.sync_force_upload;

        self.sync_busy = true;
        self.sync_status = format!(
            "{} {}…",
            match direction {
                Direction::Download => "downloading from",
                Direction::Upload => "uploading to",
            },
            service.name()
        );
        let _ = self.export_tx.send(ExportJob::ServiceSync {
            service,
            direction,
            db_path: self.persistence_store.qso_log_db_path(operator_id),
            login: cred.login,
            password: cred.password,
            since,
            force,
        });
    }

    /// Apply a completed sync from the worker. Called from the export-event drain.
    /// The [`SyncOutcome`] itself distinguishes download from upload.
    pub(crate) fn apply_service_sync(
        &mut self,
        service: Service,
        result: Result<SyncOutcome, String>,
    ) {
        self.sync_busy = false;
        let outcome = match result {
            Ok(o) => o,
            Err(e) => {
                self.sync_status = format!("{} sync failed: {e}", service.name());
                return;
            }
        };
        match outcome {
            SyncOutcome::Downloaded { plan, marker } => self.apply_download(service, *plan, marker),
            SyncOutcome::Uploaded { ids, report } => self.apply_upload(service, ids, report),
        }
    }

    /// A download: auto-commit the confirmations and advance the marker.
    fn apply_download(
        &mut self,
        service: Service,
        plan: rigflow_log::import::ImportPlan,
        marker: Option<String>,
    ) {
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
            Ok(commit) => {
                // Advance the marker only on a successful apply, so a failure
                // re-fetches the same window next time rather than skipping it.
                if let Err(e) = store.set_sync_marker(service.key(), marker.as_deref()) {
                    eprintln!("rigflow: {} sync marker not saved: {e}", service.key());
                }
                self.worked_before = store.load_worked_before().unwrap_or_default();
                self.contacts_cache_dirty = true;

                let mut msg = format!(
                    "{}: {} new confirmation{}",
                    service.name(),
                    commit.confirmed,
                    if commit.confirmed == 1 { "" } else { "s" }
                );
                if commit.imported > 0 {
                    msg.push_str(&format!(" · {} new contact(s)", commit.imported));
                }
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
            Err(e) => self.sync_status = format!("{} sync: apply failed: {e}", service.name()),
        }
    }

    /// An upload: stamp the pushed QSOs uploaded and report the service's counts.
    fn apply_upload(
        &mut self,
        service: Service,
        ids: Vec<i64>,
        report: crate::logging::services::UploadReport,
    ) {
        if let Some(store) = self.log.as_mut()
            && !ids.is_empty()
            && let Err(e) = store.mark_uploaded(&ids, service.key())
        {
            self.sync_status = format!(
                "{} upload: sent, but marking uploaded failed: {e}",
                service.name()
            );
            return;
        }
        self.contacts_cache_dirty = true; // "not uploaded" filters just changed
        let msg = if ids.is_empty() {
            format!("{}: {}", service.name(), report.message)
        } else {
            let mut m = format!("{}: uploaded {} QSO(s)", service.name(), ids.len());
            if report.rejected > 0 {
                m.push_str(&format!(" · {} rejected", report.rejected));
            }
            m
        };
        self.sync_status = msg.clone();
        self.set_log_status(msg);
    }

    pub(crate) fn draw_sync_window(&mut self, ctx: &egui::Context, operator_id: &str) {
        if !self.show_sync {
            return;
        }
        if operator_id.trim().is_empty() || self.log.is_none() {
            self.show_sync = false;
            return;
        }
        // Load creds if the service selection changed since last frame.
        self.load_sync_credentials(operator_id);

        let service = self.sync_service;
        let last_run = self
            .log
            .as_ref()
            .and_then(|s| s.sync_last_run(service.key()).ok().flatten());

        let mut open = true;
        let mut download = false;
        let mut upload = false;
        let mut forget = false;
        let mut switch_to: Option<Service> = None;

        egui::Window::new("Online Sync")
            .open(&mut open)
            .default_width(380.0)
            .show(ctx, |ui| {
                // Service picker.
                ui.horizontal(|ui| {
                    ui.label("Service:");
                    for s in Service::ALL {
                        if ui.selectable_label(service == s, s.name()).clicked() && service != s {
                            switch_to = Some(s);
                        }
                    }
                });
                ui.separator();

                // Credentials, shaped by the auth kind.
                egui::Grid::new("sync_creds")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| match service.cred_kind() {
                        CredKind::UserPass => {
                            ui.label(format!("{} username", service.name()));
                            ui.text_edit_singleline(&mut self.sync_login);
                            ui.end_row();
                            ui.label("Password");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.sync_password).password(true),
                            );
                            ui.end_row();
                        }
                        CredKind::ApiKey => {
                            ui.label("API key");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.sync_password).password(true),
                            );
                            ui.end_row();
                        }
                    });

                if let Some(b) = self.sync_backend {
                    ui.label(note_text(b.note()));
                }

                ui.separator();
                if let Some(run) = &last_run {
                    let shown: String = run.replace('T', " ").chars().take(19).collect();
                    ui.label(format!("Last download: {shown} UTC"));
                } else if service.can_download() {
                    ui.label("Never downloaded.");
                }

                let ready = !self.sync_busy
                    && !self.sync_password.is_empty()
                    && (service.cred_kind() == CredKind::ApiKey
                        || !self.sync_login.trim().is_empty());

                ui.horizontal(|ui| {
                    if service.can_download()
                        && ui
                            .add_enabled(ready, egui::Button::new("Download confirmations"))
                            .clicked()
                    {
                        download = true;
                    }
                    if service.can_upload()
                        && ui
                            .add_enabled(ready, egui::Button::new("Upload new QSOs"))
                            .clicked()
                    {
                        upload = true;
                    }
                    if self.sync_busy {
                        ui.spinner();
                    }
                });
                if service.can_upload() {
                    ui.checkbox(
                        &mut self.sync_force_upload,
                        "Re-upload QSOs already marked uploaded",
                    )
                    .on_hover_text(
                        "Ignore the upload history and send everything; the service skips real \
                         duplicates. Use this to recover if the log was wrongly marked uploaded.",
                    );
                }
                if !service.can_upload() {
                    ui.label(note_text(
                        "Uploading to LoTW needs TQSL and your certificate, so it's done with \
                         the TQSL app, not here.",
                    ));
                }

                ui.horizontal(|ui| {
                    if ui.button("Forget saved credentials").clicked() {
                        forget = true;
                    }
                });

                if !self.sync_status.is_empty() {
                    ui.separator();
                    ui.label(&self.sync_status);
                }
            });

        if let Some(s) = switch_to {
            self.sync_service = s;
            self.sync_status.clear();
            // Force a reload for the new service next frame.
            self.sync_loaded_for = None;
        }
        if download {
            self.start_sync(operator_id, Direction::Download);
        }
        if upload {
            self.start_sync(operator_id, Direction::Upload);
        }
        if forget {
            credentials::clear(service.key(), operator_id, &self.operator_dir(operator_id));
            self.sync_login.clear();
            self.sync_password.clear();
            self.sync_backend = None;
            self.sync_status = "saved credentials forgotten".to_string();
        }
        if !open {
            self.show_sync = false;
        }
    }
}

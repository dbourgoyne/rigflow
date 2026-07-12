//! Per-operator `LogStore` lifecycle and the insert/query paths.
//!
//! The store is single-owner: `RigflowApp` holds it and is the only thread that
//! touches the `rusqlite::Connection`. On an operator switch we drop the old
//! store (closing the connection, checkpointing WAL) and open the new
//! operator's database, rebuilding the worked-before index.

use rigflow_log::store::WorkedBefore;

use crate::ui::app::RigflowApp;
use crate::ui::state::UiState;

impl RigflowApp {
    /// Open / reopen the contact-log store to match the current operator.
    /// A no-op while the operator is unchanged. Empty operator → no store
    /// (logging is inert until an operator is set), mirroring recording.
    pub(crate) fn sync_log_state(&mut self, snapshot: &UiState) {
        let op = snapshot.operator_id.clone();
        if self.log_open_for.as_deref() == Some(op.as_str()) {
            return;
        }

        // Operator changed (or first frame): tear down and rebuild.
        self.log = None;
        self.worked_before = WorkedBefore::default();
        self.contacts_cache.clear();
        self.contacts_cache_dirty = true;

        if !op.trim().is_empty() {
            if let Err(e) = self.persistence_store.ensure_operator_data_layout(&op) {
                self.set_log_status(format!("log dir: {e}"));
            } else {
                let db = self.persistence_store.qso_log_db_path(&op);
                let adi = self.persistence_store.qso_log_journal_path(&op);
                match rigflow_log::LogStore::open(&db, &adi) {
                    Ok(store) => {
                        self.worked_before = store.load_worked_before().unwrap_or_default();
                        self.log = Some(store);
                    }
                    Err(e) => self.set_log_status(format!("log open failed: {e}")),
                }
            }
        }
        self.log_open_for = Some(op);
    }

    /// Log one contact through the store: insert (DB commit → journal append),
    /// then update the worked-before index and flag the contact cache stale.
    /// The `MY_*` station snapshot is applied inside the store.
    pub(crate) fn log_contact(&mut self, qso: rigflow_log::Qso) {
        let mut qso = qso;
        qso.normalize();

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
            self.set_log_status("no operator selected — contact not logged".to_string());
            return;
        };
        match store.insert(&qso, &station) {
            Ok(outcome) => {
                self.worked_before.record(&qso);
                self.contacts_cache_dirty = true;
                let note = if outcome.journal_appended {
                    String::new()
                } else {
                    " (journal not written)".to_string()
                };
                self.set_log_status(format!("logged {}{}", qso.call, note));
            }
            Err(e) => self.set_log_status(format!("log failed: {e}")),
        }
    }

    /// Drain decoded WSJT-X events and ingest them into the active operator's
    /// log. Called at the top of `update()`.
    pub(crate) fn drain_wsjtx_events(&mut self, ctx: &eframe::egui::Context) {
        let mut adifs = Vec::new();
        while let Ok(ev) = self.wsjtx_rx.try_recv() {
            match ev {
                crate::logging::wsjtx_listener::LogEvent::LoggedAdif(adif) => adifs.push(adif),
            }
        }
        if adifs.is_empty() {
            return;
        }
        for adif in adifs {
            match rigflow_log::adif::parse_adif_to_qsos(&adif) {
                Ok(qsos) => {
                    for qso in qsos {
                        self.ingest_external_qso(qso);
                    }
                }
                Err(e) => self.set_log_status(format!("WSJT-X ADIF parse: {e}")),
            }
        }
        // The app free-runs repaints, but request one anyway so an idle window
        // reflects the new contact immediately.
        ctx.request_repaint();
    }

    /// Ingest an externally-sourced QSO (WSJT-X, file import). Unlike manual
    /// entry, this **skips** a near-duplicate rather than warning — there is no
    /// operator watching to dismiss a warning, and WSJT-X can resend a QSO.
    pub(crate) fn ingest_external_qso(&mut self, qso: rigflow_log::Qso) {
        let mut qso = qso;
        qso.normalize();
        if let Some(store) = self.log.as_ref() {
            match store.find_duplicates(&qso, rigflow_log::dedupe::DEFAULT_WINDOW_SECS) {
                Ok(dups) if !dups.is_empty() => {
                    self.set_log_status(format!("skipped duplicate {}", qso.call));
                    return;
                }
                Ok(_) => {}
                Err(e) => self.set_log_status(format!("dedupe check: {e}")),
            }
        }
        self.log_contact(qso);
    }

    /// Refresh the cached contact list for the contact-view window.
    pub(crate) fn refresh_contacts_cache(&mut self) {
        match self.log.as_ref() {
            Some(store) => match store.query_contacts(500) {
                Ok(rows) => self.contacts_cache = rows,
                Err(e) => self.set_log_status(format!("query failed: {e}")),
            },
            None => self.contacts_cache.clear(),
        }
        self.contacts_cache_dirty = false;
    }

    pub(crate) fn set_log_status(&self, msg: String) {
        if let Ok(mut s) = self.state.lock() {
            s.log_status = msg;
        }
    }
}

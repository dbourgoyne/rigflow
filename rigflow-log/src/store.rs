//! The SQLite-backed contact store — the source of truth.
//!
//! `LogStore` owns one `rusqlite::Connection` and the path to the append-only
//! ADIF journal. It is single-owner by design (see the crate docs): the
//! `Connection` is `!Sync`, and `synchronous=FULL` makes every commit fsync, so
//! exactly one thread (the UI thread in the client) drives it.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{Connection, Transaction};
use rusqlite::{OptionalExtension, params};

use crate::error::LogError;
use crate::model::{Qso, Station};
use crate::{adif, dedupe, migrations, schema};

pub struct LogStore {
    conn: Connection,
    journal_path: PathBuf,
}

/// Result of a successful insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertOutcome {
    /// The new `qso.id`.
    pub id: i64,
    /// Whether the post-commit ADIF journal append succeeded. `false` means the
    /// contact is safely in the DB but the journal is missing this record (the
    /// journal is only ever a subset of the DB — a warning, not a failure).
    pub journal_appended: bool,
}

/// Result of a committed import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCommit {
    /// Rows written. Duplicates and unusable records were dropped at plan time,
    /// so this is the length of what was handed in.
    pub imported: usize,
    /// Whether the batched ADIF journal append succeeded. `false` means the
    /// contacts are in the DB but missing from the journal (a warning: the
    /// journal is only ever a subset of the DB).
    pub journal_appended: bool,
}

/// A stored contact plus its row id, for the contact view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedQso {
    pub id: i64,
    pub qso: Qso,
}

/// In-memory "worked before?" index, loaded once at open and updated on each
/// insert. Kept small: sets of what's been worked, not the full log.
#[derive(Debug, Clone, Default)]
pub struct WorkedBefore {
    calls: HashSet<String>,
    call_bands: HashSet<(String, String)>,
    dxccs: HashSet<i64>,
}

impl WorkedBefore {
    /// Never worked this callsign before.
    pub fn is_new_call(&self, call: &str) -> bool {
        !self.calls.contains(&call.trim().to_ascii_uppercase())
    }
    /// Worked this call, but never on this band.
    pub fn is_new_band(&self, call: &str, band: &str) -> bool {
        !self
            .call_bands
            .contains(&(call.trim().to_ascii_uppercase(), band.trim().to_string()))
    }
    /// Never worked this DXCC entity.
    pub fn is_new_dxcc(&self, dxcc: i64) -> bool {
        !self.dxccs.contains(&dxcc)
    }
    /// Fold a just-logged QSO into the index.
    pub fn record(&mut self, q: &Qso) {
        let call = q.call.trim().to_ascii_uppercase();
        self.calls.insert(call.clone());
        self.call_bands.insert((call, q.band.trim().to_string()));
        if let Some(d) = q.dxcc {
            self.dxccs.insert(d);
        }
    }
}

impl LogStore {
    /// Open (creating if absent) the database at `db_path` and bind the ADIF
    /// journal at `journal_path`. Applies PRAGMAs and migrates to the current
    /// schema version. The journal file itself is created lazily on first
    /// insert.
    pub fn open(
        db_path: impl AsRef<Path>,
        journal_path: impl AsRef<Path>,
    ) -> Result<Self, LogError> {
        let conn = Connection::open(db_path.as_ref())?;
        conn.execute_batch(schema::PRAGMAS)?;
        migrations::migrate(&conn)?;
        Ok(LogStore {
            conn,
            journal_path: journal_path.as_ref().to_path_buf(),
        })
    }

    /// Open an in-memory database (tests, ephemeral use). The journal path is
    /// still required but nothing is written to it unless an insert runs.
    #[cfg(test)]
    pub fn open_in_memory(journal_path: impl AsRef<Path>) -> Result<Self, LogError> {
        let conn = Connection::open_in_memory()?;
        // WAL is meaningless in-memory; still set foreign_keys + synchronous.
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        migrations::migrate(&conn)?;
        Ok(LogStore {
            conn,
            journal_path: journal_path.as_ref().to_path_buf(),
        })
    }

    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The schema version currently stamped on the database.
    pub fn schema_version(&self) -> Result<i64, LogError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Log a contact. Ordering is load-bearing:
    /// `BEGIN → upsert station → INSERT qso → COMMIT → append one ADIF record`.
    /// If the journal append fails we warn and keep the commit, preserving the
    /// invariant that the journal is a subset of the DB (never the reverse).
    ///
    /// `station` provides `station_id` provenance and the `MY_*` snapshot; the
    /// snapshot fills only `extra` keys not already present, so an imported
    /// record's own `MY_*` (from WSJT-X or another log) is preserved.
    pub fn insert(&mut self, qso: &Qso, station: &Station) -> Result<InsertOutcome, LogError> {
        let q = with_station_snapshot(qso, station);

        let tx = self.conn.transaction()?;
        let station_id = upsert_station(&tx, station)?;
        let id = insert_qso_row(&tx, &q, station_id)?;
        tx.commit()?;

        // Journal append happens AFTER the DB commit. A failure here is a
        // warning, not an error: the DB is authoritative.
        let journal_appended = match self.append_journal(std::slice::from_ref(&q)) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("rigflow-log: ADIF journal append failed (DB commit kept): {e}");
                false
            }
        };
        Ok(InsertOutcome {
            id,
            journal_appended,
        })
    }

    /// Commit a planned ADIF import: **one transaction, one journal write, one
    /// fsync** for the whole batch.
    ///
    /// This is not a loop over [`LogStore::insert`], and it must not become one.
    /// `insert` is tuned for a single live contact — `synchronous=FULL` plus a
    /// journal `fsync` per record, because a QSO cannot be re-made and must
    /// survive a crash. Paying that per row over an imported log is quadratically
    /// miserable: 20k records take ~100s that way, and under a second like this.
    ///
    /// Atomic: if any row fails, the transaction rolls back and **nothing** is
    /// imported. A half-imported log is worse than none, because the operator
    /// cannot tell which half.
    ///
    /// `qsos` is expected to be [`crate::import::ImportPlan::importable`] — already
    /// normalized, validated, and deduped. Callers get the `MY_*` snapshot
    /// semantics of `insert`: the station fills only fields the imported record
    /// doesn't already carry, so another log's `MY_*` survives.
    pub fn commit_import(
        &mut self,
        qsos: &[Qso],
        station: &Station,
    ) -> Result<ImportCommit, LogError> {
        if qsos.is_empty() {
            return Ok(ImportCommit {
                imported: 0,
                journal_appended: true,
            });
        }
        let staged: Vec<Qso> = qsos
            .iter()
            .map(|q| with_station_snapshot(q, station))
            .collect();

        let tx = self.conn.transaction()?;
        let station_id = upsert_station(&tx, station)?;
        for q in &staged {
            insert_qso_row(&tx, q, station_id)?;
        }
        tx.commit()?;

        let journal_appended = match self.append_journal(&staged) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("rigflow-log: ADIF journal append failed (DB commit kept): {e}");
                false
            }
        };
        Ok(ImportCommit {
            imported: staged.len(),
            journal_appended,
        })
    }

    /// Most-recent contacts first (by UTC date/time, then id). `limit` caps the
    /// result. Structured so a later phase can add a filter clause without
    /// touching callers.
    pub fn query_contacts(&self, limit: usize) -> Result<Vec<LoggedQso>, LogError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,call,qso_date,time_on,band,mode,submode,freq_hz,freq_rx_hz,band_rx,\
             rst_sent,rst_rcvd,gridsquare,dxcc,extra \
             FROM qso ORDER BY qso_date DESC, time_on DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], row_to_logged_qso)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }

    /// Find existing contacts that duplicate `q` under the natural key
    /// (call+mode+band) within `window_secs`. Returned for the caller to
    /// **warn** on — this never rejects an insert. Pass
    /// [`dedupe::DEFAULT_WINDOW_SECS`] for the standard ±30-minute window.
    pub fn find_duplicates(&self, q: &Qso, window_secs: i64) -> Result<Vec<LoggedQso>, LogError> {
        let mut stmt = self.conn.prepare(
            "SELECT id,call,qso_date,time_on,band,mode,submode,freq_hz,freq_rx_hz,band_rx,\
             rst_sent,rst_rcvd,gridsquare,dxcc,extra \
             FROM qso WHERE call = ?1 COLLATE NOCASE AND mode = ?2 AND band = ?3",
        )?;
        let rows = stmt.query_map(params![q.call, q.mode, q.band], row_to_logged_qso)?;
        let mut out = Vec::new();
        for r in rows {
            let lq = r??;
            if dedupe::within_window(q, &lq.qso, window_secs) {
                out.push(lq);
            }
        }
        Ok(out)
    }

    /// Build the "worked before?" index from the whole log.
    pub fn load_worked_before(&self) -> Result<WorkedBefore, LogError> {
        let mut wb = WorkedBefore::default();
        let mut stmt = self.conn.prepare("SELECT call, band, dxcc FROM qso")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })?;
        for r in rows {
            let (call, band, dxcc) = r?;
            let call = call.trim().to_ascii_uppercase();
            wb.calls.insert(call.clone());
            wb.call_bands.insert((call, band.trim().to_string()));
            if let Some(d) = dxcc {
                wb.dxccs.insert(d);
            }
        }
        Ok(wb)
    }

    /// Count the QSOs an export filter would match, without writing a file.
    ///
    /// Convenience for a caller that already holds the store (and for tests).
    /// The client's dialog counts through [`crate::export::Exporter`] on a
    /// worker thread instead, so a full scan can't stutter the UI.
    pub fn count_matching(&self, filter: &crate::export::ExportFilter) -> Result<usize, LogError> {
        crate::export::writer::count_with(&self.conn, filter)
    }

    /// Contacts matching an export filter, newest first, capped at `limit`.
    ///
    /// Shares [`crate::export::query::build`] with the export writer, so the
    /// contact view lists exactly the QSOs an export of the same filter writes.
    pub fn query_contacts_filtered(
        &self,
        filter: &crate::export::ExportFilter,
        limit: usize,
    ) -> Result<Vec<LoggedQso>, LogError> {
        crate::export::writer::query_with(
            &self.conn,
            filter,
            limit,
            crate::export::Sort::Reverse, // the view is always newest-first
        )
    }

    /// The current position of a named incremental-export bookmark.
    pub fn export_bookmark(&self, profile: &str) -> Result<Option<i64>, LogError> {
        crate::export::writer::read_bookmark(&self.conn, profile)
    }

    /// Advance a named incremental-export bookmark to `last_qso_id`.
    ///
    /// **This is the only way the bookmark moves, and it is not reachable from
    /// the export writer** — that runs on a read-only connection. Call it only
    /// after an incremental (`since_last_export`), non-dry-run export has
    /// actually written its file. An ad-hoc filtered export must never land
    /// here: moving the bookmark on a one-off "export my 20m QSOs" would skip
    /// every unexported QSO outside that filter on the next incremental run.
    ///
    /// Monotonic by construction: a bookmark never moves backwards, so a
    /// re-export of an older slice can't rewind the operator's position.
    pub fn advance_export_bookmark(
        &mut self,
        profile: &str,
        last_qso_id: i64,
    ) -> Result<(), LogError> {
        let now = Utc::now().to_rfc3339();
        let created_at: Option<String> = self
            .conn
            .query_row(
                "SELECT created_at FROM qso WHERE id = ?1",
                [last_qso_id],
                |r| r.get(0),
            )
            .optional()?;
        self.conn.execute(
            "INSERT INTO export_state (profile, last_qso_id, last_created_at, last_run_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(profile) DO UPDATE SET \
               last_qso_id     = max(last_qso_id, excluded.last_qso_id), \
               last_created_at = excluded.last_created_at, \
               last_run_at     = excluded.last_run_at",
            params![profile, last_qso_id, created_at, now],
        )?;
        Ok(())
    }

    /// Append records to the journal, creating it (with header) on first write.
    /// `O_APPEND` + a single `fsync` for the whole batch; safe because this store
    /// is single-owner.
    ///
    /// Batched deliberately: a live contact passes one record (one fsync, which is
    /// the durability we want for a QSO that cannot be re-made), while an import
    /// passes thousands and pays that cost **once**.
    fn append_journal(&self, qsos: &[Qso]) -> Result<(), LogError> {
        if qsos.is_empty() {
            return Ok(());
        }
        let fresh = !self.journal_path.exists();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)?;

        let mut buf = String::new();
        if fresh {
            buf.push_str(&adif::adif_header());
        }
        for q in qsos {
            buf.push_str(&adif::write_record(&adif::qso_to_record(q)));
        }
        f.write_all(buf.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }
}

/// Apply the station's `MY_*` snapshot to a QSO, filling only fields the record
/// does not already carry — so an imported record's own `MY_*` (from WSJT-X, or
/// from another logging program) is preserved as the historical truth it is.
fn with_station_snapshot(qso: &Qso, station: &Station) -> Qso {
    let mut q = qso.clone();
    for (k, v) in station.my_adif_fields() {
        q.extra.entry(k).or_insert(v);
    }
    q
}

/// Insert one `qso` row inside an open transaction. Shared by the single-contact
/// and bulk-import paths so the column list can only ever drift in one place.
fn insert_qso_row(tx: &Transaction<'_>, q: &Qso, station_id: i64) -> Result<i64, LogError> {
    let now = Utc::now().to_rfc3339();
    let extra_json = serde_json::to_string(&q.extra)?;
    tx.execute(
        "INSERT INTO qso (call,qso_date,time_on,band,mode,submode,freq_hz,freq_rx_hz,\
         band_rx,rst_sent,rst_rcvd,gridsquare,dxcc,station_id,extra,created_at,updated_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        params![
            q.call,
            q.qso_date,
            q.time_on,
            q.band,
            q.mode,
            q.submode,
            q.freq_hz.map(|v| v as i64),
            q.freq_rx_hz.map(|v| v as i64),
            q.band_rx,
            q.rst_sent,
            q.rst_rcvd,
            q.gridsquare,
            q.dxcc,
            station_id,
            extra_json,
            now,
            now,
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Upsert the station row keyed by callsign, refreshing its location fields to
/// the current profile, and return its id (provenance for `qso.station_id`).
fn upsert_station(tx: &Transaction<'_>, s: &Station) -> Result<i64, LogError> {
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM station WHERE station_call = ?1",
            [&s.station_call],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        tx.execute(
            "UPDATE station SET gridsquare=?2, my_state=?3, my_county=?4, cq_zone=?5, \
             itu_zone=?6, name=?7 WHERE id=?1",
            params![
                id,
                s.gridsquare,
                s.my_state,
                s.my_county,
                s.cq_zone,
                s.itu_zone,
                s.name
            ],
        )?;
        Ok(id)
    } else {
        tx.execute(
            "INSERT INTO station (station_call,gridsquare,my_state,my_county,cq_zone,itu_zone,name) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                s.station_call,
                s.gridsquare,
                s.my_state,
                s.my_county,
                s.cq_zone,
                s.itu_zone,
                s.name
            ],
        )?;
        Ok(tx.last_insert_rowid())
    }
}

/// The `SELECT` list [`row_to_logged_qso`] expects, in order. Shared by every
/// query that hydrates a [`LoggedQso`] (contact view, dedupe, export) so a
/// column added here can't silently shift an index in one caller and not
/// another.
pub(crate) const QSO_COLUMNS: &str = "id,call,qso_date,time_on,band,mode,submode,freq_hz,\
     freq_rx_hz,band_rx,rst_sent,rst_rcvd,gridsquare,dxcc,extra";

pub(crate) fn row_to_logged_qso(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<Result<LoggedQso, LogError>> {
    let extra_json: String = r.get(14)?;
    let extra: BTreeMap<String, String> = match serde_json::from_str(&extra_json) {
        Ok(m) => m,
        Err(e) => return Ok(Err(LogError::Json(e))),
    };
    Ok(Ok(LoggedQso {
        id: r.get(0)?,
        qso: Qso {
            call: r.get(1)?,
            qso_date: r.get(2)?,
            time_on: r.get(3)?,
            band: r.get(4)?,
            mode: r.get(5)?,
            submode: r.get(6)?,
            freq_hz: r.get::<_, Option<i64>>(7)?.map(|v| v as u64),
            freq_rx_hz: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
            band_rx: r.get(9)?,
            rst_sent: r.get(10)?,
            rst_rcvd: r.get(11)?,
            gridsquare: r.get(12)?,
            dxcc: r.get(13)?,
            extra,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> Station {
        Station {
            station_call: "N0CALL".into(),
            gridsquare: Some("EM12".into()),
            name: Some("Dave".into()),
            ..Default::default()
        }
    }

    fn qso(call: &str, band: &str, time_on: &str) -> Qso {
        Qso {
            call: call.into(),
            qso_date: "20260711".into(),
            time_on: time_on.into(),
            band: band.into(),
            mode: "SSB".into(),
            freq_hz: Some(14_207_000),
            rst_sent: Some("59".into()),
            rst_rcvd: Some("59".into()),
            ..Default::default()
        }
    }

    #[test]
    fn insert_and_query_roundtrip() {
        let mut s = LogStore::open_in_memory("/nonexistent-dir-abc/ignored.adi").unwrap();
        // Journal path is a bogus dir so append fails; the DB commit must stand.
        let out = s.insert(&qso("W1AW", "20m", "142300"), &station()).unwrap();
        assert!(out.id > 0);
        assert!(!out.journal_appended, "append to bogus path should fail");

        let rows = s.query_contacts(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].qso.call, "W1AW");
        assert_eq!(rows[0].qso.band, "20m");
        // MY_* snapshot landed in extra.
        assert_eq!(
            rows[0].qso.extra.get("MY_GRIDSQUARE"),
            Some(&"EM12".to_string())
        );
        assert_eq!(
            rows[0].qso.extra.get("STATION_CALLSIGN"),
            Some(&"N0CALL".to_string())
        );
    }

    #[test]
    fn imported_my_fields_not_clobbered() {
        let mut s = LogStore::open_in_memory("/nonexistent/ignored.adi").unwrap();
        let mut q = qso("K5ZD", "20m", "142300");
        q.extra.insert("MY_GRIDSQUARE".into(), "FN31".into()); // imported value
        s.insert(&q, &station()).unwrap();
        let rows = s.query_contacts(1).unwrap();
        // Station grid is EM12, but the imported FN31 must be preserved.
        assert_eq!(
            rows[0].qso.extra.get("MY_GRIDSQUARE"),
            Some(&"FN31".to_string())
        );
    }

    #[test]
    fn query_orders_recent_first() {
        let mut s = LogStore::open_in_memory("/nonexistent/ignored.adi").unwrap();
        s.insert(&qso("W1AW", "20m", "142300"), &station()).unwrap();
        s.insert(&qso("K5ZD", "20m", "143000"), &station()).unwrap();
        let rows = s.query_contacts(10).unwrap();
        assert_eq!(rows[0].qso.call, "K5ZD"); // later time_on first
        assert_eq!(rows[1].qso.call, "W1AW");
    }

    #[test]
    fn worked_before_index() {
        let mut s = LogStore::open_in_memory("/nonexistent/ignored.adi").unwrap();
        let mut q = qso("W1AW", "20m", "142300");
        q.dxcc = Some(291);
        s.insert(&q, &station()).unwrap();

        let wb = s.load_worked_before().unwrap();
        assert!(!wb.is_new_call("w1aw")); // case-insensitive
        assert!(wb.is_new_call("K5ZD"));
        assert!(!wb.is_new_band("W1AW", "20m"));
        assert!(wb.is_new_band("W1AW", "40m"));
        assert!(!wb.is_new_dxcc(291));
        assert!(wb.is_new_dxcc(1));
    }

    #[test]
    fn find_duplicates_flags_near_repeat_only() {
        let mut s = LogStore::open_in_memory("/nonexistent/ignored.adi").unwrap();
        s.insert(&qso("W1AW", "20m", "142300"), &station()).unwrap();

        // 5 minutes later, same call/mode/band → flagged.
        let near = qso("w1aw", "20m", "142800");
        assert_eq!(
            s.find_duplicates(&near, crate::dedupe::DEFAULT_WINDOW_SECS)
                .unwrap()
                .len(),
            1
        );

        // Hours later (contest re-work) → not flagged.
        let later = qso("W1AW", "20m", "182300");
        assert!(
            s.find_duplicates(&later, crate::dedupe::DEFAULT_WINDOW_SECS)
                .unwrap()
                .is_empty()
        );

        // Different band → not flagged.
        let other_band = qso("W1AW", "40m", "142400");
        assert!(
            s.find_duplicates(&other_band, crate::dedupe::DEFAULT_WINDOW_SECS)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn append_failure_keeps_commit() {
        // A committed QSO with a failed journal append must still be a complete
        // log on the next read — the crash-safety / subset invariant.
        let mut s = LogStore::open_in_memory("/definitely/not/writable/x.adi").unwrap();
        let out = s.insert(&qso("W1AW", "20m", "142300"), &station()).unwrap();
        assert!(!out.journal_appended);
        assert_eq!(s.query_contacts(10).unwrap().len(), 1);
    }

    #[test]
    fn journal_written_with_header_then_records() {
        let dir = std::env::temp_dir().join(format!("rigflow-jrnl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("rigflow_log.db");
        let adi = dir.join("rigflow_log.adi");
        std::fs::remove_file(&adi).ok();

        {
            let mut s = LogStore::open(&db, &adi).unwrap();
            let out = s.insert(&qso("W1AW", "20m", "142300"), &station()).unwrap();
            assert!(out.journal_appended);
            s.insert(&qso("K5ZD", "20m", "143000"), &station()).unwrap();
        }
        let text = std::fs::read_to_string(&adi).unwrap();
        assert!(text.starts_with("<ADIF_VER:5>"), "header must be first");
        assert_eq!(text.matches("<EOH>").count(), 1, "exactly one header");
        assert_eq!(text.matches("<EOR>").count(), 2, "one EOR per QSO");
        assert!(text.contains("W1AW") && text.contains("K5ZD"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_migrates_to_current_version() {
        let store = LogStore::open_in_memory("/tmp/ignored.adi").unwrap();
        assert_eq!(store.schema_version().unwrap(), schema::SCHEMA_VERSION);
    }

    #[test]
    fn reopen_on_disk_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("rigflow-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("rigflow_log.db");
        let adi = dir.join("rigflow_log.adi");

        {
            let s = LogStore::open(&db, &adi).unwrap();
            assert_eq!(s.schema_version().unwrap(), schema::SCHEMA_VERSION);
        }
        // Reopen: migrate() must be a no-op and tables must still be there.
        {
            let s = LogStore::open(&db, &adi).unwrap();
            assert_eq!(s.schema_version().unwrap(), schema::SCHEMA_VERSION);
            let n: i64 = s
                .conn()
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='qso'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tables_and_indexes_exist() {
        let s = LogStore::open_in_memory("/tmp/ignored.adi").unwrap();
        for tbl in ["station", "qso", "qso_service", "sync_state"] {
            let n: i64 = s
                .conn()
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [tbl],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "table {tbl} missing");
        }
        let idx: i64 = s
            .conn()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_qso_match'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
    }
}

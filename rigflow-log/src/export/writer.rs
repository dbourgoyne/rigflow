//! The streaming ADIF export writer, the dry-run counter, and the
//! incremental-bookmark reader.
//!
//! **Export derives from SQLite. It never replays `rigflow_log.adi`.** The
//! journal is an append-only event tape: it still holds superseded (corrected)
//! records and records for since-deleted QSOs, ordered by log time rather than
//! contact time. The database is the materialized current state — corrections
//! applied, deletions gone, one row per QSO. The journal is replayed in exactly
//! one situation, and it is not this one: rebuilding a lost or corrupt `.db`.
//!
//! **Export is read-only, and [`Exporter`] makes that structural** rather than a
//! promise: it opens its own connection with `SQLITE_OPEN_READ_ONLY`, so SQLite
//! itself refuses any write. It therefore *cannot* touch `updated_at`, cannot
//! write `qso_service`, and — see [`crate::export`] — cannot advance the
//! incremental bookmark. It also means export can run on a worker thread while
//! the app's read-write `LogStore` keeps logging on the UI thread; WAL gives
//! concurrent readers for free.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{Connection, OpenFlags, params_from_iter};

use super::filter::{ExportFilter, ExportOptions};
use super::query;
use crate::error::LogError;
use crate::{adif, store};

/// This build's `PROGRAMVERSION`.
pub const PROGRAM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What an export did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    /// Records written (0 is a success, not an error — see [`export_with`]).
    pub count: usize,
    pub path: PathBuf,
    /// The filter that produced it, for the UI to show back and for a log line.
    pub filter: ExportFilter,
    /// ADIF `CREATED_TIMESTAMP` stamped in the header (`YYYYMMDD HHMMSS`, UTC).
    pub created_timestamp: String,
    /// Highest `qso.id` written. This is what an incremental bookmark advances
    /// to — but *this* type only reports it; advancing is the caller's job on a
    /// write connection. `None` when nothing matched.
    pub max_qso_id: Option<i64>,
}

/// A read-only view of a log database, for counting and exporting.
///
/// Owns its own `SQLITE_OPEN_READ_ONLY` connection, so it is safe to hand to a
/// worker thread even while the app's `LogStore` is inserting on another.
pub struct Exporter {
    conn: Connection,
}

impl Exporter {
    /// Open `db_path` read-only. Fails if the database doesn't exist — an export
    /// of a log that was never created is a caller bug, not an empty export.
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, LogError> {
        let conn = Connection::open_with_flags(
            db_path.as_ref(),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Exporter { conn })
    }

    /// Dry run: how many QSOs match, without writing anything. This is what the
    /// dialog's live "1,483 QSOs match" count calls.
    pub fn count(&self, filter: &ExportFilter) -> Result<usize, LogError> {
        count_with(&self.conn, filter)
    }

    /// Stream the matching QSOs to `opts.output_path`.
    pub fn export(
        &self,
        filter: &ExportFilter,
        opts: &ExportOptions,
    ) -> Result<ExportSummary, LogError> {
        export_with(&self.conn, filter, opts)
    }

    /// The current position of a named incremental bookmark, if it has one.
    pub fn bookmark(&self, profile: &str) -> Result<Option<i64>, LogError> {
        read_bookmark(&self.conn, profile)
    }

    /// Test-only handle, used to prove SQLite itself refuses a write here.
    #[cfg(test)]
    pub(crate) fn conn_for_test(&self) -> &Connection {
        &self.conn
    }
}

/// Read a named bookmark's `last_qso_id`. `None` = no such profile yet, which
/// means a first incremental export covers the whole log.
pub(crate) fn read_bookmark(conn: &Connection, profile: &str) -> Result<Option<i64>, LogError> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT last_qso_id FROM export_state WHERE profile = ?1",
            [profile],
            |r| r.get(0),
        )
        .optional()?)
}

/// Resolve the bookmark a filter needs (if it is an incremental export).
fn resolve_bookmark(conn: &Connection, filter: &ExportFilter) -> Result<Option<i64>, LogError> {
    match filter.incremental_profile() {
        Some(profile) => read_bookmark(conn, profile),
        None => Ok(None),
    }
}

/// Count matching QSOs. Same filter → same `WHERE` as [`export_with`], because
/// both go through [`query::build`]; a count that disagreed with the subsequent
/// export would be worse than no count at all.
pub(crate) fn count_with(conn: &Connection, filter: &ExportFilter) -> Result<usize, LogError> {
    filter.validate()?;
    let bookmark = resolve_bookmark(conn, filter)?;
    let q = query::build(filter, bookmark);
    let sql = format!("SELECT COUNT(*) FROM qso WHERE {}", q.where_sql);
    let n: i64 = conn.query_row(&sql, params_from_iter(q.params.iter()), |r| r.get(0))?;
    Ok(n as usize)
}

/// Write the matching QSOs to `opts.output_path` as ADIF.
///
/// Streams: one record is in memory at a time, so a 200k-QSO export costs a
/// buffer, not a heap of records.
///
/// An empty match set writes a **valid, importable ADIF file with a header and
/// zero records** and returns `count: 0`. That is not an error: "no QSOs matched
/// your filter" is a legitimate answer, and a caller that wants to treat it as
/// noteworthy can check `count`.
pub(crate) fn export_with(
    conn: &Connection,
    filter: &ExportFilter,
    opts: &ExportOptions,
) -> Result<ExportSummary, LogError> {
    filter.validate()?;
    opts.validate()?;

    let bookmark = resolve_bookmark(conn, filter)?;
    let q = query::build(filter, bookmark);
    let sql = format!(
        "SELECT {} FROM qso WHERE {} {}",
        store::QSO_COLUMNS,
        q.where_sql,
        query::order_by(opts.sort),
    );

    let created_timestamp = Utc::now().format("%Y%m%d %H%M%S").to_string();

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params_from_iter(q.params.iter()))?;

    let mut w = BufWriter::new(File::create(&opts.output_path)?);
    w.write_all(
        adif::export_header(&opts.adif_version, PROGRAM_VERSION, &created_timestamp).as_bytes(),
    )?;

    let mut count = 0usize;
    let mut max_qso_id: Option<i64> = None;
    while let Some(row) = rows.next()? {
        let lq = store::row_to_logged_qso(row)??;

        // The shared record-writer — the same one the journal appends through.
        // Split fields (FREQ_RX/BAND_RX only when present) and modeled-column-
        // wins are applied in there, once, for both callers.
        let mut record = adif::qso_to_record(&lq.qso);
        opts.project(&mut record);
        w.write_all(adif::write_record(&record).as_bytes())?;

        count += 1;
        max_qso_id = Some(max_qso_id.map_or(lq.id, |m: i64| m.max(lq.id)));
    }
    w.flush()?;

    Ok(ExportSummary {
        count,
        path: opts.output_path.clone(),
        filter: filter.clone(),
        created_timestamp,
        max_qso_id,
    })
}

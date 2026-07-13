//! ADIF file import: plan first, then commit.
//!
//! Import is two phases, and the split is the point:
//!
//! 1. [`plan`] — parse, normalize, validate, and dedupe against the existing log.
//!    **Read-only**: it touches nothing, so it can run on the export worker's
//!    read-only connection while the operator keeps logging. It answers "what
//!    would this file do to my log?" — *1,483 records · 12 duplicates · 3
//!    unusable* — before a single row is written.
//! 2. [`crate::LogStore::commit_import`] — one transaction, one journal write, one
//!    fsync, all-or-nothing.
//!
//! **Duplicates are skipped**, on the same ±30-minute call+band+mode rule the
//! WSJT-X ingest path uses ([`crate::dedupe`]). That makes import *idempotent*:
//! importing the same file twice adds nothing the second time, which is the
//! property that lets an operator re-run an import without fear.
//!
//! **Bad records are skipped, not fatal.** A real twenty-year log exported from
//! another program will contain some cruft; refusing a 20,000-QSO file over three
//! junk rows would be hostile. They are counted and named in the plan so the
//! operator can see exactly what was left behind.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::error::LogError;
use crate::model::Qso;
use crate::store::LoggedQso;
use crate::{adif, dedupe};

/// Why one record could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProblem {
    /// 1-based position in the file, so the operator can find it.
    pub record: usize,
    /// The record's callsign, if it had one — the only thing that makes a bad
    /// record identifiable to a human.
    pub call: String,
    pub reason: String,
}

impl std::fmt::Display for ImportProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let call = if self.call.is_empty() {
            "(no call)"
        } else {
            &self.call
        };
        write!(f, "record {} [{}]: {}", self.record, call, self.reason)
    }
}

/// What an import *would* do. Produced without writing anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportPlan {
    /// Records found in the file.
    pub total: usize,
    /// Good, non-duplicate contacts, ready to commit.
    pub importable: Vec<Qso>,
    /// Records skipped because the log (or an earlier record in this same file)
    /// already has them.
    pub duplicates: usize,
    /// Records that could not be made into a contact.
    pub unusable: Vec<ImportProblem>,
}

impl ImportPlan {
    /// One-line summary for the dialog.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} record{} · {} to import",
            self.total,
            if self.total == 1 { "" } else { "s" },
            self.importable.len()
        );
        if self.duplicates > 0 {
            s.push_str(&format!(" · {} duplicate", self.duplicates));
            if self.duplicates != 1 {
                s.push('s');
            }
            s.push_str(" (skipped)");
        }
        if !self.unusable.is_empty() {
            s.push_str(&format!(" · {} unusable", self.unusable.len()));
        }
        s
    }

    /// Nothing to do — no good records at all.
    pub fn is_empty(&self) -> bool {
        self.importable.is_empty()
    }
}

/// Validate a parsed record. `Err(reason)` = unusable.
///
/// The bar is deliberately low: only what the schema and any future matching
/// genuinely require. A contact with no callsign identifies no one; one with no
/// date/time cannot be matched against a QSL or a confirmation. Everything else
/// (RST, grid, name…) is optional in ADIF and optional here.
fn check(q: &Qso) -> Result<(), String> {
    if q.call.trim().is_empty() {
        return Err("no CALL".into());
    }
    let d = q.qso_date.trim();
    if d.len() != 8 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("bad QSO_DATE {:?} (want YYYYMMDD)", q.qso_date));
    }
    let t = q.time_on.trim();
    if !matches!(t.len(), 4 | 6) || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("bad TIME_ON {:?} (want HHMM or HHMMSS)", q.time_on));
    }
    if q.mode.trim().is_empty() {
        return Err("no MODE".into());
    }
    Ok(())
}

/// Existing contacts that duplicate `q`, read straight from the database.
///
/// Same natural key and window as the live dedupe path — one rule for WSJT-X
/// ingest, manual entry, and import, so "already logged" means the same thing
/// however a contact arrived.
fn db_duplicates(conn: &Connection, q: &Qso, window_secs: i64) -> Result<Vec<LoggedQso>, LogError> {
    let mut stmt = conn.prepare(
        "SELECT id,call,qso_date,time_on,band,mode,submode,freq_hz,freq_rx_hz,band_rx,\
         rst_sent,rst_rcvd,gridsquare,dxcc,extra \
         FROM qso WHERE call = ?1 COLLATE NOCASE AND mode = ?2 AND band = ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![q.call, q.mode, q.band],
        crate::store::row_to_logged_qso,
    )?;
    let mut out = Vec::new();
    for r in rows {
        let lq = r??;
        if dedupe::within_window(q, &lq.qso, window_secs) {
            out.push(lq);
        }
    }
    Ok(out)
}

/// Plan an import from ADIF `text`, against the log behind `conn`.
///
/// Read-only. Safe on a `SQLITE_OPEN_READ_ONLY` connection, which is how the
/// client runs it (on the worker, off the UI thread).
///
/// A parse failure is fatal and returns `Err` — a file we cannot even tokenize is
/// not an ADIF file, and guessing at it would be worse than refusing. Per-record
/// problems are *not* fatal; they land in [`ImportPlan::unusable`].
pub fn plan(conn: &Connection, text: &str, window_secs: i64) -> Result<ImportPlan, LogError> {
    let records = adif::parse_adif(text)?;

    let mut plan = ImportPlan {
        total: records.len(),
        ..Default::default()
    };

    // Within-file duplicates are checked against a natural-key index, NOT by
    // scanning everything imported so far: that scan is O(n²), and on a 20k-record
    // log it costs ~20 seconds of pure string comparison. Bucketing by
    // (call, band, mode) — the same natural key the DB check uses — leaves only
    // the handful of contacts with that same station on that same band and mode
    // to compare timestamps against.
    let mut seen: HashMap<(String, String, String), Vec<usize>> = HashMap::new();

    for (i, rec) in records.iter().enumerate() {
        let mut q = adif::record_to_qso(rec);
        q.normalize();

        if let Err(reason) = check(&q) {
            plan.unusable.push(ImportProblem {
                record: i + 1,
                call: q.call.trim().to_string(),
                reason,
            });
            continue;
        }

        // Already in the log?
        if !db_duplicates(conn, &q, window_secs)?.is_empty() {
            plan.duplicates += 1;
            continue;
        }

        // …or already earlier in THIS file? A log exported from another program
        // can carry its own internal duplicates, and the DB check cannot see the
        // records we are about to add in the same batch.
        let key = natural_key(&q);
        let dup = seen.get(&key).is_some_and(|idxs| {
            idxs.iter()
                .any(|&j| dedupe::within_window(&q, &plan.importable[j], window_secs))
        });
        if dup {
            plan.duplicates += 1;
            continue;
        }

        seen.entry(key).or_default().push(plan.importable.len());
        plan.importable.push(q);
    }

    Ok(plan)
}

/// The dedupe natural key: call + band + mode, normalized the way the SQL check
/// compares them (`call` case-insensitively).
fn natural_key(q: &Qso) -> (String, String, String) {
    (
        q.call.trim().to_ascii_uppercase(),
        q.band.trim().to_string(),
        q.mode.trim().to_string(),
    )
}

#[cfg(test)]
mod tests;

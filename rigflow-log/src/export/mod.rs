//! ADIF export: filters, the streaming writer, and incremental bookmarks.
//!
//! Four invariants hold across this module. They are deliberate, and each one
//! encodes a failure mode:
//!
//! 1. **Export is generated from SQLite, never by replaying the ADIF journal.**
//!    The journal is an append-only tape (superseded records, deleted QSOs,
//!    log-time order); the DB is the materialized current state. See
//!    [`writer`].
//!
//! 2. **Export reuses the shared ADIF record-writer**
//!    ([`crate::adif::qso_to_record`] + [`crate::adif::write_record`]) rather
//!    than serializing a second time. One serializer, two callers (journal +
//!    export) — which is what guarantees the split-frequency rule
//!    (`FREQ_RX`/`BAND_RX` iff `freq_rx_hz` is set) is applied identically in
//!    both. Field profiles are a *projection* over that writer's output
//!    ([`filter::ExportOptions::project`]), not a rewrite of it.
//!
//! 3. **Export is strictly read-only**, enforced by SQLite rather than by
//!    convention: [`Exporter`] holds a `SQLITE_OPEN_READ_ONLY` connection.
//!
//! 4. **Only an incremental export advances an incremental bookmark**, and only
//!    on a successful non-dry-run write. This is structural too: the exporter's
//!    connection *cannot* write, so it reports [`ExportSummary::max_qso_id`] and
//!    the caller advances the bookmark on the read-write [`crate::LogStore`]
//!    ([`crate::LogStore::advance_export_bookmark`]). An ad-hoc "export my 20m
//!    QSOs" therefore has no code path that could corrupt an operator's
//!    incremental position.
//!
//! Filters over fields that aren't modeled as columns (continent, zones,
//! contest id, QSL status, and the `MY_*` station snapshot) read the `extra`
//! JSON. In particular `my_gridsquare` reads the **per-QSO snapshot**, not the
//! `station` table: that row is keyed on callsign and updated in place, so it
//! holds today's location, and joining it would mis-attribute every QSO made
//! before an operator moved.

pub mod filter;
pub mod query;
pub mod writer;

pub use filter::{
    CORE_FIELDS, DEFAULT_ADIF_VERSION, DEFAULT_EXPORT_PROFILE, ExportFilter, ExportOptions,
    FieldProfile, FilterError, GridPrecision, QslStatusFilter, Sort, Timestamp,
};
pub use writer::{ContactPage, ExportSummary, Exporter, PROGRAM_VERSION};

#[cfg(test)]
mod tests;

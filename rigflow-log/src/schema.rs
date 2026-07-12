//! SQLite schema (v1) and PRAGMA setup.
//!
//! The schema is deliberately forward-looking: `qso_service` and `sync_state`
//! are created now but stay empty until a later phase adds online-service
//! confirmation sync. The `extra` JSON column round-trips arbitrary ADIF
//! fields, and `idx_qso_match` is the join key future confirmation matching
//! will use, so it is created correctly now.

/// Current schema version stamped into `PRAGMA user_version`.
pub const SCHEMA_VERSION: i64 = 1;

/// DDL that brings a fresh database to v1.
pub const V1_DDL: &str = r#"
CREATE TABLE station (
    id            INTEGER PRIMARY KEY,
    station_call  TEXT NOT NULL,
    gridsquare    TEXT,
    my_state      TEXT,
    my_county     TEXT,
    cq_zone       TEXT,
    itu_zone      TEXT,
    name          TEXT
);

CREATE TABLE qso (
    id            INTEGER PRIMARY KEY,
    call          TEXT NOT NULL,
    qso_date      TEXT NOT NULL,
    time_on       TEXT NOT NULL,
    band          TEXT NOT NULL,
    mode          TEXT NOT NULL,
    submode       TEXT,
    freq_hz       INTEGER,
    freq_rx_hz    INTEGER,
    band_rx       TEXT,
    rst_sent      TEXT,
    rst_rcvd      TEXT,
    gridsquare    TEXT,
    dxcc          INTEGER,
    station_id    INTEGER NOT NULL REFERENCES station(id),
    extra         TEXT NOT NULL DEFAULT '{}',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX idx_qso_match ON qso(call, band, mode, qso_date);
CREATE INDEX idx_qso_call  ON qso(call);

CREATE TABLE qso_service (
    qso_id       INTEGER NOT NULL REFERENCES qso(id) ON DELETE CASCADE,
    service      TEXT NOT NULL,
    uploaded_at  TEXT,
    confirmed_at TEXT,
    detail       TEXT,
    PRIMARY KEY (qso_id, service)
);

CREATE TABLE sync_state (
    service      TEXT PRIMARY KEY,
    last_marker  TEXT,
    last_run_at  TEXT
);
"#;

/// Connection PRAGMAs applied at open. WAL + `synchronous=FULL` prioritizes
/// durability over throughput — a QSO cannot be re-made, so a committed contact
/// must survive a crash. `foreign_keys` enforces the `station_id` reference.
pub const PRAGMAS: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA foreign_keys = ON;
"#;

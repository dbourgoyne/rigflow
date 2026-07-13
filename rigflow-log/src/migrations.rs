//! Minimal migration mechanism keyed on `PRAGMA user_version`.
//!
//! A schema this small doesn't need a migration framework: each step bumps
//! `user_version` by one inside a transaction. Adding a future column means
//! appending a step and incrementing [`schema::SCHEMA_VERSION`].

use rusqlite::Connection;

use crate::error::LogError;
use crate::schema;

/// Bring `conn` up to [`schema::SCHEMA_VERSION`], applying only the steps its
/// current `user_version` is missing. Idempotent: a fully-migrated database is
/// a no-op; reopening never re-runs a step.
pub fn migrate(conn: &Connection) -> Result<(), LogError> {
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    while version < schema::SCHEMA_VERSION {
        let tx = conn.unchecked_transaction()?;
        match version {
            0 => {
                tx.execute_batch(schema::V1_DDL)?;
            }
            1 => {
                tx.execute_batch(schema::V2_DDL)?;
            }
            other => {
                return Err(LogError::UnknownSchemaVersion(other));
            }
        }
        version += 1;
        // user_version can't be bound as a parameter; it's an integer we
        // control, so format it directly.
        tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        tx.commit()?;
    }
    Ok(())
}

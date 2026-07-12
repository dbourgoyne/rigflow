//! Crate error type.

use crate::adif::AdifError;

#[derive(Debug)]
pub enum LogError {
    Db(rusqlite::Error),
    Io(std::io::Error),
    Adif(AdifError),
    Json(serde_json::Error),
    /// A database reported a `user_version` this build doesn't know how to
    /// migrate (e.g. opened by a newer rigflow).
    UnknownSchemaVersion(i64),
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogError::Db(e) => write!(f, "database error: {e}"),
            LogError::Io(e) => write!(f, "io error: {e}"),
            LogError::Adif(e) => write!(f, "adif error: {e}"),
            LogError::Json(e) => write!(f, "json error: {e}"),
            LogError::UnknownSchemaVersion(v) => {
                write!(
                    f,
                    "unknown schema version {v} (database newer than this build?)"
                )
            }
        }
    }
}

impl std::error::Error for LogError {}

impl From<rusqlite::Error> for LogError {
    fn from(e: rusqlite::Error) -> Self {
        LogError::Db(e)
    }
}
impl From<std::io::Error> for LogError {
    fn from(e: std::io::Error) -> Self {
        LogError::Io(e)
    }
}
impl From<AdifError> for LogError {
    fn from(e: AdifError) -> Self {
        LogError::Adif(e)
    }
}
impl From<serde_json::Error> for LogError {
    fn from(e: serde_json::Error) -> Self {
        LogError::Json(e)
    }
}

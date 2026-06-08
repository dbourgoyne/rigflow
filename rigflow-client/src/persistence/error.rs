use std::{fmt, io, path::PathBuf};

#[derive(Debug)]
pub enum PersistenceError {
    Io(io::Error),
    Json(serde_json::Error),
    NoConfigDirectory,
    InvalidOperatorId,
    InvalidPath(PathBuf),
    /// Schema version is newer than this build understands.
    Migration(String),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Json(err) => write!(f, "JSON error: {err}"),
            Self::NoConfigDirectory => {
                write!(f, "could not determine config directory")
            }
            Self::InvalidOperatorId => write!(f, "invalid operator id"),
            Self::InvalidPath(path) => {
                write!(f, "invalid persistence path: {}", path.display())
            }
            Self::Migration(msg) => write!(f, "migration error: {msg}"),
        }
    }
}

impl PersistenceError {
    /// True when the bytes on disk are bad (unparseable / fail to deserialize).
    /// These are recoverable by quarantining the file and resetting to defaults.
    pub fn is_content_corruption(&self) -> bool {
        matches!(self, Self::Json(_))
    }

    /// True when a config file is valid but written by a *newer* build than this
    /// one (the only `Migration` error today — a downgrade). The file is good
    /// data, so it is preserved rather than quarantined.
    pub fn is_version_too_new(&self) -> bool {
        matches!(self, Self::Migration(_))
    }
}

impl std::error::Error for PersistenceError {}

impl From<io::Error> for PersistenceError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

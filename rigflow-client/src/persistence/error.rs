use std::{fmt, io, path::PathBuf};

#[derive(Debug)]
pub enum PersistenceError {
    Io(io::Error),
    Json(serde_json::Error),
    NoConfigDirectory,
    InvalidOperatorId,
    InvalidPath(PathBuf),
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
        }
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

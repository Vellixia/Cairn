//! The shared error type for Cairn.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    /// Storage-layer failures (e.g. SQLite). Kept as a string so `cairn-core` need not depend on
    /// any particular storage backend.
    #[error("storage error: {0}")]
    Storage(String),

    #[error("{0}")]
    Other(String),

    #[error("path escapes workspace root: {0}")]
    WorkspaceEscape(PathBuf),
}

pub type Result<T> = std::result::Result<T, Error>;

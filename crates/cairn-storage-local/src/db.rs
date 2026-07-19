//! T011: pool bootstrap, pragmas, migrations, corruption detection.

use std::path::PathBuf;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("local state corrupted or unavailable: {0}")]
    Corrupted(String),
    #[error("record not found")]
    NotFound,
    #[error("uniqueness conflict: {0}")]
    Conflict(String),
    #[error("illegal state transition: {0}")]
    IllegalTransition(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

impl StorageError {
    /// True when the error indicates unreadable/corrupt local state (FR-033).
    pub fn is_corruption(&self) -> bool {
        match self {
            StorageError::Corrupted(_) => true,
            StorageError::Sqlx(sqlx::Error::Database(db)) => {
                let msg = db.message().to_lowercase();
                msg.contains("malformed") || msg.contains("not a database")
            }
            _ => false,
        }
    }
}

/// Cairn data directory. `CAIRN_DATA_DIR` overrides (tests, portable setups).
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CAIRN_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cairn")
}

pub fn db_path() -> PathBuf {
    data_dir().join("cairn.db")
}

/// Open (creating if needed) the local database, run migrations, verify
/// integrity. Corruption is reported, never masked (FR-033).
pub async fn open_pool() -> Result<SqlitePool, StorageError> {
    open_pool_at(&db_path()).await
}

/// Open a database at an explicit path (tests, tooling).
pub async fn open_pool_at(path: &std::path::Path) -> Result<SqlitePool, StorageError> {
    let path = path.to_path_buf();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| StorageError::Corrupted(format!("cannot create data dir: {e}")))?;
    }

    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(StorageError::Sqlx)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .map_err(|e| StorageError::Corrupted(format!("cannot open {}: {e}", path.display())))?;

    // Fast integrity probe before trusting the file.
    let check: (String,) = sqlx::query_as("PRAGMA quick_check(1)")
        .fetch_one(&pool)
        .await
        .map_err(|e| StorageError::Corrupted(format!("integrity check failed: {e}")))?;
    if check.0 != "ok" {
        return Err(StorageError::Corrupted(format!("quick_check: {}", check.0)));
    }

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|e| StorageError::Corrupted(format!("migration failed: {e}")))?;

    Ok(pool)
}

//! T011: pool bootstrap, pragmas, migrations, corruption detection.

use std::path::PathBuf;
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use thiserror::Error;

pub const SUPPORTED_SCHEMA_VERSION: i64 = 2;
const MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

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
    #[error("database migration failed for schema version {target_version}")]
    MigrationFailed { target_version: i64 },
    #[error("storage remained busy for {max_elapsed_ms}ms")]
    StorageBusy { max_elapsed_ms: u64 },
    #[error("idempotency key conflicts with an earlier operation")]
    IdempotencyConflict {
        existing_method: String,
        reason: IdempotencyConflictReason,
    },
    #[error("session is already bound differently")]
    SessionAlreadyBound {
        existing_project_id: String,
        existing_revision_id: String,
    },
    #[error("project/task scope is required for a new session")]
    ProjectScopeRequired { project_id: String },
    #[error("healthy session scope conflicts with the requested scope")]
    SessionScopeConflict {
        session_id: String,
        existing_mode: String,
        requested_mode: String,
    },
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdempotencyConflictReason {
    MethodMismatch,
    RequestMismatch,
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

    pub fn is_migration_failure(&self) -> bool {
        matches!(self, StorageError::MigrationFailed { .. })
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

    fail_closed_on_future_schema(&pool).await?;

    MIGRATOR
        .run(&pool)
        .await
        .map_err(|_| StorageError::MigrationFailed {
            target_version: SUPPORTED_SCHEMA_VERSION,
        })?;

    Ok(pool)
}

async fn fail_closed_on_future_schema(pool: &SqlitePool) -> Result<(), StorageError> {
    let (has_migrations,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| StorageError::MigrationFailed {
        target_version: SUPPORTED_SCHEMA_VERSION,
    })?;
    if has_migrations != 0 {
        let (version,): (Option<i64>,) =
            sqlx::query_as("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
                .fetch_one(pool)
                .await
                .map_err(|_| StorageError::MigrationFailed {
                    target_version: SUPPORTED_SCHEMA_VERSION,
                })?;
        if version.is_some_and(|v| v > SUPPORTED_SCHEMA_VERSION) {
            return Err(StorageError::MigrationFailed {
                target_version: SUPPORTED_SCHEMA_VERSION,
            });
        }
    }

    let (has_meta,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'")
            .fetch_one(pool)
            .await
            .map_err(|_| StorageError::MigrationFailed {
                target_version: SUPPORTED_SCHEMA_VERSION,
            })?;
    if has_meta != 0 {
        let version: Option<(String,)> =
            sqlx::query_as("SELECT value FROM meta WHERE key='local_schema_version'")
                .fetch_optional(pool)
                .await
                .map_err(|_| StorageError::MigrationFailed {
                    target_version: SUPPORTED_SCHEMA_VERSION,
                })?;
        if version
            .and_then(|(value,)| value.parse::<i64>().ok())
            .is_some_and(|v| v > SUPPORTED_SCHEMA_VERSION)
        {
            return Err(StorageError::MigrationFailed {
                target_version: SUPPORTED_SCHEMA_VERSION,
            });
        }
    }
    Ok(())
}

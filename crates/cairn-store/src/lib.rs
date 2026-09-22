//! Local edge storage: binding, correlation, integration ownership, typed
//! delivery spools, migration artifacts, diagnostics and transactions.

pub mod constraints;
pub mod cursor;
pub mod diag;
pub mod integrations;
pub mod migrate;
pub mod repo;
pub mod rows;
/// Feature 005's edge spools: approved events and knowledge commands waiting
/// for the server, with durable ordinals and an exact per-account claim.
pub mod spool;
/// Versioned, file-backed V1 export/import manifests and retry-safe restore.
pub mod transfer;
pub mod tx;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlx(sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] migrate::MigrateError),
    #[error("{0} not found")]
    NotFound(String),
    #[error("invalid stored value: {0}")]
    Corrupt(String),
    /// A refusal the caller can act on, carrying its stable wire code.
    ///
    /// Distinct from `Corrupt` and from a bare `Sqlx` error: a refusal means the
    /// store understood the request and declined it for a reason the contract
    /// names, so the daemon can surface that code verbatim rather than matching
    /// on message text.
    #[error("{code}: {message}")]
    Refused { code: &'static str, message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Conversion by hand rather than `#[from]`, so every SQLite error passes one
/// point where contention can be reported (`diag`). Without this a lost write
/// arrives at the user as "database is locked" with no way to tell which
/// statement lost, or at which stage.
impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        if diag::enabled() && diag::is_contention(&e) {
            diag::record("?", diag::Stage::Unknown, "?", 0, &e);
        }
        StoreError::Sqlx(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// A handle on the local database.
#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

/// How long opening a store may spend waiting for its first connection.
///
/// Opening is create-if-missing plus `PRAGMA journal_mode = WAL`, which is a
/// handful of small writes; ten seconds is a budget for a wedged disk, not for
/// a busy one.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

impl Store {
    /// Open (creating if needed) the database at `path` and migrate it.
    ///
    /// WAL plus a busy timeout is what lets concurrent sessions in different
    /// worktrees write without coordination (D12).
    pub async fn open(path: &Path) -> Result<Self> {
        Self::open_with_busy_timeout(path, Duration::from_secs(5)).await
    }

    /// `open`, with the busy timeout chosen by the caller.
    ///
    /// Only the contention tests use this: they need SQLite to give up quickly
    /// so that exercising a fully exhausted retry takes seconds rather than the
    /// best part of a minute.
    pub(crate) async fn open_with_busy_timeout(path: &Path, busy: Duration) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(busy)
            // Deletion must actually erase. Without this, cleared content stays
            // legible in freed pages until a VACUUM, which is not what FR-052
            // promises a developer who deleted something.
            .pragma("secure_delete", "ON");

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .acquire_timeout(OPEN_TIMEOUT)
            .connect_with(options)
            .await
            // **Name the file and the budget.** `PoolTimedOut` is what sqlx
            // reports when the pool could not hand out a connection in time,
            // and it says nothing about which store or how long it waited —
            // the underlying connect error is swallowed with it. That reached
            // a user as `storage_unavailable: PoolTimedOut`, and reached a
            // test as one line naming a pool. It is a real state: a private,
            // newly created database timing out here cannot be contending with
            // anybody, so it is the machine and the message should say which
            // machine and which file.
            .map_err(|e| match e {
                sqlx::Error::PoolTimedOut => StoreError::Corrupt(format!(
                    "{} could not be opened within {}s",
                    path.display(),
                    OPEN_TIMEOUT.as_secs()
                )),
                other => StoreError::from(other),
            })?;

        migrate::run(&pool).await?;
        Ok(Self { pool })
    }

    /// An in-memory database, for tests that do not need a file.
    pub async fn open_memory() -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .foreign_keys(true)
            .shared_cache(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        migrate::run(&pool).await?;
        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Fold the write-ahead log back into the database file.
    ///
    /// Called after a deletion so the removed content leaves the WAL too,
    /// rather than lingering in an old frame (FR-052).
    pub async fn checkpoint(&self) -> Result<()> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrates_and_reports_schema_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .unwrap();
        let v: i64 = sqlx::query_scalar("SELECT MAX(version) FROM schema_migrations")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(v, migrate::latest_version());
    }

    #[tokio::test]
    async fn migration_is_idempotent_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cairn.sqlite3");
        let a = Store::open(&path).await.unwrap();
        a.close().await;
        let b = Store::open(&path).await.unwrap();
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_migrations")
            .fetch_one(b.pool())
            .await
            .unwrap();
        assert_eq!(n, migrate::MIGRATIONS.len() as i64);
    }

    #[tokio::test]
    async fn refuses_a_newer_schema_than_this_build_supports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cairn.sqlite3");
        let store = Store::open(&path).await.unwrap();
        sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1,?2,?3)")
            .bind(migrate::latest_version() + 5)
            .bind("from-the-future")
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(store.pool())
            .await
            .unwrap();
        store.close().await;

        match Store::open(&path).await {
            Err(StoreError::Migrate(migrate::MigrateError::TooNew { .. })) => {}
            other => panic!("expected TooNew, got {other:?}"),
        }
    }
}

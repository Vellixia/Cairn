#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

pub struct TestDatabase {
    pub dir: tempfile::TempDir,
    pub path: PathBuf,
    pub pool: SqlitePool,
}

impl TestDatabase {
    pub async fn empty() -> Self {
        let dir = tempfile::TempDir::new().expect("test database tempdir");
        let path = dir.path().join("cairn.db");
        let pool = cairn_storage_local::open_pool_at(&path)
            .await
            .expect("empty migrated test database");
        Self { dir, path, pool }
    }

    pub async fn from_feature001_fixture() -> Self {
        let dir = tempfile::TempDir::new().expect("fixture database tempdir");
        let path = dir.path().join("cairn.db");
        std::fs::copy(feature001_fixture_path(), &path).expect("copy Feature 001 fixture");
        let pool = cairn_storage_local::open_pool_at(&path)
            .await
            .expect("migrate Feature 001 fixture copy");
        Self { dir, path, pool }
    }
}

pub fn feature001_fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/databases/feature-001-v1.sqlite3")
}

pub async fn independent_pool(path: &Path, busy_timeout: Duration) -> SqlitePool {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .expect("sqlite options")
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(busy_timeout);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("independent pool")
}

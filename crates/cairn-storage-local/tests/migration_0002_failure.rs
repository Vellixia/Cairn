mod support;

use std::path::Path;
use std::str::FromStr;

use cairn_storage_local::{open_pool_at, StorageError};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use support::feature001_fixture_path;

async fn fixture_copy() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("cairn.db");
    std::fs::copy(feature001_fixture_path(), &path).unwrap();
    (dir, path)
}

async fn raw_pool(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .create_if_missing(false)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap()
}

async fn has_column(pool: &SqlitePool, table: &str, column: &str) -> bool {
    let query = format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?");
    let (count,): (i64,) = sqlx::query_as(&query)
        .bind(column)
        .fetch_one(pool)
        .await
        .unwrap();
    count == 1
}

fn assert_bounded(error: StorageError) {
    assert!(error.is_migration_failure());
    assert_eq!(
        error.to_string(),
        "database migration failed for schema version 2"
    );
    let rendered = format!("{error:?}");
    assert!(!rendered.contains(".sqlite"));
    assert!(!rendered.contains("checksum"));
    assert!(!rendered.contains("CREATE TABLE"));
}

#[tokio::test]
async fn migration_failure_rolls_back_then_retries_and_second_open_is_gated() {
    let (_dir, path) = fixture_copy().await;
    let raw = raw_pool(&path).await;
    sqlx::query("CREATE TABLE projects (collision INTEGER)")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    assert_bounded(open_pool_at(&path).await.unwrap_err());
    let raw = raw_pool(&path).await;
    assert!(!has_column(&raw, "sessions", "binding_mode").await);
    assert!(!has_column(&raw, "events", "aggregate_type").await);
    let (version,): (i64,) =
        sqlx::query_as("SELECT MAX(version) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(&raw)
            .await
            .unwrap();
    assert_eq!(version, 1);
    sqlx::query("DROP TABLE projects")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    let first = open_pool_at(&path).await.unwrap();
    assert!(has_column(&first, "sessions", "binding_mode").await);
    first.close().await;
    let second = open_pool_at(&path).await.unwrap();
    let (version,): (i64,) =
        sqlx::query_as("SELECT MAX(version) FROM _sqlx_migrations WHERE success=1")
            .fetch_one(&second)
            .await
            .unwrap();
    assert_eq!(version, 2);
}

#[tokio::test]
async fn recorded_checksum_mismatch_fails_closed_before_upgrade() {
    let (_dir, path) = fixture_copy().await;
    let raw = raw_pool(&path).await;
    sqlx::query("UPDATE _sqlx_migrations SET checksum=zeroblob(48) WHERE version=1")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    assert_bounded(open_pool_at(&path).await.unwrap_err());
    let raw = raw_pool(&path).await;
    assert!(!has_column(&raw, "sessions", "binding_mode").await);
}

#[tokio::test]
async fn unsupported_future_schema_version_fails_closed_before_upgrade() {
    let (_dir, path) = fixture_copy().await;
    let raw = raw_pool(&path).await;
    sqlx::query("INSERT INTO _sqlx_migrations (version,description,installed_on,success,checksum,execution_time) VALUES (3,'future',CURRENT_TIMESTAMP,1,zeroblob(48),0)")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    assert_bounded(open_pool_at(&path).await.unwrap_err());
    let raw = raw_pool(&path).await;
    assert!(!has_column(&raw, "sessions", "binding_mode").await);
}

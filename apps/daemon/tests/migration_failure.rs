use std::str::FromStr;

use cairn_daemon::{AppState, DaemonConfig};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn migration_failure_never_constructs_a_partially_healthy_daemon() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("cairn.db");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/databases/feature-001-v1.sqlite3");
    std::fs::copy(fixture, &db_path).unwrap();
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
        .unwrap()
        .create_if_missing(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum=zeroblob(48) WHERE version=1")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let mut config = DaemonConfig::from_env();
    config.data_dir = dir.path().to_path_buf();
    config.socket_path = dir.path().join("daemon.sock");
    let error = match AppState::init(config).await {
        Ok(_) => panic!("migration failure constructed AppState"),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "database migration failed for schema version 2"
    );
    assert!(!error.to_string().contains("checksum"));
    assert!(!error.to_string().contains(".sqlite"));
}

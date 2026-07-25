mod support;

use support::TestDatabase;

#[tokio::test]
async fn actual_runner_applies_0001_then_0002_with_complete_schema() {
    let db = TestDatabase::empty().await;
    let migrations: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT version, description, success FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        migrations,
        vec![(1, "init".into(), 1), (2, "project task binding".into(), 1)]
    );
    let (schema_version,): (String,) =
        sqlx::query_as("SELECT value FROM meta WHERE key='local_schema_version'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(schema_version, "2");

    for table in [
        "projects",
        "project_repository_associations",
        "tasks",
        "task_revisions",
        "session_bindings",
        "operation_idempotency",
        "event_aggregate_heads",
    ] {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "missing table {table}");
    }
    for index in [
        "events_one_aggregate_seq",
        "projects_by_status_id",
        "tasks_by_project_id",
        "operation_idempotency_by_result",
    ] {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?")
                .bind(index)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "missing index {index}");
    }
    for trigger in [
        "events_require_explicit_aggregate",
        "task_revisions_no_update",
        "session_bindings_no_delete",
        "operation_idempotency_no_update",
    ] {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name=?")
                .bind(trigger)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "missing trigger {trigger}");
    }
    let (foreign_keys,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);
    let (quick_check,): (String,) = sqlx::query_as("PRAGMA quick_check")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(quick_check, "ok");
}

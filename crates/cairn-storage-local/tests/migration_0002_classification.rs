mod support;

use support::TestDatabase;

#[tokio::test]
async fn migration_classifies_only_and_fabricates_nothing() {
    let db = TestDatabase::from_feature001_fixture().await;
    let (sessions, unbound): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), SUM(binding_mode='local_unbound') FROM sessions")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(sessions > 0);
    assert_eq!(sessions, unbound);

    for table in [
        "projects",
        "project_repository_associations",
        "tasks",
        "task_revisions",
        "session_bindings",
        "operation_idempotency",
        "event_aggregate_heads",
    ] {
        let (count,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table} must not be fabricated");
    }
    let (feature002_events,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE event_type IN ('project.created','project.updated','project.repository_associated','task.created','task.revision_created','session.bound')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(feature002_events, 0);
    let (legacy, null_scopes): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), SUM(aggregate_type IS NULL AND aggregate_id IS NULL AND aggregate_seq IS NULL) FROM events",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(legacy > 0);
    assert_eq!(legacy, null_scopes);
}

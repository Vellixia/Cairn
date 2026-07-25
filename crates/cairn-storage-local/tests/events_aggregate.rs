mod support;

use cairn_storage_local::aggregate_events::derive_event_key;
use cairn_storage_local::events::{append_event, NewEvent};
use cairn_storage_local::writer;
use cairn_storage_local::WriterPolicy;
use support::TestDatabase;

type AggregateRow = (String, String, i64, Option<String>, Option<String>);

fn event(key: String, event_type: &str, aggregate_type: &str, aggregate_id: &str) -> NewEvent {
    NewEvent {
        id: uuid::Uuid::now_v7().to_string(),
        idempotency_key: key,
        event_type: event_type.into(),
        repository_id: None,
        worktree_id: None,
        session_id: None,
        snapshot_id: None,
        aggregate_type: aggregate_type.into(),
        aggregate_id: aggregate_id.into(),
        payload: serde_json::json!({"schema_version": 1}),
        recorded_at: "2026-07-22T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn legacy_rows_stay_null_and_new_rows_have_real_contiguous_scope() {
    let db = TestDatabase::from_feature001_fixture().await;
    let (legacy_count, legacy_max): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*),MAX(seq) FROM events")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let duplicate_key = derive_event_key("operation", "project.create", 0, "project.created");
    let (first, second, other, duplicate) = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                let first = append_event(
                    conn,
                    &event(duplicate_key.clone(), "project.created", "project", "p1"),
                )
                .await?;
                let second = append_event(
                    conn,
                    &event(
                        derive_event_key("operation", "project.update", 0, "project.updated"),
                        "project.updated",
                        "project",
                        "p1",
                    ),
                )
                .await?;
                let other = append_event(
                    conn,
                    &event(
                        derive_event_key("operation-2", "task.create", 0, "task.created"),
                        "task.created",
                        "task",
                        "t1",
                    ),
                )
                .await?;
                let duplicate = append_event(
                    conn,
                    &event(duplicate_key, "project.created", "project", "p1"),
                )
                .await?;
                Ok((first, second, other, duplicate))
            })
        }),
    )
    .await
    .unwrap();
    assert_eq!(first.seq, legacy_max + 1);
    assert_eq!(second.seq, legacy_max + 2);
    assert_eq!(other.seq, legacy_max + 3);
    assert!(duplicate.deduplicated);
    assert_eq!(duplicate.seq, first.seq);

    let legacy_null: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events WHERE seq<=? AND aggregate_type IS NULL AND aggregate_id IS NULL AND aggregate_seq IS NULL",
    )
    .bind(legacy_max)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(legacy_null.0, legacy_count);
    let rows: Vec<AggregateRow> = sqlx::query_as(
        "SELECT aggregate_type,aggregate_id,aggregate_seq,repository_id,worktree_id FROM events WHERE seq>? ORDER BY seq",
    )
    .bind(legacy_max)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("project".into(), "p1".into(), 1, None, None),
            ("project".into(), "p1".into(), 2, None, None),
            ("task".into(), "t1".into(), 1, None, None),
        ]
    );
    assert!(!rows.iter().any(|row| row.1.starts_with("__")));
}

#[tokio::test]
async fn incomplete_or_fake_aggregate_scope_is_rejected() {
    let db = TestDatabase::empty().await;
    let bad = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                append_event(
                    conn,
                    &event("bad".into(), "project.created", "project", " "),
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await;
    assert!(bad.is_err());
    assert!(sqlx::query("INSERT INTO events (id,idempotency_key,event_type,payload,recorded_at) VALUES ('x','x','project.created','{}','t')")
        .execute(&db.pool)
        .await
        .is_err());
}

mod support;

use std::time::{Duration, Instant};

use cairn_storage_local::events::{append_event, NewEvent};
use cairn_storage_local::records::{OperationIdempotencyRow, ProjectRow, TaskRevisionRow, TaskRow};
use cairn_storage_local::{
    operation_idempotency, projects, tasks, writer, StorageError, WriteCheckpoint, WriteTestHooks,
    WriterPolicy,
};
use sqlx::Connection;
use support::{independent_pool, TestDatabase};

fn event(index: usize) -> NewEvent {
    NewEvent {
        id: uuid::Uuid::now_v7().to_string(),
        idempotency_key: format!("concurrent-{index}"),
        event_type: "project.updated".into(),
        repository_id: None,
        worktree_id: None,
        session_id: None,
        snapshot_id: None,
        aggregate_type: "project".into(),
        aggregate_id: "p".into(),
        payload: serde_json::json!({"index": index}),
        recorded_at: "2026-07-22T00:00:00Z".into(),
    }
}

fn revision(id: &str, task_id: &str, parent: Option<&str>) -> TaskRevisionRow {
    TaskRevisionRow {
        id: id.into(),
        task_id: task_id.into(),
        revision_number: 0,
        parent_revision_id: parent.map(str::to_string),
        goal_contract_json: r#"{"version":1,"goal":"g","included_scope":[],"excluded_scope":[],"acceptance_criteria":[],"constraints":[]}"#.into(),
        goal_contract_schema_version: 1,
        goal_contract_fingerprint: "a".repeat(64),
        created_at: "t".into(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_connections_serialize_global_and_aggregate_sequences() {
    let db = TestDatabase::empty().await;
    let mut handles = Vec::new();
    for index in 0..12 {
        let pool = independent_pool(&db.path, Duration::from_secs(5)).await;
        handles.push(tokio::spawn(async move {
            writer::begin_immediate(
                &pool,
                WriterPolicy::default(),
                None,
                Box::new(move |conn| {
                    Box::pin(async move { Ok(append_event(conn, &event(index)).await?.seq) })
                }),
            )
            .await
            .unwrap()
        }));
    }
    let mut global = Vec::new();
    for handle in handles {
        global.push(handle.await.unwrap());
    }
    global.sort_unstable();
    assert_eq!(global, (1..=12).collect::<Vec<_>>());
    let aggregate: Vec<(i64,)> =
        sqlx::query_as("SELECT aggregate_seq FROM events ORDER BY aggregate_seq")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        aggregate.into_iter().map(|row| row.0).collect::<Vec<_>>(),
        (1..=12).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn lock_exhaustion_is_bounded_and_typed_without_retry() {
    let db = TestDatabase::empty().await;
    let holder_pool = independent_pool(&db.path, Duration::from_secs(5)).await;
    let contender_pool = independent_pool(&db.path, Duration::from_millis(50)).await;
    let mut holder = holder_pool.acquire().await.unwrap();
    let tx = holder.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let started = Instant::now();
    let error = writer::begin_immediate(
        &contender_pool,
        WriterPolicy::test_with_busy_timeout(Duration::from_millis(50)),
        None,
        Box::new(|_| Box::pin(async { Ok(()) })),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        StorageError::StorageBusy { max_elapsed_ms: 50 }
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn cancellation_rolls_back_before_connection_reuse() {
    let db = TestDatabase::empty().await;
    sqlx::query("CREATE TABLE probe (value INTEGER)")
        .execute(&db.pool)
        .await
        .unwrap();
    let hooks = WriteTestHooks::default();
    hooks.pause_at(WriteCheckpoint::PreCommit);
    let task_pool = independent_pool(&db.path, Duration::from_secs(1)).await;
    let task_hooks = hooks.clone();
    let task = tokio::spawn(async move {
        writer::begin_immediate(
            &task_pool,
            WriterPolicy::default(),
            Some(task_hooks),
            Box::new(|conn| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO probe VALUES (1)")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            }),
        )
        .await
    });
    hooks.wait_until_reached(WriteCheckpoint::PreCommit).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());

    let pool = independent_pool(&db.path, Duration::from_secs(1)).await;
    writer::begin_immediate(
        &pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO probe VALUES (2)")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();
    let values: Vec<(i64,)> = sqlx::query_as("SELECT value FROM probe")
        .fetch_all(&db.pool)
        .await
        .unwrap();
    assert_eq!(values, vec![(2,)]);
}

#[tokio::test]
async fn every_injected_mutation_boundary_rolls_back() {
    let db = TestDatabase::empty().await;
    sqlx::query("CREATE TABLE probe (phase TEXT)")
        .execute(&db.pool)
        .await
        .unwrap();
    for point in [
        WriteCheckpoint::PreEvent,
        WriteCheckpoint::BetweenEvents,
        WriteCheckpoint::PreProjection,
        WriteCheckpoint::PreCommit,
    ] {
        let hooks = WriteTestHooks::default();
        hooks.fail_at(point);
        let closure_hooks = hooks.clone();
        let result = writer::begin_immediate(
            &db.pool,
            WriterPolicy::default(),
            Some(hooks),
            Box::new(move |conn| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO probe VALUES (?)")
                        .bind(format!("{point:?}"))
                        .execute(&mut *conn)
                        .await?;
                    if point != WriteCheckpoint::PreCommit {
                        closure_hooks.checkpoint(point).await?;
                    }
                    Ok(())
                })
            }),
        )
        .await;
        assert!(result.is_err(), "{point:?} must fail");
    }
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM probe")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn registry_and_revision_counter_roll_back_then_allocate_gap_free() {
    let db = TestDatabase::empty().await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::PostRegistryReservation);
    let closure_hooks = hooks.clone();
    let record = OperationIdempotencyRow {
        idempotency_key: uuid::Uuid::now_v7().to_string(),
        method: "project.create".into(),
        request_fingerprint: "b".repeat(64),
        result_kind: "event".into(),
        result_locator: "e".into(),
        created_at: "t".into(),
    };
    let result = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                operation_idempotency::reserve_or_get(conn, &record, Some(&closure_hooks)).await?;
                Ok(())
            })
        }),
    )
    .await;
    assert!(result.is_err());
    let (registry_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM operation_idempotency")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(registry_count, 0);

    writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                projects::insert(
                    conn,
                    &ProjectRow {
                        id: "p".into(),
                        name: "p".into(),
                        description: None,
                        status: "active".into(),
                        created_at: "t".into(),
                        updated_at: "t".into(),
                    },
                )
                .await?;
                let mut first = revision("r1", "task", None);
                first.revision_number = 1;
                tasks::insert_task(
                    conn,
                    &TaskRow {
                        id: "task".into(),
                        project_id: "p".into(),
                        title: "task".into(),
                        latest_revision_number: 1,
                        created_at: "t".into(),
                        updated_at: "t".into(),
                    },
                    &first,
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    let counter_hooks = WriteTestHooks::default();
    counter_hooks.fail_at(WriteCheckpoint::PostCounterAllocation);
    let closure_hooks = counter_hooks.clone();
    let failed = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                tasks::insert_next_revision(
                    conn,
                    revision("r2-failed", "task", Some("r1")),
                    "t2",
                    Some(&closure_hooks),
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await;
    assert!(failed.is_err());
    let (counter, revisions): (i64, i64) = sqlx::query_as(
        "SELECT latest_revision_number,(SELECT COUNT(*) FROM task_revisions WHERE task_id='task') FROM tasks WHERE id='task'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((counter, revisions), (1, 1));

    let created = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                tasks::insert_next_revision(conn, revision("r2", "task", Some("r1")), "t2", None)
                    .await
            })
        }),
    )
    .await
    .unwrap();
    assert_eq!(created.revision_number, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_raw_key_registry_first_committed_operation_wins() {
    let db = TestDatabase::empty().await;
    sqlx::query("CREATE TABLE idempotency_probe (winner TEXT)")
        .execute(&db.pool)
        .await
        .unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let key = uuid::Uuid::now_v7().to_string();
    let mut handles = Vec::new();
    for index in 0..2 {
        let pool = independent_pool(&db.path, Duration::from_secs(2)).await;
        let barrier = barrier.clone();
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            writer::begin_immediate(
                &pool,
                WriterPolicy::default(),
                None,
                Box::new(move |conn| {
                    Box::pin(async move {
                        let proposed = OperationIdempotencyRow {
                            idempotency_key: key,
                            method: "project.create".into(),
                            request_fingerprint: "d".repeat(64),
                            result_kind: "event".into(),
                            result_locator: format!("winner-{index}"),
                            created_at: "t".into(),
                        };
                        match operation_idempotency::reserve_or_get(conn, &proposed, None).await? {
                            operation_idempotency::Reservation::Inserted(row) => {
                                sqlx::query("INSERT INTO idempotency_probe VALUES (?)")
                                    .bind(&row.result_locator)
                                    .execute(&mut *conn)
                                    .await?;
                                let mut accepted_event = event(99);
                                accepted_event.idempotency_key = "idempotent-event".into();
                                append_event(conn, &accepted_event).await?;
                                Ok((true, row))
                            }
                            operation_idempotency::Reservation::Existing(row) => Ok((false, row)),
                        }
                    })
                }),
            )
            .await
            .unwrap()
        }));
    }
    barrier.wait().await;
    let first = handles.remove(0).await.unwrap();
    let second = handles.remove(0).await.unwrap();
    assert_ne!(first.0, second.0);
    assert_eq!(first.1, second.1);
    let (registry, probes, events): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM operation_idempotency),(SELECT COUNT(*) FROM idempotency_probe),(SELECT COUNT(*) FROM events)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!((registry, probes, events), (1, 1, 1));

    let conflicting = OperationIdempotencyRow {
        idempotency_key: key,
        method: "task.create".into(),
        request_fingerprint: "e".repeat(64),
        result_kind: "event".into(),
        result_locator: "different".into(),
        created_at: "t".into(),
    };
    let error = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                operation_idempotency::reserve_or_get(conn, &conflicting, None).await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, StorageError::IdempotencyConflict { .. }));
}

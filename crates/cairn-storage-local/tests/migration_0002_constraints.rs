mod support;

use cairn_storage_local::events::{append_event, NewEvent};
use cairn_storage_local::records::{
    OperationIdempotencyRow, ProjectRepositoryAssociationRow, ProjectRow, TaskRevisionRow, TaskRow,
};
use cairn_storage_local::{
    operation_idempotency, projects, tasks, writer, StorageError, WriterPolicy,
};
use support::TestDatabase;

fn event(key: &str, aggregate_type: &str, aggregate_id: &str) -> NewEvent {
    NewEvent {
        id: uuid::Uuid::now_v7().to_string(),
        idempotency_key: key.into(),
        event_type: "project.created".into(),
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

fn revision(id: &str, task_id: &str, number: i64, parent: Option<&str>) -> TaskRevisionRow {
    TaskRevisionRow {
        id: id.into(),
        task_id: task_id.into(),
        revision_number: number,
        parent_revision_id: parent.map(str::to_string),
        goal_contract_json: r#"{"version":1,"goal":"g","included_scope":[],"excluded_scope":[],"acceptance_criteria":[],"constraints":[]}"#.into(),
        goal_contract_schema_version: 1,
        goal_contract_fingerprint: "a".repeat(64),
        created_at: "2026-07-22T00:00:00Z".into(),
    }
}

#[tokio::test]
async fn foreign_keys_archive_guards_uniqueness_and_immutability_hold() {
    let db = TestDatabase::empty().await;
    sqlx::query("INSERT INTO repositories (id,repo_uuid,canonical_path,registered_at) VALUES ('r1','ru1','/r1','t'),('r2','ru2','/r2','t')")
        .execute(&db.pool)
        .await
        .unwrap();

    writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                projects::insert(
                    conn,
                    &ProjectRow {
                        id: "p1".into(),
                        name: "same".into(),
                        description: None,
                        status: "active".into(),
                        created_at: "t".into(),
                        updated_at: "t".into(),
                    },
                )
                .await?;
                projects::insert(
                    conn,
                    &ProjectRow {
                        id: "p2".into(),
                        name: "same".into(),
                        description: None,
                        status: "active".into(),
                        created_at: "t".into(),
                        updated_at: "t".into(),
                    },
                )
                .await?;
                let first = append_event(conn, &event("assoc-1", "project", "p1")).await?;
                projects::insert_association(
                    conn,
                    &ProjectRepositoryAssociationRow {
                        id: "a1".into(),
                        project_id: "p1".into(),
                        repository_id: "r1".into(),
                        associated_at: "t".into(),
                        event_seq: first.seq,
                    },
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    assert!(sqlx::query("DELETE FROM projects WHERE id='p1'")
        .execute(&db.pool)
        .await
        .is_err());
    assert!(
        sqlx::query("UPDATE projects SET id='changed' WHERE id='p1'")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM project_repository_associations WHERE id='a1'")
            .execute(&db.pool)
            .await
            .is_err()
    );

    sqlx::query("UPDATE projects SET status='archived',updated_at='t2' WHERE id='p2'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(sqlx::query("INSERT INTO tasks (id,project_id,title,latest_revision_number,created_at,updated_at) VALUES ('blocked','p2','x',1,'t','t')")
        .execute(&db.pool)
        .await
        .is_err());
    assert!(
        sqlx::query("UPDATE projects SET status='active',updated_at='t3' WHERE id='p2'")
            .execute(&db.pool)
            .await
            .is_ok()
    );

    let second_seq = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                Ok(append_event(conn, &event("assoc-2", "project", "p2"))
                    .await?
                    .seq)
            })
        }),
    )
    .await
    .unwrap();
    assert!(sqlx::query("INSERT INTO project_repository_associations (id,project_id,repository_id,associated_at,event_seq) VALUES ('a2','p2','r1','t',?)")
        .bind(second_seq)
        .execute(&db.pool)
        .await
        .is_err());
}

#[tokio::test]
async fn task_revision_and_operation_registry_constraints_are_closed() {
    let db = TestDatabase::empty().await;
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
                tasks::insert_task(
                    conn,
                    &TaskRow {
                        id: "task".into(),
                        project_id: "p".into(),
                        title: "duplicate".into(),
                        latest_revision_number: 1,
                        created_at: "t".into(),
                        updated_at: "t".into(),
                    },
                    &revision("rev1", "task", 1, None),
                )
                .await?;
                tasks::insert_task(
                    conn,
                    &TaskRow {
                        id: "task2".into(),
                        project_id: "p".into(),
                        title: "duplicate".into(),
                        latest_revision_number: 1,
                        created_at: "t".into(),
                        updated_at: "t".into(),
                    },
                    &revision("rev-other", "task2", 1, None),
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    assert!(
        sqlx::query("UPDATE task_revisions SET created_at='x' WHERE id='rev1'")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM task_revisions WHERE id='rev1'")
        .execute(&db.pool)
        .await
        .is_err());
    assert!(sqlx::query("UPDATE tasks SET title='x' WHERE id='task'")
        .execute(&db.pool)
        .await
        .is_err());
    assert!(
        sqlx::query("UPDATE tasks SET latest_revision_number=3 WHERE id='task'")
            .execute(&db.pool)
            .await
            .is_err()
    );

    let bad_parent = writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                let row = revision("rev2", "task", 0, Some("rev-other"));
                tasks::insert_next_revision(conn, row, "t2", None).await?;
                Ok(())
            })
        }),
    )
    .await;
    assert!(matches!(bad_parent, Err(StorageError::Conflict(_))));
    let (counter,): (i64,) =
        sqlx::query_as("SELECT latest_revision_number FROM tasks WHERE id='task'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(counter, 1);

    let registry = OperationIdempotencyRow {
        idempotency_key: uuid::Uuid::now_v7().to_string(),
        method: "project.create".into(),
        request_fingerprint: "b".repeat(64),
        result_kind: "event".into(),
        result_locator: "event-id".into(),
        created_at: "t".into(),
    };
    writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                operation_idempotency::reserve_or_get(conn, &registry, None).await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();
    assert!(
        sqlx::query("UPDATE operation_idempotency SET method='task.create'")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("DELETE FROM operation_idempotency")
        .execute(&db.pool)
        .await
        .is_err());
    assert!(sqlx::query("INSERT INTO operation_idempotency (idempotency_key,method,request_fingerprint,result_kind,result_locator,created_at) VALUES ('bad','not.closed',?,'event','x','t')")
        .bind("c".repeat(64)).execute(&db.pool).await.is_err());
}

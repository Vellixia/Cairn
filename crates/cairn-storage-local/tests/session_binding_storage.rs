mod support;

use cairn_storage_local::events::{append_event, NewEvent};
use cairn_storage_local::records::{
    ProjectRepositoryAssociationRow, ProjectRow, SessionBindingRow, TaskRevisionRow, TaskRow,
};
use cairn_storage_local::{
    projects, session_bindings, tasks, writer, RepositoryRow, SessionRow, SnapshotRow, WorktreeRow,
    WriterPolicy,
};
use support::TestDatabase;

fn event(key: &str, event_type: &str, aggregate_type: &str, aggregate_id: &str) -> NewEvent {
    NewEvent {
        id: uuid::Uuid::now_v7().to_string(),
        idempotency_key: key.into(),
        event_type: event_type.into(),
        repository_id: Some("r".into()),
        worktree_id: Some("w".into()),
        session_id: None,
        snapshot_id: None,
        aggregate_type: aggregate_type.into(),
        aggregate_id: aggregate_id.into(),
        payload: serde_json::json!({"schema_version": 1}),
        recorded_at: "t".into(),
    }
}

#[tokio::test]
async fn binding_projection_changes_scope_once_without_touching_lifecycle_or_history() {
    let db = TestDatabase::empty().await;
    writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                cairn_storage_local::repos::insert(
                    &mut *conn,
                    &RepositoryRow {
                        id: "r".into(),
                        repo_uuid: "ru".into(),
                        canonical_path: "/r".into(),
                        default_remote_name: None,
                        default_remote_url: None,
                        copied_from_repository_id: None,
                        registered_at: "t".into(),
                    },
                )
                .await?;
                cairn_storage_local::worktrees::insert(
                    &mut *conn,
                    &WorktreeRow {
                        id: "w".into(),
                        repository_id: "r".into(),
                        worktree_uuid: "wu".into(),
                        path: "/r".into(),
                        is_main: 1,
                        registered_at: "t".into(),
                    },
                )
                .await?;
                cairn_storage_local::snapshots::insert(
                    &mut *conn,
                    &SnapshotRow {
                        id: "snap".into(),
                        worktree_id: "w".into(),
                        branch: Some("main".into()),
                        head_commit: "h".into(),
                        staged_fp: "a".into(),
                        unstaged_fp: "b".into(),
                        untracked_fp: "c".into(),
                        snapshot_fp: "fp".into(),
                        fp_schema_version: 1,
                        created_at: "t".into(),
                    },
                )
                .await?;
                cairn_storage_local::sessions::insert(
                    &mut *conn,
                    &SessionRow {
                        id: "s".into(),
                        repository_id: "r".into(),
                        worktree_id: "w".into(),
                        local_user: "u".into(),
                        agent_type: "a".into(),
                        agent_instance_id: "i".into(),
                        agent_pid: None,
                        resume_token_hash: "hash".into(),
                        lease_expires_at: "lease".into(),
                        state: "active".into(),
                        start_snapshot_id: "snap".into(),
                        current_snapshot_id: "snap".into(),
                        started_at: "start".into(),
                        ended_at: None,
                        last_heartbeat_at: "heartbeat".into(),
                        recovering_since: None,
                        binding_mode: "local_unbound".into(),
                    },
                )
                .await?;
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
                let association_event =
                    append_event(conn, &event("association", "project.repository_associated", "project", "p")).await?;
                projects::insert_association(
                    conn,
                    &ProjectRepositoryAssociationRow {
                        id: "a".into(),
                        project_id: "p".into(),
                        repository_id: "r".into(),
                        associated_at: "t".into(),
                        event_seq: association_event.seq,
                    },
                )
                .await?;
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
                    &TaskRevisionRow {
                        id: "rev".into(),
                        task_id: "task".into(),
                        revision_number: 1,
                        parent_revision_id: None,
                        goal_contract_json: r#"{"version":1,"goal":"g","included_scope":[],"excluded_scope":[],"acceptance_criteria":[],"constraints":[]}"#.into(),
                        goal_contract_schema_version: 1,
                        goal_contract_fingerprint: "a".repeat(64),
                        created_at: "t".into(),
                    },
                )
                .await?;
                let mut binding_event = event("binding", "session.bound", "session", "s");
                binding_event.session_id = Some("s".into());
                let binding_event = append_event(conn, &binding_event).await?;
                session_bindings::insert(
                    conn,
                    &SessionBindingRow {
                        session_id: "s".into(),
                        project_id: "p".into(),
                        task_revision_id: "rev".into(),
                        bound_at: "bound".into(),
                        binding_event_seq: binding_event.seq,
                    },
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    let session = cairn_storage_local::sessions::get_by_id(&db.pool, "s")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.binding_mode, "project_bound");
    assert_eq!(session.state, "active");
    assert_eq!(session.resume_token_hash, "hash");
    assert_eq!(session.current_snapshot_id, "snap");
    let binding = session_bindings::get(&db.pool, "s").await.unwrap().unwrap();
    assert_eq!(binding.project_id, "p");
    assert_eq!(binding.task_revision_id, "rev");
    assert!(
        sqlx::query("UPDATE session_bindings SET project_id='other' WHERE session_id='s'")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM session_bindings WHERE session_id='s'")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE sessions SET binding_mode='local_unbound' WHERE id='s'")
            .execute(&db.pool)
            .await
            .is_err()
    );
}

mod support;

use cairn_domain::SessionState;
use cairn_storage_local::events::{append_event, NewEvent};
use cairn_storage_local::{
    repos, sessions, snapshots, worktrees, writer, RepositoryRow, SessionRow, SnapshotRow,
    WorktreeRow, WriterPolicy,
};
use support::TestDatabase;

#[tokio::test]
async fn feature001_daos_read_and_write_on_migrated_schema() {
    let db = TestDatabase::from_feature001_fixture().await;
    assert_eq!(repos::count(&db.pool).await.unwrap(), 4);
    let existing_sessions = sessions::list(&db.pool, None, None).await.unwrap();
    assert_eq!(existing_sessions.len(), 4);
    assert!(existing_sessions
        .iter()
        .all(|session| session.binding_mode == "local_unbound"));
    assert_eq!(
        sessions::count_by_state(&db.pool, SessionState::Active)
            .await
            .unwrap(),
        1
    );
    let legacy_events =
        cairn_storage_local::events::list_events(&db.pool, None, None, None, None, 100)
            .await
            .unwrap();
    assert_eq!(legacy_events.len(), 18);
    assert!(legacy_events
        .iter()
        .all(|event| event.aggregate_type.is_none()));

    writer::begin_immediate(
        &db.pool,
        WriterPolicy::default(),
        None,
        Box::new(|conn| {
            Box::pin(async move {
                repos::insert(
                    &mut *conn,
                    &RepositoryRow {
                        id: "compat-repo".into(),
                        repo_uuid: "compat-repo-uuid".into(),
                        canonical_path: "/tmp/compat".into(),
                        default_remote_name: None,
                        default_remote_url: None,
                        copied_from_repository_id: None,
                        registered_at: "t".into(),
                    },
                )
                .await?;
                worktrees::insert(
                    &mut *conn,
                    &WorktreeRow {
                        id: "compat-worktree".into(),
                        repository_id: "compat-repo".into(),
                        worktree_uuid: "compat-worktree-uuid".into(),
                        path: "/tmp/compat".into(),
                        is_main: 1,
                        registered_at: "t".into(),
                    },
                )
                .await?;
                snapshots::insert(
                    &mut *conn,
                    &SnapshotRow {
                        id: "compat-snapshot".into(),
                        worktree_id: "compat-worktree".into(),
                        branch: Some("main".into()),
                        head_commit: "head".into(),
                        staged_fp: "a".into(),
                        unstaged_fp: "b".into(),
                        untracked_fp: "c".into(),
                        snapshot_fp: "snapshot".into(),
                        fp_schema_version: 1,
                        created_at: "t".into(),
                    },
                )
                .await?;
                sessions::insert(
                    &mut *conn,
                    &SessionRow {
                        id: "compat-session".into(),
                        repository_id: "compat-repo".into(),
                        worktree_id: "compat-worktree".into(),
                        local_user: "user".into(),
                        agent_type: "agent".into(),
                        agent_instance_id: "instance".into(),
                        agent_pid: None,
                        resume_token_hash: "hash-only".into(),
                        lease_expires_at: "t".into(),
                        state: "active".into(),
                        start_snapshot_id: "compat-snapshot".into(),
                        current_snapshot_id: "compat-snapshot".into(),
                        started_at: "t".into(),
                        ended_at: None,
                        last_heartbeat_at: "t".into(),
                        recovering_since: None,
                        binding_mode: "local_unbound".into(),
                    },
                )
                .await?;
                append_event(
                    conn,
                    &NewEvent {
                        id: uuid::Uuid::now_v7().to_string(),
                        idempotency_key: "compat-event".into(),
                        event_type: "session.started".into(),
                        repository_id: Some("compat-repo".into()),
                        worktree_id: Some("compat-worktree".into()),
                        session_id: Some("compat-session".into()),
                        snapshot_id: Some("compat-snapshot".into()),
                        aggregate_type: "session".into(),
                        aggregate_id: "compat-session".into(),
                        payload: serde_json::json!({"schema_version": 1}),
                        recorded_at: "t".into(),
                    },
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    assert_eq!(repos::count(&db.pool).await.unwrap(), 5);
    assert_eq!(
        worktrees::get_by_uuid(&db.pool, "compat-worktree-uuid")
            .await
            .unwrap()
            .unwrap()
            .repository_id,
        "compat-repo"
    );
    assert_eq!(
        snapshots::get_by_id(&db.pool, "compat-snapshot")
            .await
            .unwrap()
            .unwrap()
            .snapshot_fp,
        "snapshot"
    );
    let session = sessions::get_by_id(&db.pool, "compat-session")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.state, "active");
    assert_eq!(session.binding_mode, "local_unbound");
    let new_events =
        cairn_storage_local::events::list_events(&db.pool, None, None, None, Some(18), 10)
            .await
            .unwrap();
    assert_eq!(new_events.len(), 1);
    assert_eq!(new_events[0].aggregate_type.as_deref(), Some("session"));
}

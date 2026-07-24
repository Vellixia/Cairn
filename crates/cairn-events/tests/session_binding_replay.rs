use cairn_domain::{
    EventId, GoalContractV1, Project, ProjectId, ProjectRepositoryAssociationId, SessionId,
    SessionState, SnapshotId, Task, TaskId, TaskRevision, TaskRevisionId, Timestamp,
};
use cairn_events::aggregate::EventOperationMethod;
use cairn_events::replay::{live_session_binding_projections, rebuild_session_binding_projections};
use cairn_events::{
    EventBuilder, ProjectAssociationEvent, ProjectCreatedPayload,
    ProjectRepositoryAssociatedPayload, SessionBindingEvent, SessionBoundPayload,
    SessionStartedPayload, TaskCreatedPayload, TaskRevisionCreatedPayload,
};
use cairn_storage_local::records::{
    ProjectRepositoryAssociationRow, ProjectRow, RepositoryRow, SessionBindingRow, SessionRow,
    SnapshotRow, TaskRevisionRow, TaskRow, WorktreeRow,
};
use cairn_storage_local::writer::{begin_immediate, WriterPolicy};
use cairn_storage_local::{
    events, open_pool_at, projects, repos, session_bindings, sessions, snapshots, tasks, worktrees,
};
use schemars::schema_for;

fn contract() -> GoalContractV1 {
    GoalContractV1::new(
        "Bind the session".into(),
        vec!["binding".into()],
        vec![],
        vec!["persists".into()],
        vec!["append-only".into()],
    )
    .unwrap()
}

#[tokio::test]
async fn session_bound_is_typed_and_replays_field_for_field_as_the_sole_bound_transition() {
    let rendered = serde_json::to_string(&schema_for!(SessionBoundPayload)).unwrap();
    for field in [
        "session_id",
        "repository_id",
        "worktree_id",
        "project_id",
        "task_id",
        "task_revision_id",
        "bound_at",
    ] {
        assert!(rendered.contains(field));
    }

    let temp = tempfile::tempdir().unwrap();
    let pool = open_pool_at(&temp.path().join("binding-replay.db"))
        .await
        .unwrap();
    let now = Timestamp::now();
    let repository_id = EventId::new_v7().to_string();
    let worktree_id = EventId::new_v7().to_string();
    let snapshot_id = SnapshotId::new_v7().to_string();
    let session_id = SessionId::new_v7();
    let project = Project::new(ProjectId::new_v7(), "Replay project", None, now).unwrap();
    let association_id = ProjectRepositoryAssociationId::new_v7();
    let task = Task::new(TaskId::new_v7(), project.id, "Replay task", now).unwrap();
    let revision =
        TaskRevision::new(TaskRevisionId::new_v7(), task.id, 1, None, contract(), now).unwrap();

    repos::insert(
        &pool,
        &RepositoryRow {
            id: repository_id.clone(),
            repo_uuid: EventId::new_v7().to_string(),
            canonical_path: "/replay/repo".into(),
            default_remote_name: None,
            default_remote_url: None,
            copied_from_repository_id: None,
            registered_at: now.to_rfc3339(),
        },
    )
    .await
    .unwrap();
    worktrees::insert(
        &pool,
        &WorktreeRow {
            id: worktree_id.clone(),
            repository_id: repository_id.clone(),
            worktree_uuid: EventId::new_v7().to_string(),
            path: "/replay/repo".into(),
            is_main: 1,
            registered_at: now.to_rfc3339(),
        },
    )
    .await
    .unwrap();
    snapshots::insert(
        &pool,
        &SnapshotRow {
            id: snapshot_id.clone(),
            worktree_id: worktree_id.clone(),
            branch: Some("main".into()),
            head_commit: "0".repeat(40),
            staged_fp: "1".repeat(64),
            unstaged_fp: "2".repeat(64),
            untracked_fp: "3".repeat(64),
            snapshot_fp: "4".repeat(64),
            fp_schema_version: 1,
            created_at: now.to_rfc3339(),
        },
    )
    .await
    .unwrap();
    sessions::insert(
        &pool,
        &SessionRow {
            id: session_id.to_string(),
            repository_id: repository_id.clone(),
            worktree_id: worktree_id.clone(),
            local_user: "tester".into(),
            agent_type: "test".into(),
            agent_instance_id: EventId::new_v7().to_string(),
            agent_pid: None,
            resume_token_hash: "5".repeat(64),
            lease_expires_at: now.plus_seconds(900).to_rfc3339(),
            state: SessionState::Stopped.as_str().into(),
            start_snapshot_id: snapshot_id.clone(),
            current_snapshot_id: snapshot_id.clone(),
            started_at: now.to_rfc3339(),
            ended_at: Some(now.to_rfc3339()),
            last_heartbeat_at: now.to_rfc3339(),
            recovering_since: None,
            binding_mode: "local_unbound".into(),
        },
    )
    .await
    .unwrap();

    let project_event = EventBuilder::project_created(
        EventId::new_v7(),
        "project-create",
        &ProjectCreatedPayload {
            schema_version: 1,
            project: project.clone().into(),
        },
    );
    let association_event = EventBuilder::project_repository_associated(
        EventId::new_v7(),
        "association-create",
        &ProjectRepositoryAssociatedPayload {
            schema_version: 1,
            association: ProjectAssociationEvent {
                association_id,
                project_id: project.id,
                repository_id: repository_id.clone(),
                associated_at: now,
            },
        },
    );
    let task_event = EventBuilder::task_created(
        EventId::new_v7(),
        "task-create",
        &TaskCreatedPayload {
            schema_version: 1,
            task: task.clone().into(),
        },
    );
    let revision_event = EventBuilder::task_revision_created(
        EventId::new_v7(),
        "task-create",
        EventOperationMethod::TaskCreate,
        1,
        &TaskRevisionCreatedPayload {
            schema_version: 1,
            revision: revision.clone().into(),
            task: task.clone().into(),
        },
    );
    let started_event = EventBuilder::session_started(
        &repository_id,
        &worktree_id,
        &session_id.to_string(),
        &SessionStartedPayload {
            agent_type: "test".into(),
            agent_instance_id: EventId::new_v7().to_string(),
            start_snapshot_id: snapshot_id,
            local_user: "tester".into(),
        },
    );
    let bound_event = EventBuilder::session_bound_for_start(
        EventId::new_v7(),
        &session_id.to_string(),
        &SessionBoundPayload {
            schema_version: 1,
            binding: SessionBindingEvent {
                session_id,
                repository_id: repository_id.clone(),
                worktree_id: worktree_id.clone(),
                project_id: project.id,
                task_id: task.id,
                task_revision_id: revision.id,
                bound_at: now,
            },
        },
    );
    let project_row = ProjectRow {
        id: project.id.to_string(),
        name: project.name.clone(),
        description: None,
        status: "active".into(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let task_row = TaskRow {
        id: task.id.to_string(),
        project_id: project.id.to_string(),
        title: task.title.clone(),
        latest_revision_number: 1,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let revision_row = TaskRevisionRow {
        id: revision.id.to_string(),
        task_id: task.id.to_string(),
        revision_number: 1,
        parent_revision_id: None,
        goal_contract_json: String::from_utf8(revision.goal_contract.canonical_bytes()).unwrap(),
        goal_contract_schema_version: 1,
        goal_contract_fingerprint: revision.goal_contract_fingerprint.clone(),
        created_at: now.to_rfc3339(),
    };
    begin_immediate(
        &pool,
        WriterPolicy::default(),
        None,
        Box::new(move |conn| {
            Box::pin(async move {
                let project_seq = events::append_event(conn, &project_event).await?.seq;
                projects::insert(conn, &project_row).await?;
                let association_seq = events::append_event(conn, &association_event).await?.seq;
                projects::insert_association(
                    conn,
                    &ProjectRepositoryAssociationRow {
                        id: association_id.to_string(),
                        project_id: project.id.to_string(),
                        repository_id: repository_id.clone(),
                        associated_at: now.to_rfc3339(),
                        event_seq: association_seq,
                    },
                )
                .await?;
                assert!(project_seq < association_seq);
                events::append_event(conn, &task_event).await?;
                events::append_event(conn, &revision_event).await?;
                tasks::insert_task(conn, &task_row, &revision_row).await?;
                let started_seq = events::append_event(conn, &started_event).await?.seq;
                let binding_seq = events::append_event(conn, &bound_event).await?.seq;
                assert_eq!(binding_seq, started_seq + 1);
                session_bindings::insert(
                    conn,
                    &SessionBindingRow {
                        session_id: session_id.to_string(),
                        project_id: project.id.to_string(),
                        task_revision_id: revision.id.to_string(),
                        bound_at: now.to_rfc3339(),
                        binding_event_seq: binding_seq,
                    },
                )
                .await?;
                Ok(())
            })
        }),
    )
    .await
    .unwrap();

    let rebuilt = rebuild_session_binding_projections(&pool).await.unwrap();
    let live = live_session_binding_projections(&pool).await.unwrap();
    assert_eq!(rebuilt, live);
    assert_eq!(rebuilt.bindings.len(), 1);
    assert!(matches!(
        rebuilt.scopes[&session_id],
        cairn_domain::SessionBindingMode::ProjectBound { .. }
    ));
}

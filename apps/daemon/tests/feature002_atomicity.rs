use std::sync::Arc;
use std::time::Duration;

use cairn_domain::{
    AgentInstanceId, GoalContractV1, IdempotencyKey, ProjectStatus, SessionBindingMode, SessionId,
    Timestamp,
};
use cairn_project::{
    AssociateRepository, CreateProject, CreateTask, ProjectService, ReviseTask, TaskService,
    UpdateProject,
};
use cairn_session::{BindSession, SessionConfig, SessionService};
use cairn_storage_local::records::{RepositoryRow, SessionRow, SnapshotRow, WorktreeRow};
use cairn_storage_local::writer::{WorktreeWriters, WriteCheckpoint, WriteTestHooks, WriterPolicy};
use sqlx::{Connection, SqlitePool};

struct Fixture {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    repository_id: String,
    worktree_id: String,
    snapshot: SnapshotRow,
}

async fn new_fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let pool = cairn_storage_local::open_pool_at(&dir.path().join("cairn.db"))
        .await
        .unwrap();
    let repository_id = uuid::Uuid::now_v7().to_string();
    let worktree_id = uuid::Uuid::now_v7().to_string();
    let now = Timestamp::now().to_rfc3339();
    cairn_storage_local::repos::insert(
        &pool,
        &RepositoryRow {
            id: repository_id.clone(),
            repo_uuid: uuid::Uuid::new_v4().to_string(),
            canonical_path: "/atomic/repository".into(),
            default_remote_name: None,
            default_remote_url: None,
            copied_from_repository_id: None,
            registered_at: now.clone(),
        },
    )
    .await
    .unwrap();
    cairn_storage_local::worktrees::insert(
        &pool,
        &WorktreeRow {
            id: worktree_id.clone(),
            repository_id: repository_id.clone(),
            worktree_uuid: uuid::Uuid::new_v4().to_string(),
            path: "/atomic/repository".into(),
            is_main: 1,
            registered_at: now.clone(),
        },
    )
    .await
    .unwrap();
    let snapshot = SnapshotRow {
        id: uuid::Uuid::now_v7().to_string(),
        worktree_id: worktree_id.clone(),
        branch: Some("main".into()),
        head_commit: "atomic-head".into(),
        staged_fp: "atomic-staged".into(),
        unstaged_fp: "atomic-unstaged".into(),
        untracked_fp: "atomic-untracked".into(),
        snapshot_fp: uuid::Uuid::new_v4().to_string(),
        fp_schema_version: 1,
        created_at: now,
    };
    cairn_storage_local::snapshots::insert(&pool, &snapshot)
        .await
        .unwrap();
    Fixture {
        _dir: dir,
        pool,
        repository_id,
        worktree_id,
        snapshot,
    }
}

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(goal.into(), vec![], vec![], vec!["accepted".into()], vec![]).unwrap()
}

async fn digest(pool: &SqlitePool) -> Vec<String> {
    let queries = [
        "SELECT COALESCE(group_concat(id || ':' || name || ':' || status, '|'),'') FROM (SELECT * FROM projects ORDER BY id)",
        "SELECT COALESCE(group_concat(id || ':' || project_id || ':' || repository_id, '|'),'') FROM (SELECT * FROM project_repository_associations ORDER BY id)",
        "SELECT COALESCE(group_concat(id || ':' || latest_revision_number, '|'),'') FROM (SELECT * FROM tasks ORDER BY id)",
        "SELECT COALESCE(group_concat(id || ':' || task_id || ':' || revision_number, '|'),'') FROM (SELECT * FROM task_revisions ORDER BY id)",
        "SELECT COALESCE(group_concat(id || ':' || state || ':' || binding_mode, '|'),'') FROM (SELECT * FROM sessions ORDER BY id)",
        "SELECT COALESCE(group_concat(session_id || ':' || project_id || ':' || task_revision_id, '|'),'') FROM (SELECT * FROM session_bindings ORDER BY session_id)",
        "SELECT COALESCE(group_concat(seq || ':' || id || ':' || event_type, '|'),'') FROM (SELECT * FROM events ORDER BY seq)",
        "SELECT COALESCE(group_concat(aggregate_type || ':' || aggregate_id || ':' || last_seq, '|'),'') FROM (SELECT * FROM event_aggregate_heads ORDER BY aggregate_type, aggregate_id)",
        "SELECT COALESCE(group_concat(idempotency_key || ':' || method || ':' || result_locator, '|'),'') FROM (SELECT * FROM operation_idempotency ORDER BY idempotency_key)",
    ];
    let mut result = Vec::new();
    for query in queries {
        result.push(
            sqlx::query_scalar::<_, String>(query)
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    result
}

async fn project_and_task(
    fixture: &Fixture,
) -> (cairn_domain::Project, cairn_project::CreateTaskResult) {
    let projects = ProjectService::new(fixture.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "Atomic scope".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    projects
        .associate_repository(AssociateRepository {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            repository_id: fixture.repository_id.clone(),
        })
        .await
        .unwrap();
    let task = TaskService::new(fixture.pool.clone())
        .create(CreateTask {
            idempotency_key: IdempotencyKey::new_v7(),
            project_id: project.id,
            title: "Atomic task".into(),
            goal_contract: contract("revision one"),
        })
        .await
        .unwrap();
    (project, task)
}

async fn insert_unbound_session(fixture: &Fixture) -> SessionId {
    let id = SessionId::new_v7();
    let now = Timestamp::now().to_rfc3339();
    cairn_storage_local::sessions::insert(
        &fixture.pool,
        &SessionRow {
            id: id.to_string(),
            repository_id: fixture.repository_id.clone(),
            worktree_id: fixture.worktree_id.clone(),
            local_user: "atomic".into(),
            agent_type: "atomic".into(),
            agent_instance_id: AgentInstanceId(uuid::Uuid::new_v4()).to_string(),
            agent_pid: None,
            resume_token_hash: "atomic-hash".into(),
            lease_expires_at: now.clone(),
            state: "active".into(),
            start_snapshot_id: fixture.snapshot.id.clone(),
            current_snapshot_id: fixture.snapshot.id.clone(),
            started_at: now.clone(),
            ended_at: None,
            last_heartbeat_at: now,
            recovering_since: None,
            binding_mode: "local_unbound".into(),
        },
    )
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn every_project_create_boundary_rolls_back_then_reuses_gap_free_sequences() {
    let points = [
        WriteCheckpoint::PreRegistryReservation,
        WriteCheckpoint::PostRegistryReservation,
        WriteCheckpoint::PreEvent,
        WriteCheckpoint::PostEvent,
        WriteCheckpoint::PreProjection,
        WriteCheckpoint::PostProjection,
        WriteCheckpoint::PreResultLocator,
        WriteCheckpoint::PreCommit,
    ];
    for point in points {
        let fixture = new_fixture().await;
        let before = digest(&fixture.pool).await;
        let hooks = WriteTestHooks::default();
        hooks.fail_at(point);
        let request = CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: format!("rollback-{point:?}"),
            description: None,
        };
        assert!(ProjectService::with_test_controls(
            fixture.pool.clone(),
            WriterPolicy::default(),
            hooks,
        )
        .create(request.clone())
        .await
        .is_err());
        assert_eq!(digest(&fixture.pool).await, before);
        ProjectService::new(fixture.pool.clone())
            .create(request)
            .await
            .unwrap();
        let (event_seq, aggregate_seq): (i64, i64) = sqlx::query_as(
            "SELECT seq, aggregate_seq FROM events WHERE event_type='project.created'",
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap();
        assert_eq!((event_seq, aggregate_seq), (1, 1));
    }
}

#[tokio::test]
async fn all_feature002_mutation_shapes_are_atomic_at_distinct_boundaries() {
    // Project update after event append.
    let fixture = new_fixture().await;
    let projects = ProjectService::new(fixture.pool.clone());
    let project = projects
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "update".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::PostEvent);
    let update = UpdateProject {
        idempotency_key: IdempotencyKey::new_v7(),
        project_id: project.id,
        name: Some("changed".into()),
        description: None,
        clear_description: false,
        status: None,
    };
    assert!(ProjectService::with_test_controls(
        fixture.pool.clone(),
        WriterPolicy::default(),
        hooks,
    )
    .update(update.clone())
    .await
    .is_err());
    assert_eq!(digest(&fixture.pool).await, before);
    ProjectService::new(fixture.pool.clone())
        .update(update)
        .await
        .unwrap();

    // Association after projection.
    let fixture = new_fixture().await;
    let project = ProjectService::new(fixture.pool.clone())
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "association".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::PostProjection);
    let association = AssociateRepository {
        idempotency_key: IdempotencyKey::new_v7(),
        project_id: project.id,
        repository_id: fixture.repository_id.clone(),
    };
    assert!(ProjectService::with_test_controls(
        fixture.pool.clone(),
        WriterPolicy::default(),
        hooks,
    )
    .associate_repository(association.clone())
    .await
    .is_err());
    assert_eq!(digest(&fixture.pool).await, before);
    ProjectService::new(fixture.pool.clone())
        .associate_repository(association)
        .await
        .unwrap();

    // Task creation between its two events.
    let fixture = new_fixture().await;
    let project = ProjectService::new(fixture.pool.clone())
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "task create".into(),
            description: None,
        })
        .await
        .unwrap()
        .project;
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::BetweenEvents);
    let create_task = CreateTask {
        idempotency_key: IdempotencyKey::new_v7(),
        project_id: project.id,
        title: "created atomically".into(),
        goal_contract: contract("revision one"),
    };
    assert!(
        TaskService::with_test_controls(fixture.pool.clone(), WriterPolicy::default(), hooks,)
            .create(create_task.clone())
            .await
            .is_err()
    );
    assert_eq!(digest(&fixture.pool).await, before);
    let created = TaskService::new(fixture.pool.clone())
        .create(create_task)
        .await
        .unwrap();
    assert_eq!(created.revision.revision_number, 1);

    // Revision allocation after the counter advancement leaves no gap.
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::PostCounterAllocation);
    let revise = ReviseTask {
        idempotency_key: IdempotencyKey::new_v7(),
        task_id: created.task.id,
        parent_revision_id: None,
        goal_contract: contract("revision two"),
    };
    assert!(
        TaskService::with_test_controls(fixture.pool.clone(), WriterPolicy::default(), hooks,)
            .revise(revise.clone())
            .await
            .is_err()
    );
    assert_eq!(digest(&fixture.pool).await, before);
    assert_eq!(
        TaskService::new(fixture.pool.clone())
            .revise(revise)
            .await
            .unwrap()
            .revision
            .revision_number,
        2
    );

    // Existing-session binding before result resolution.
    let fixture = new_fixture().await;
    let (project, task) = project_and_task(&fixture).await;
    let session_id = insert_unbound_session(&fixture).await;
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::PreResultLocator);
    let bind = BindSession {
        idempotency_key: IdempotencyKey::new_v7(),
        session_id,
        project_id: project.id,
        task_revision_id: task.revision.id,
    };
    assert!(SessionService::with_binding_test_controls(
        fixture.pool.clone(),
        Arc::new(WorktreeWriters::new()),
        SessionConfig::from_env(),
        WriterPolicy::default(),
        hooks,
    )
    .bind(bind.clone())
    .await
    .is_err());
    assert_eq!(digest(&fixture.pool).await, before);
    SessionService::new(
        fixture.pool.clone(),
        Arc::new(WorktreeWriters::new()),
        SessionConfig::from_env(),
    )
    .bind(bind)
    .await
    .unwrap();

    // Bound start between session.started and session.bound.
    let fixture = new_fixture().await;
    let (project, task) = project_and_task(&fixture).await;
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.fail_at(WriteCheckpoint::BetweenEvents);
    let service = SessionService::with_test_controls(
        fixture.pool.clone(),
        Arc::new(WorktreeWriters::new()),
        SessionConfig::from_env(),
        WriterPolicy::default(),
        hooks,
    );
    let scope = SessionBindingMode::ProjectBound {
        project_id: project.id,
        task_revision_id: task.revision.id,
    };
    assert!(service
        .start(
            &fixture.repository_id,
            &fixture.worktree_id,
            "atomic",
            "bound-start",
            &AgentInstanceId(uuid::Uuid::new_v4()).to_string(),
            None,
            &fixture.snapshot,
            scope,
        )
        .await
        .is_err());
    assert_eq!(digest(&fixture.pool).await, before);
    let started = SessionService::new(
        fixture.pool.clone(),
        Arc::new(WorktreeWriters::new()),
        SessionConfig::from_env(),
    )
    .start(
        &fixture.repository_id,
        &fixture.worktree_id,
        "atomic",
        "bound-start",
        &AgentInstanceId(uuid::Uuid::new_v4()).to_string(),
        None,
        &fixture.snapshot,
        scope,
    )
    .await
    .unwrap();
    assert_eq!(started.session.binding_mode, "project_bound");
}

#[tokio::test]
async fn cancellation_and_independent_connection_busy_exhaustion_leave_no_owner() {
    let fixture = new_fixture().await;
    let before = digest(&fixture.pool).await;
    let hooks = WriteTestHooks::default();
    hooks.pause_at(WriteCheckpoint::PostRegistryReservation);
    let task_hooks = hooks.clone();
    let pool = fixture.pool.clone();
    let request = CreateProject {
        idempotency_key: IdempotencyKey::new_v7(),
        name: "cancelled".into(),
        description: None,
    };
    let task = tokio::spawn(async move {
        ProjectService::with_test_controls(pool, WriterPolicy::default(), task_hooks)
            .create(request)
            .await
    });
    hooks
        .wait_until_reached(WriteCheckpoint::PostRegistryReservation)
        .await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(digest(&fixture.pool).await, before);

    let mut holder = fixture.pool.acquire().await.unwrap();
    let transaction = holder.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let hooks = WriteTestHooks::default();
    let error = ProjectService::with_test_controls(
        fixture.pool.clone(),
        WriterPolicy::test_with_busy_timeout(Duration::from_millis(25)),
        hooks,
    )
    .create(CreateProject {
        idempotency_key: IdempotencyKey::new_v7(),
        name: "busy".into(),
        description: None,
    })
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        cairn_project::ProjectTaskError::StorageBusy { max_elapsed_ms: 25 }
    ));
    transaction.rollback().await.unwrap();
    assert_eq!(digest(&fixture.pool).await, before);

    ProjectService::new(fixture.pool.clone())
        .create(CreateProject {
            idempotency_key: IdempotencyKey::new_v7(),
            name: "after contention".into(),
            description: None,
        })
        .await
        .unwrap();
}

#[test]
fn lifecycle_and_binding_are_separate_dimensions() {
    assert_eq!(ProjectStatus::Active.as_str(), "active");
    assert_ne!(format!("{:?}", SessionBindingMode::LocalUnbound), "active");
}

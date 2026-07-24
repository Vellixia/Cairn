mod support;

use cairn_domain::{AgentInstanceId, GoalContractV1, IdempotencyKey};
use cairn_protocol::*;
use cairn_storage_local::events;
use fixtures_repositories::FixtureRepo;
use support::TestDaemon;

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(
        goal.into(),
        vec!["quickstart".into()],
        vec!["no synchronization".into()],
        vec!["replay equals live".into()],
        vec!["append only".into()],
    )
    .unwrap()
}

fn stable_event_rows(rows: &[events::EventRow]) -> Vec<(i64, String, String, String)> {
    rows.iter()
        .map(|event| {
            (
                event.seq,
                event.id.clone(),
                event.event_type.clone(),
                event.payload.clone(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn authoritative_quickstart_preserves_bootstrap_history_and_replays_exactly() {
    let daemon = TestDaemon::start().await;
    let repository = FixtureRepo::new().unwrap();
    let registered: RegisterResult = serde_json::from_value(
        daemon
            .call(
                methods::REPOSITORY_REGISTER,
                &RegisterParams {
                    path: repository.root().to_string_lossy().to_string(),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let repository_id = registered.repository.repository_id;

    let bootstrap_agent = AgentInstanceId(uuid::Uuid::now_v7());
    let bootstrap: SessionStartResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_START,
                &SessionStartParams {
                    path: Some(repository.root().to_string_lossy().to_string()),
                    repository_id: None,
                    agent_type: "quickstart-bootstrap".into(),
                    agent_instance_id: bootstrap_agent,
                    agent_pid: None,
                    scope: Some(SessionScopeDto::LocalUnbound),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(bootstrap.session.scope, SessionScopeDto::LocalUnbound);
    let original_session_id = bootstrap.session.session_id;
    let original_lifecycle = bootstrap.session.state;
    let pool = cairn_storage_local::open_pool_at(&daemon.db_path())
        .await
        .unwrap();
    let original_events = events::list_events(&pool, None, None, None, None, 1_000)
        .await
        .unwrap();
    let original_hash_material = stable_event_rows(&original_events);

    let project: ProjectCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::PROJECT_CREATE,
                &ProjectCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    name: "Quickstart project".into(),
                    description: Some("local binding acceptance".into()),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let _: ProjectRepositoryAssociateResult = serde_json::from_value(
        daemon
            .call(
                methods::PROJECT_REPOSITORY_ASSOCIATE,
                &ProjectRepositoryAssociateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    project_id: project.project.project_id,
                    repository_id: repository_id.clone(),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let task: TaskCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_CREATE,
                &TaskCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    project_id: project.project.project_id,
                    title: "Quickstart task".into(),
                    goal_contract: contract("revision one"),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let bound: SessionBindResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_BIND,
                &SessionBindParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    session_id: original_session_id,
                    project_id: project.project.project_id,
                    task_revision_id: task.revision.revision_id,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(bound.session_id, original_session_id);

    let shown: SessionGetResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_GET,
                &SessionGetParams {
                    path: None,
                    repository_id: None,
                    session_id: Some(original_session_id),
                    agent_instance_id: None,
                    agent_type: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let shown = shown.session.unwrap();
    assert_eq!(shown.session_id, original_session_id);
    assert_eq!(shown.state, original_lifecycle);
    assert_eq!(
        shown.scope,
        SessionScopeDto::ProjectBound {
            project_id: project.project.project_id,
            task_revision_id: task.revision.revision_id,
        }
    );
    let after_bind = events::list_events(&pool, None, None, None, None, 1_000)
        .await
        .unwrap();
    assert_eq!(
        stable_event_rows(&after_bind[..original_events.len()]),
        original_hash_material,
        "binding must not rewrite any Feature 001 event"
    );
    pool.close().await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let after_restart: SessionGetResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_GET,
                &SessionGetParams {
                    path: None,
                    repository_id: None,
                    session_id: Some(original_session_id),
                    agent_instance_id: None,
                    agent_type: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        after_restart.session.unwrap().scope,
        SessionScopeDto::ProjectBound {
            project_id: project.project.project_id,
            task_revision_id: task.revision.revision_id,
        }
    );

    let revision_two: TaskReviseResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_REVISE,
                &TaskReviseParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    task_id: task.task.task_id,
                    parent_revision_id: None,
                    goal_contract: contract("revision two"),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(revision_two.revision.revision_number, 2);
    let still_revision_one: SessionGetResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_GET,
                &SessionGetParams {
                    path: None,
                    repository_id: None,
                    session_id: Some(original_session_id),
                    agent_instance_id: None,
                    agent_type: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        still_revision_one.session.unwrap().scope,
        SessionScopeDto::ProjectBound {
            project_id: project.project.project_id,
            task_revision_id: task.revision.revision_id,
        }
    );

    let newly_bound: SessionStartResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_START,
                &SessionStartParams {
                    path: Some(repository.root().to_string_lossy().to_string()),
                    repository_id: None,
                    agent_type: "quickstart-bound".into(),
                    agent_instance_id: AgentInstanceId(uuid::Uuid::now_v7()),
                    agent_pid: None,
                    scope: Some(SessionScopeDto::ProjectBound {
                        project_id: project.project.project_id,
                        task_revision_id: revision_two.revision.revision_id,
                    }),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        newly_bound.scope(),
        SessionScopeDto::ProjectBound { .. }
    ));

    let required = daemon
        .call(
            methods::SESSION_START,
            &SessionStartParams {
                path: Some(repository.root().to_string_lossy().to_string()),
                repository_id: None,
                agent_type: "quickstart-invalid-unbound".into(),
                agent_instance_id: AgentInstanceId(uuid::Uuid::now_v7()),
                agent_pid: None,
                scope: Some(SessionScopeDto::LocalUnbound),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(required.code, ErrorCode::ProjectScopeRequired);

    let pool = cairn_storage_local::open_pool_at(&daemon.db_path())
        .await
        .unwrap();
    cairn_events::replay::verify_mixed_projections(&pool)
        .await
        .unwrap();
    let all_events = events::list_events(&pool, None, None, None, None, 1_000)
        .await
        .unwrap();
    let count = |event_type: &str| {
        all_events
            .iter()
            .filter(|event| event.event_type == event_type)
            .count()
    };
    assert_eq!(count("project.created"), 1);
    assert_eq!(count("project.repository_associated"), 1);
    assert_eq!(count("project.updated"), 0);
    assert_eq!(count("task.created"), 1);
    assert_eq!(count("task.revision_created"), 2);
    assert_eq!(count("session.started"), 2);
    assert_eq!(count("session.bound"), 2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM tasks")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_revisions")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM session_bindings")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    println!(
        "quickstart_counts={{\"project_events\":2,\"task_events\":3,\"session_started\":2,\"session_bound\":2,\"projects\":1,\"tasks\":1,\"revisions\":2,\"bindings\":2}}"
    );
    pool.close().await;
    drop(repository);
    daemon.stop().await;
}

trait SessionStartScope {
    fn scope(&self) -> &SessionScopeDto;
}

impl SessionStartScope for SessionStartResult {
    fn scope(&self) -> &SessionScopeDto {
        &self.session.scope
    }
}

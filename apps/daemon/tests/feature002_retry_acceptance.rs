mod support;

use cairn_domain::{AgentInstanceId, GoalContractV1, IdempotencyKey};
use cairn_protocol::*;
use fixtures_repositories::FixtureRepo;
use support::TestDaemon;

const RETRIES: usize = 100;

fn contract(goal: &str) -> GoalContractV1 {
    GoalContractV1::new(goal.into(), vec![], vec![], vec!["accepted".into()], vec![]).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn exactly_one_hundred_association_revision_and_binding_retries_remain_one_result() {
    assert_eq!(RETRIES, 100, "acceptance count is normative");
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
    let started: SessionStartResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_START,
                &SessionStartParams {
                    path: Some(repository.root().to_string_lossy().to_string()),
                    repository_id: None,
                    agent_type: "retry-acceptance".into(),
                    agent_instance_id: AgentInstanceId(uuid::Uuid::now_v7()),
                    agent_pid: None,
                    scope: Some(SessionScopeDto::LocalUnbound),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let project: ProjectCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::PROJECT_CREATE,
                &ProjectCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    name: "Retry project".into(),
                    description: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();

    let association_key = IdempotencyKey::new_v7();
    let association_params = ProjectRepositoryAssociateParams {
        idempotency_key: association_key,
        project_id: project.project.project_id,
        repository_id: registered.repository.repository_id.clone(),
    };
    let mut association_result = None;
    let mut association_calls = 0usize;
    for _ in 0..RETRIES {
        let result: ProjectRepositoryAssociateResult = serde_json::from_value(
            daemon
                .call(methods::PROJECT_REPOSITORY_ASSOCIATE, &association_params)
                .await
                .unwrap(),
        )
        .unwrap();
        if let Some(original) = association_result.as_ref() {
            assert_eq!(&result, original);
        } else {
            association_result = Some(result);
        }
        association_calls += 1;
    }
    assert_eq!(
        association_calls, 100,
        "exactly 100 identical association retries must execute"
    );

    let task: TaskCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_CREATE,
                &TaskCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    project_id: project.project.project_id,
                    title: "Retry task".into(),
                    goal_contract: contract("revision one"),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let revision_key = IdempotencyKey::new_v7();
    let revision_params = TaskReviseParams {
        idempotency_key: revision_key,
        task_id: task.task.task_id,
        parent_revision_id: None,
        goal_contract: contract("revision two"),
    };
    let mut revision_result = None;
    let mut revision_calls = 0usize;
    for _ in 0..RETRIES {
        let result: TaskReviseResult = serde_json::from_value(
            daemon
                .call(methods::TASK_REVISE, &revision_params)
                .await
                .unwrap(),
        )
        .unwrap();
        if let Some(original) = revision_result.as_ref() {
            assert_eq!(&result, original);
        } else {
            revision_result = Some(result);
        }
        revision_calls += 1;
    }
    assert_eq!(
        revision_calls, 100,
        "exactly 100 identical revision retries must execute"
    );
    let revision_two = revision_result.as_ref().unwrap().revision.revision_id;

    let binding_key = IdempotencyKey::new_v7();
    let binding_params = SessionBindParams {
        idempotency_key: binding_key,
        session_id: started.session.session_id,
        project_id: project.project.project_id,
        task_revision_id: task.revision.revision_id,
    };
    let mut binding_result = None;
    let mut binding_calls = 0usize;
    for _ in 0..RETRIES {
        let result: SessionBindResult = serde_json::from_value(
            daemon
                .call(methods::SESSION_BIND, &binding_params)
                .await
                .unwrap(),
        )
        .unwrap();
        if let Some(original) = binding_result.as_ref() {
            assert_eq!(&result, original);
        } else {
            binding_result = Some(result);
        }
        binding_calls += 1;
    }
    assert_eq!(
        binding_calls, 100,
        "exactly 100 identical binding retries must execute"
    );

    let other_project: ProjectCreateResult = serde_json::from_value(
        daemon
            .call(
                methods::PROJECT_CREATE,
                &ProjectCreateParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    name: "Other retry project".into(),
                    description: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    let association_conflict = daemon
        .call(
            methods::PROJECT_REPOSITORY_ASSOCIATE,
            &ProjectRepositoryAssociateParams {
                idempotency_key: association_key,
                project_id: other_project.project.project_id,
                repository_id: registered.repository.repository_id.clone(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(association_conflict.code, ErrorCode::IdempotencyConflict);
    let revision_conflict = daemon
        .call(
            methods::TASK_REVISE,
            &TaskReviseParams {
                idempotency_key: revision_key,
                task_id: task.task.task_id,
                parent_revision_id: None,
                goal_contract: contract("PRIVATE_CONFLICT_GOAL_MUST_NOT_LEAK"),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(revision_conflict.code, ErrorCode::IdempotencyConflict);
    assert!(!serde_json::to_string(&revision_conflict)
        .unwrap()
        .contains("PRIVATE_CONFLICT_GOAL_MUST_NOT_LEAK"));
    let binding_conflict = daemon
        .call(
            methods::SESSION_BIND,
            &SessionBindParams {
                idempotency_key: binding_key,
                session_id: started.session.session_id,
                project_id: project.project.project_id,
                task_revision_id: revision_two,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(binding_conflict.code, ErrorCode::IdempotencyConflict);

    let db_path = daemon.db_path();
    let pool = cairn_storage_local::open_pool_at(&db_path).await.unwrap();
    for key in [association_key, revision_key, binding_key] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM operation_idempotency WHERE idempotency_key=?",
            )
            .bind(key.to_string())
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM project_repository_associations WHERE repository_id=?",
        )
        .bind(&registered.repository.repository_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM task_revisions WHERE task_id=?")
            .bind(task.task.task_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM session_bindings WHERE session_id=?")
            .bind(started.session.session_id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM events WHERE event_type='project.repository_associated'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM events WHERE event_type='task.revision_created'",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM events WHERE event_type='session.bound'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    let gaps: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT aggregate_type,aggregate_id,aggregate_seq,LAG(aggregate_seq) OVER(PARTITION BY aggregate_type,aggregate_id ORDER BY aggregate_seq) AS previous FROM events WHERE aggregate_seq IS NOT NULL) WHERE previous IS NOT NULL AND aggregate_seq<>previous+1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(gaps, 0);
    pool.close().await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let pool = cairn_storage_local::open_pool_at(&daemon.db_path())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM operation_idempotency WHERE idempotency_key IN (?,?,?)"
        )
        .bind(association_key.to_string())
        .bind(revision_key.to_string())
        .bind(binding_key.to_string())
        .fetch_one(&pool)
        .await
        .unwrap(),
        3
    );
    assert_eq!(
        cairn_storage_local::session_bindings::mode(&pool, &started.session.session_id.to_string())
            .await
            .unwrap()
            .as_deref(),
        Some("project_bound")
    );

    // Restart preserves the same authoritative result for every retried operation.
    let replayed_association: ProjectRepositoryAssociateResult = serde_json::from_value(
        daemon
            .call(methods::PROJECT_REPOSITORY_ASSOCIATE, &association_params)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(&replayed_association, association_result.as_ref().unwrap());
    let replayed_revision: TaskReviseResult = serde_json::from_value(
        daemon
            .call(methods::TASK_REVISE, &revision_params)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(&replayed_revision, revision_result.as_ref().unwrap());
    let replayed_binding: SessionBindResult = serde_json::from_value(
        daemon
            .call(methods::SESSION_BIND, &binding_params)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(&replayed_binding, binding_result.as_ref().unwrap());
    for (event_type, expected) in [
        ("project.repository_associated", 1),
        ("task.revision_created", 2),
        ("session.bound", 1),
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE event_type=?")
                .bind(event_type)
                .fetch_one(&pool)
                .await
                .unwrap(),
            expected,
            "a post-restart retry appended a duplicate {event_type}"
        );
    }
    println!(
        "feature002_retries={{\"association\":100,\"revision\":100,\"binding\":100,\"registry_records\":3,\"sequence_gaps\":0,\"conflicts\":3}}"
    );
    pool.close().await;
    drop(repository);
    daemon.stop().await;
}

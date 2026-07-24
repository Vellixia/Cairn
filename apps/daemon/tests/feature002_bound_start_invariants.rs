mod support;

use cairn_protocol::*;
use cairn_storage_local::{events, open_pool_at, session_bindings, sessions};
use serde_json::json;
use support::binding::BoundStartFixture;
use support::TestDaemon;

#[tokio::test(flavor = "multi_thread")]
async fn bound_start_preserves_feature001_uniqueness_tokens_snapshots_and_watcher_boundary() {
    let daemon = TestDaemon::start().await;
    let fixture = BoundStartFixture::create(&daemon).await;
    let controls = daemon.watcher_controls();
    let installed = controls.installed_count();
    let reconciled = controls.reconciled_count();
    let agent = AgentInstanceId(uuid::Uuid::now_v7());

    let first: SessionStartResult = serde_json::from_value(
        daemon
            .call(methods::SESSION_START, &fixture.start_params(agent))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first.outcome, StartOutcome::Created);
    assert!(first.resume_token.is_some());
    assert_eq!(
        first.session.scope,
        SessionScopeDto::ProjectBound {
            project_id: fixture.project_id,
            task_revision_id: fixture.revision_id,
        }
    );
    assert!(controls.installed_count() > installed);
    assert!(controls.reconciled_count() > reconciled);
    let installed_after_first = controls.installed_count();

    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    let before_retry = sessions::get_by_id(&pool, &first.session.session_id.to_string())
        .await
        .unwrap()
        .unwrap();

    let same: SessionStartResult = serde_json::from_value(
        daemon
            .call(methods::SESSION_START, &fixture.start_params(agent))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(same.outcome, StartOutcome::Existing);
    assert_eq!(same.session.session_id, first.session.session_id);
    assert!(same.resume_token.is_none());
    assert_eq!(
        controls.installed_count(),
        installed_after_first,
        "identical scope reuse must retain the existing OS watcher"
    );

    let row = sessions::get_by_id(&pool, &first.session.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(row.resume_token_hash, first.resume_token.unwrap());
    assert_eq!(row.resume_token_hash, before_retry.resume_token_hash);
    assert_eq!(row.lease_expires_at, before_retry.lease_expires_at);
    assert_eq!(row.start_snapshot_id, row.current_snapshot_id);
    assert!(session_bindings::get(&pool, &row.id)
        .await
        .unwrap()
        .is_some());
    let session_events = events::list_events(&pool, None, None, Some(&row.id), None, 20)
        .await
        .unwrap();
    assert_eq!(
        session_events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["session.started", "session.bound"]
    );
    pool.close().await;
    drop(fixture);
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_bound_session_takeover_interrupts_old_scope_and_creates_one_new_bound_session() {
    let daemon = TestDaemon::start().await;
    let fixture = BoundStartFixture::create(&daemon).await;
    let agent = AgentInstanceId(uuid::Uuid::now_v7());
    let params = fixture.start_params(agent);
    let first: SessionStartResult =
        serde_json::from_value(daemon.call(methods::SESSION_START, &params).await.unwrap())
            .unwrap();
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    sqlx::query("UPDATE sessions SET lease_expires_at='2000-01-01T00:00:00Z' WHERE id=?")
        .bind(first.session.session_id.to_string())
        .execute(&pool)
        .await
        .unwrap();

    let takeover: SessionStartResult =
        serde_json::from_value(daemon.call(methods::SESSION_START, &params).await.unwrap())
            .unwrap();
    assert_eq!(takeover.outcome, StartOutcome::Takeover);
    assert_ne!(takeover.session.session_id, first.session.session_id);
    assert!(takeover.resume_token.is_some());
    assert_eq!(takeover.session.scope, first.session.scope);
    let old = sessions::get_by_id(&pool, &first.session.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.state, "interrupted");
    assert_eq!(old.binding_mode, "project_bound");
    assert_eq!(
        session_bindings::list_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|binding| {
                binding.session_id == first.session.session_id.to_string()
                    || binding.session_id == takeover.session.session_id.to_string()
            })
            .count(),
        2
    );
    let new_events = events::list_events(
        &pool,
        None,
        None,
        Some(&takeover.session.session_id.to_string()),
        None,
        20,
    )
    .await
    .unwrap();
    assert_eq!(
        new_events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["session.started", "session.bound"]
    );
    pool.close().await;
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unbound_bootstrap_gate_and_incomplete_bound_scope_return_stable_typed_errors() {
    let daemon = TestDaemon::start().await;
    let fixture = BoundStartFixture::create(&daemon).await;

    for scope in [None, Some(SessionScopeDto::LocalUnbound)] {
        let mut params = fixture.start_params(AgentInstanceId(uuid::Uuid::now_v7()));
        params.scope = scope;
        let error = daemon
            .call(methods::SESSION_START, &params)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProjectScopeRequired);
        assert_eq!(error.code.exit_code(), 1);
        assert!(matches!(
            error.data,
            Some(ErrorData::ProjectScopeRequired {
                ref repository_id,
                project_id,
            }) if repository_id == &fixture.repository_id && project_id == fixture.project_id
        ));
    }

    let malformed = daemon
        .call(
            methods::SESSION_START,
            &json!({
                "path": fixture.repository.root(),
                "agent_type": "incomplete-bound",
                "agent_instance_id": uuid::Uuid::now_v7(),
                "scope": {
                    "mode": "project_bound",
                    "project_id": fixture.project_id,
                }
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(malformed.code, ErrorCode::Usage);
    assert!(!malformed.message.contains(&fixture.repository_id));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_bound_session_changes_only_lifecycle_state() {
    let daemon = TestDaemon::start().await;
    let fixture = BoundStartFixture::create(&daemon).await;
    let agent = AgentInstanceId(uuid::Uuid::now_v7());
    let started: SessionStartResult = serde_json::from_value(
        daemon
            .call(methods::SESSION_START, &fixture.start_params(agent))
            .await
            .unwrap(),
    )
    .unwrap();
    let stopped: SessionStopResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_STOP,
                &SessionStopParams {
                    session_id: Some(started.session.session_id),
                    repository_id: None,
                    path: None,
                    agent_instance_id: None,
                    resume_token: None,
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(stopped.session.state, cairn_domain::SessionState::Stopped);
    assert_eq!(stopped.session.scope, started.session.scope);
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    assert!(
        session_bindings::get(&pool, &stopped.session.session_id.to_string())
            .await
            .unwrap()
            .is_some()
    );
    pool.close().await;
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bound_start_relationship_failures_are_typed_and_leave_no_session() {
    let daemon = TestDaemon::start().await;
    let first = BoundStartFixture::create(&daemon).await;
    let second = BoundStartFixture::create(&daemon).await;
    let cases = [
        (
            SessionScopeDto::ProjectBound {
                project_id: ProjectId::new_v7(),
                task_revision_id: first.revision_id,
            },
            ErrorCode::ProjectNotFound,
        ),
        (
            SessionScopeDto::ProjectBound {
                project_id: first.project_id,
                task_revision_id: TaskRevisionId::new_v7(),
            },
            ErrorCode::TaskRevisionNotFound,
        ),
        (
            SessionScopeDto::ProjectBound {
                project_id: second.project_id,
                task_revision_id: second.revision_id,
            },
            ErrorCode::RepositoryNotAssociated,
        ),
        (
            SessionScopeDto::ProjectBound {
                project_id: first.project_id,
                task_revision_id: second.revision_id,
            },
            ErrorCode::TaskRevisionProjectMismatch,
        ),
    ];
    for (scope, code) in cases {
        let mut params = first.start_params(AgentInstanceId(uuid::Uuid::now_v7()));
        params.scope = Some(scope);
        let error = daemon
            .call(methods::SESSION_START, &params)
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
    }

    daemon
        .call(
            methods::PROJECT_UPDATE,
            &ProjectUpdateParams {
                idempotency_key: IdempotencyKey::new_v7(),
                project_id: first.project_id,
                name: None,
                description: None,
                clear_description: false,
                status: Some(cairn_domain::ProjectStatus::Archived),
            },
        )
        .await
        .unwrap();
    let error = daemon
        .call(
            methods::SESSION_START,
            &first.start_params(AgentInstanceId(uuid::Uuid::now_v7())),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ProjectArchived);

    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    assert!(sessions::list(&pool, Some(&first.repository_id), None)
        .await
        .unwrap()
        .is_empty());
    pool.close().await;
    daemon.stop().await;
}

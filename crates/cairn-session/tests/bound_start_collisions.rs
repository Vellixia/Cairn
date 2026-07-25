mod support;

use std::sync::Arc;

use cairn_domain::{EventId, IdempotencyKey, ProjectId, SessionBindingMode};
use cairn_project::{ReviseTask, TaskService};
use cairn_session::{SessionError, StartOutcome};
use cairn_storage_local::{events, session_bindings, WorktreeWriters};
use support::{contract, Harness};

#[tokio::test]
async fn identical_scope_reuses_healthy_session_and_incompatible_scope_never_converts_it() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let (project_id, _, revision_id) = harness.add_project_scope(&context).await;
    let agent = EventId::new_v7().to_string();
    let scope = SessionBindingMode::ProjectBound {
        project_id,
        task_revision_id: revision_id,
    };
    let first = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bound-agent",
            &agent,
            None,
            &context.snapshot,
            scope,
        )
        .await
        .unwrap();
    let same = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bound-agent",
            &agent,
            None,
            &context.snapshot,
            scope,
        )
        .await
        .unwrap();
    assert_eq!(same.outcome, StartOutcome::Existing);
    assert_eq!(same.session.id, first.session.id);

    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bound-agent",
            &agent,
            None,
            &context.snapshot,
            SessionBindingMode::LocalUnbound,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, SessionError::ScopeConflict { .. }));
    assert_eq!(
        events::list_events(&harness.pool, None, None, Some(&first.session.id), None, 10,)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(session_bindings::get(&harness.pool, &first.session.id)
        .await
        .unwrap()
        .is_some());

    for conflicting_scope in [
        SessionBindingMode::ProjectBound {
            project_id,
            task_revision_id: cairn_domain::TaskRevisionId::new_v7(),
        },
        SessionBindingMode::ProjectBound {
            project_id: ProjectId::new_v7(),
            task_revision_id: revision_id,
        },
    ] {
        let error = harness
            .sessions()
            .start(
                &context.repository_id,
                &context.worktree_id,
                "tester",
                "bound-agent",
                &agent,
                None,
                &context.snapshot,
                conflicting_scope,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, SessionError::ScopeConflict { .. }));
    }
}

#[tokio::test]
async fn healthy_unbound_session_is_never_implicitly_bound_after_scope_becomes_available() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let agent = EventId::new_v7().to_string();
    let first = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bootstrap-agent",
            &agent,
            None,
            &context.snapshot,
            SessionBindingMode::LocalUnbound,
        )
        .await
        .unwrap();
    let (project_id, _, revision_id) = harness.add_project_scope(&context).await;
    let error = harness
        .sessions()
        .start(
            &context.repository_id,
            &context.worktree_id,
            "tester",
            "bootstrap-agent",
            &agent,
            None,
            &context.snapshot,
            SessionBindingMode::ProjectBound {
                project_id,
                task_revision_id: revision_id,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, SessionError::ScopeConflict { .. }));
    assert_eq!(first.session.binding_mode, "local_unbound");
    assert!(session_bindings::get(&harness.pool, &first.session.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_connections_commit_exactly_one_of_two_different_bound_scopes() {
    let harness = Harness::new().await;
    let context = harness.start_context().await;
    let (project_id, task_id, revision_one) = harness.add_project_scope(&context).await;
    let revision_two = TaskService::new(harness.pool.clone())
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id,
            parent_revision_id: Some(revision_one),
            goal_contract: contract("revision two"),
        })
        .await
        .unwrap()
        .revision
        .id;
    let agent = EventId::new_v7().to_string();
    let pool_one = harness.independent_pool().await;
    let pool_two = harness.independent_pool().await;
    let service_one = cairn_session::SessionService::new(
        pool_one.clone(),
        Arc::new(WorktreeWriters::new()),
        cairn_session::SessionConfig::from_env(),
    );
    let service_two = cairn_session::SessionService::new(
        pool_two.clone(),
        Arc::new(WorktreeWriters::new()),
        cairn_session::SessionConfig::from_env(),
    );
    let start_one = service_one.start(
        &context.repository_id,
        &context.worktree_id,
        "tester",
        "race-agent",
        &agent,
        None,
        &context.snapshot,
        SessionBindingMode::ProjectBound {
            project_id,
            task_revision_id: revision_one,
        },
    );
    let start_two = service_two.start(
        &context.repository_id,
        &context.worktree_id,
        "tester",
        "race-agent",
        &agent,
        None,
        &context.snapshot,
        SessionBindingMode::ProjectBound {
            project_id,
            task_revision_id: revision_two,
        },
    );
    let (one, two) = tokio::join!(start_one, start_two);
    assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
    assert_eq!(
        usize::from(matches!(one, Err(SessionError::ScopeConflict { .. })))
            + usize::from(matches!(two, Err(SessionError::ScopeConflict { .. }))),
        1
    );

    let rows =
        cairn_storage_local::sessions::list(&harness.pool, Some(&context.repository_id), None)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        session_bindings::list_all(&harness.pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|binding| binding.session_id == rows[0].id)
            .count(),
        1
    );
    assert_eq!(
        events::list_events(&harness.pool, None, None, Some(&rows[0].id), None, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    pool_one.close().await;
    pool_two.close().await;
}

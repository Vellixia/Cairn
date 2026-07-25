mod support;

use std::time::Duration;

use cairn_domain::{IdempotencyKey, SessionState};
use cairn_project::{ReviseTask, TaskService};
use cairn_session::{BindSession, SessionBindingError, SessionService};
use cairn_storage_local::writer::{WriteCheckpoint, WriteTestHooks, WriterPolicy};
use cairn_storage_local::{events, operation_idempotency, session_bindings, sessions};
use support::{contract, Harness};

#[tokio::test]
async fn independent_pools_serialize_identical_bindings_to_one_result() {
    let harness = Harness::new().await;
    let context = harness.context(SessionState::Active).await;
    let second_pool = harness.independent_pool().await;
    let hooks = WriteTestHooks::default();
    hooks.pause_at(WriteCheckpoint::PreCommit);
    let first_service = SessionService::with_binding_test_controls(
        harness.pool.clone(),
        harness.sessions().writers().clone(),
        harness.sessions().config,
        WriterPolicy::test_with_busy_timeout(Duration::from_secs(5)),
        hooks.clone(),
    );
    let second_service = SessionService::new(
        second_pool,
        harness.sessions().writers().clone(),
        harness.sessions().config,
    );
    let key = IdempotencyKey::new_v7();
    let request = BindSession {
        idempotency_key: key,
        session_id: context.session.id.parse().unwrap(),
        project_id: context.project_id,
        task_revision_id: context.revision_id,
    };
    let first_request = request.clone();
    let first = tokio::spawn(async move { first_service.bind(first_request).await });
    hooks.wait_until_reached(WriteCheckpoint::PreCommit).await;
    let second = tokio::spawn(async move { second_service.bind(request).await });
    hooks.resume(WriteCheckpoint::PreCommit);
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first, second);
    assert!(first.created);
    assert!(operation_idempotency::get(&harness.pool, &key.to_string())
        .await
        .unwrap()
        .is_some());
    assert_eq!(binding_counts(&harness, &context.session.id).await, (1, 1));
}

#[tokio::test]
async fn independent_pools_commit_one_of_two_different_bindings_without_partial_loser_state() {
    let harness = Harness::new().await;
    let context = harness.context(SessionState::Recovering).await;
    let revision_two = TaskService::new(harness.pool.clone())
        .revise(ReviseTask {
            idempotency_key: IdempotencyKey::new_v7(),
            task_id: context.task_id,
            parent_revision_id: None,
            goal_contract: contract("revision two"),
        })
        .await
        .unwrap()
        .revision;
    let second_pool = harness.independent_pool().await;
    let hooks = WriteTestHooks::default();
    hooks.pause_at(WriteCheckpoint::PreCommit);
    let first_service = SessionService::with_binding_test_controls(
        harness.pool.clone(),
        harness.sessions().writers().clone(),
        harness.sessions().config,
        WriterPolicy::test_with_busy_timeout(Duration::from_secs(5)),
        hooks.clone(),
    );
    let second_service = SessionService::new(
        second_pool,
        harness.sessions().writers().clone(),
        harness.sessions().config,
    );
    let winner_key = IdempotencyKey::new_v7();
    let loser_key = IdempotencyKey::new_v7();
    let first = tokio::spawn({
        let context = context.clone();
        async move {
            first_service
                .bind(BindSession {
                    idempotency_key: winner_key,
                    session_id: context.session.id.parse().unwrap(),
                    project_id: context.project_id,
                    task_revision_id: context.revision_id,
                })
                .await
        }
    });
    hooks.wait_until_reached(WriteCheckpoint::PreCommit).await;
    let second = tokio::spawn({
        let context = context.clone();
        async move {
            second_service
                .bind(BindSession {
                    idempotency_key: loser_key,
                    session_id: context.session.id.parse().unwrap(),
                    project_id: context.project_id,
                    task_revision_id: revision_two.id,
                })
                .await
        }
    });
    hooks.resume(WriteCheckpoint::PreCommit);
    assert!(first.await.unwrap().unwrap().created);
    assert!(matches!(
        second.await.unwrap().unwrap_err(),
        SessionBindingError::SessionAlreadyBound { .. }
    ));
    assert!(
        operation_idempotency::get(&harness.pool, &loser_key.to_string())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(binding_counts(&harness, &context.session.id).await, (1, 1));
    assert_eq!(
        sessions::get_by_id(&harness.pool, &context.session.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        "recovering"
    );
}

async fn binding_counts(harness: &Harness, session_id: &str) -> (i64, usize) {
    let projections: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM session_bindings WHERE session_id=?")
            .bind(session_id)
            .fetch_one(&harness.pool)
            .await
            .unwrap();
    let event_count = events::list_events(&harness.pool, None, None, Some(session_id), None, 100)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.event_type == "session.bound")
        .count();
    assert!(session_bindings::get(&harness.pool, session_id)
        .await
        .unwrap()
        .is_some());
    (projections.0, event_count)
}

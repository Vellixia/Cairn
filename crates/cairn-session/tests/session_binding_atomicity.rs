mod support;

use std::time::Duration;

use cairn_domain::{IdempotencyKey, SessionState};
use cairn_session::{BindSession, SessionService};
use cairn_storage_local::writer::{WriteCheckpoint, WriteTestHooks, WriterPolicy};
use cairn_storage_local::{events, operation_idempotency, session_bindings, sessions};
use support::Harness;

#[tokio::test]
async fn every_binding_write_boundary_rolls_back_registry_event_head_projection_and_mode() {
    for checkpoint in [
        WriteCheckpoint::PostRegistryReservation,
        WriteCheckpoint::PreEvent,
        WriteCheckpoint::PreProjection,
        WriteCheckpoint::PreCommit,
    ] {
        let harness = Harness::new().await;
        let context = harness.context(SessionState::Interrupted).await;
        let key = IdempotencyKey::new_v7();
        let before_events = events::list_events(
            &harness.pool,
            None,
            None,
            Some(&context.session.id),
            None,
            100,
        )
        .await
        .unwrap();
        let before_head: Option<(i64,)> = sqlx::query_as(
            "SELECT last_seq FROM event_aggregate_heads WHERE aggregate_type='session' AND aggregate_id=?",
        )
        .bind(&context.session.id)
        .fetch_optional(&harness.pool)
        .await
        .unwrap();
        let hooks = WriteTestHooks::default();
        hooks.fail_at(checkpoint);
        let service = SessionService::with_binding_test_controls(
            harness.pool.clone(),
            harness.sessions().writers().clone(),
            harness.sessions().config,
            WriterPolicy::test_with_busy_timeout(Duration::from_secs(1)),
            hooks,
        );
        assert!(service
            .bind(BindSession {
                idempotency_key: key,
                session_id: context.session.id.parse().unwrap(),
                project_id: context.project_id,
                task_revision_id: context.revision_id,
            })
            .await
            .is_err());
        assert!(operation_idempotency::get(&harness.pool, &key.to_string())
            .await
            .unwrap()
            .is_none());
        assert!(session_bindings::get(&harness.pool, &context.session.id)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            sessions::get_by_id(&harness.pool, &context.session.id)
                .await
                .unwrap()
                .unwrap()
                .binding_mode,
            "local_unbound"
        );
        let after_events = events::list_events(
            &harness.pool,
            None,
            None,
            Some(&context.session.id),
            None,
            100,
        )
        .await
        .unwrap();
        assert_eq!(after_events.len(), before_events.len());
        let after_head: Option<(i64,)> = sqlx::query_as(
            "SELECT last_seq FROM event_aggregate_heads WHERE aggregate_type='session' AND aggregate_id=?",
        )
        .bind(&context.session.id)
        .fetch_optional(&harness.pool)
        .await
        .unwrap();
        assert_eq!(after_head, before_head);
    }
}

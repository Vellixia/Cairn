mod support;

use cairn_domain::{IdempotencyKey, SessionState};
use cairn_session::BindSession;
use cairn_storage_local::{session_bindings, sessions};
use support::{stable_session_events, Harness};

#[tokio::test]
async fn binding_preserves_every_feature001_lifecycle_state_and_field() {
    for state in [
        SessionState::Active,
        SessionState::Recovering,
        SessionState::Stopped,
        SessionState::Interrupted,
    ] {
        let harness = Harness::new().await;
        let context = harness.context(state).await;
        let before = sessions::get_by_id(&harness.pool, &context.session.id)
            .await
            .unwrap()
            .unwrap();
        let prior_events = stable_session_events(&harness.pool, &before.id).await;
        let result = harness
            .sessions()
            .bind(BindSession {
                idempotency_key: IdempotencyKey::new_v7(),
                session_id: before.id.parse().unwrap(),
                project_id: context.project_id,
                task_revision_id: context.revision_id,
            })
            .await
            .unwrap();
        assert!(result.created);
        let after = sessions::get_by_id(&harness.pool, &before.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.state, before.state);
        assert_eq!(after.repository_id, before.repository_id);
        assert_eq!(after.worktree_id, before.worktree_id);
        assert_eq!(after.agent_instance_id, before.agent_instance_id);
        assert_eq!(after.start_snapshot_id, before.start_snapshot_id);
        assert_eq!(after.current_snapshot_id, before.current_snapshot_id);
        assert_eq!(after.resume_token_hash, before.resume_token_hash);
        assert_eq!(after.lease_expires_at, before.lease_expires_at);
        assert_eq!(after.started_at, before.started_at);
        assert_eq!(after.ended_at, before.ended_at);
        assert_eq!(after.last_heartbeat_at, before.last_heartbeat_at);
        assert_eq!(after.recovering_since, before.recovering_since);
        assert_eq!(after.binding_mode, "project_bound");
        assert_eq!(
            &stable_session_events(&harness.pool, &before.id).await[..prior_events.len()],
            prior_events
        );
        assert_eq!(
            session_bindings::get(&harness.pool, &before.id)
                .await
                .unwrap()
                .unwrap()
                .task_revision_id,
            context.revision_id.to_string()
        );
    }
}

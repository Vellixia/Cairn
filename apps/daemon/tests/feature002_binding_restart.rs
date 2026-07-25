mod support;

use cairn_events::replay::{live_session_binding_projections, rebuild_session_binding_projections};
use cairn_protocol::*;
use cairn_storage_local::{events, open_pool_at, session_bindings, sessions};
use support::binding::{contract, BindingFixture};
use support::TestDaemon;

#[tokio::test(flavor = "multi_thread")]
async fn binding_survives_two_restarts_and_stays_on_immutable_revision_one() {
    let daemon = TestDaemon::start().await;
    let fixture = BindingFixture::create(&daemon).await;
    daemon
        .call(
            methods::SESSION_STOP,
            &SessionStopParams {
                session_id: Some(fixture.session_id),
                repository_id: None,
                path: None,
                agent_instance_id: None,
                resume_token: None,
            },
        )
        .await
        .unwrap();
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    let before = sessions::get_by_id(&pool, &fixture.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.state, "stopped");
    let prior = stable_events(&pool, &fixture.session_id.to_string()).await;
    let bound: SessionBindResult = serde_json::from_value(
        daemon
            .call(
                methods::SESSION_BIND,
                &fixture.bind_params(IdempotencyKey::new_v7()),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(bound.created);
    assert_eq!(
        sessions::get_by_id(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .state,
        "stopped"
    );
    assert_eq!(
        &stable_events(&pool, &fixture.session_id.to_string()).await[..prior.len()],
        prior
    );
    pool.close().await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    let after_restart = sessions::get_by_id(&pool, &fixture.session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_restart.state, "stopped");
    assert_eq!(after_restart.binding_mode, "project_bound");
    assert_eq!(
        session_bindings::get(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .task_revision_id,
        fixture.revision_id.to_string()
    );
    let revised: TaskReviseResult = serde_json::from_value(
        daemon
            .call(
                methods::TASK_REVISE,
                &TaskReviseParams {
                    idempotency_key: IdempotencyKey::new_v7(),
                    task_id: fixture.task_id,
                    parent_revision_id: None,
                    goal_contract: contract("revision two"),
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(revised.revision.revision_number, 2);
    assert_ne!(revised.revision.revision_id, fixture.revision_id);
    assert_eq!(
        session_bindings::get(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .task_revision_id,
        fixture.revision_id.to_string()
    );
    pool.close().await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let pool = open_pool_at(&daemon.db_path()).await.unwrap();
    assert_eq!(
        session_bindings::get(&pool, &fixture.session_id.to_string())
            .await
            .unwrap()
            .unwrap()
            .task_revision_id,
        fixture.revision_id.to_string()
    );
    assert_eq!(
        rebuild_session_binding_projections(&pool).await.unwrap(),
        live_session_binding_projections(&pool).await.unwrap()
    );
    assert_eq!(
        &stable_events(&pool, &fixture.session_id.to_string()).await[..prior.len()],
        prior
    );
    pool.close().await;
    drop(fixture);
    daemon.stop().await;
}

async fn stable_events(pool: &sqlx::SqlitePool, session_id: &str) -> Vec<String> {
    events::list_events(pool, None, None, Some(session_id), None, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|event| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{:?}",
                event.seq,
                event.id,
                event.idempotency_key,
                event.event_type,
                event.repository_id.as_deref().unwrap_or(""),
                event.worktree_id.as_deref().unwrap_or(""),
                event.session_id.as_deref().unwrap_or(""),
                event.snapshot_id.as_deref().unwrap_or(""),
                event.payload,
                event.recorded_at,
                event.aggregate_seq,
            )
        })
        .collect()
}

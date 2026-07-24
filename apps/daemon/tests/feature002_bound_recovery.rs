mod support;

use cairn_protocol::*;
use cairn_storage_local::{
    events, open_pool_at, session_bindings, sessions, WriteCheckpoint, WriteTestHooks,
};
use support::binding::BoundStartFixture;
use support::{test_config, TestDaemon};

const CYCLES: usize = 20;

#[tokio::test(flavor = "multi_thread")]
async fn bound_session_survives_exactly_twenty_restart_and_reattach_cycles() {
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
    let session_id = started.session.session_id;
    let mut token = started.resume_token.unwrap();
    let (mut dir, mut config) = daemon.stop().await;

    for cycle in 1..=CYCLES {
        let before_install = config
            .watcher_test_controls
            .as_ref()
            .expect("watcher controls")
            .installed_count();
        let daemon = TestDaemon::start_with(dir, config).await;
        assert!(
            daemon.watcher_controls().installed_count() > before_install,
            "startup did not reinstall the watcher at recovery cycle {cycle}"
        );
        let recovered: SessionReattachResult = serde_json::from_value(
            daemon
                .call(
                    methods::SESSION_REATTACH,
                    &SessionReattachParams {
                        session_id,
                        agent_instance_id: agent,
                        resume_token: token,
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(recovered.session.state, cairn_domain::SessionState::Active);
        assert_eq!(
            recovered.session.scope,
            SessionScopeDto::ProjectBound {
                project_id: fixture.project_id,
                task_revision_id: fixture.revision_id,
            },
            "binding changed at recovery cycle {cycle}"
        );
        token = recovered.resume_token.unwrap();

        let pool = open_pool_at(&daemon.db_path()).await.unwrap();
        assert_eq!(
            sessions::get_by_id(&pool, &session_id.to_string())
                .await
                .unwrap()
                .unwrap()
                .binding_mode,
            "project_bound"
        );
        assert_eq!(
            session_bindings::list_all(&pool)
                .await
                .unwrap()
                .into_iter()
                .filter(|binding| binding.session_id == session_id.to_string())
                .count(),
            1
        );
        assert_eq!(
            events::list_events(&pool, None, None, Some(&session_id.to_string()), None, 100,)
                .await
                .unwrap()
                .iter()
                .filter(|event| event.event_type == "session.bound")
                .count(),
            1
        );
        pool.close().await;
        (dir, config) = daemon.stop().await;
    }
    assert_eq!(CYCLES, 20);
    drop(fixture);
}

#[tokio::test(flavor = "multi_thread")]
async fn recovery_and_reattach_barriers_never_expose_partial_binding_or_lifecycle_state() {
    let dir = tempfile::TempDir::new().unwrap();
    let hooks = WriteTestHooks::default();
    let mut config = test_config(&dir);
    config.session_test_hooks = Some(hooks.clone());
    let daemon = TestDaemon::start_with(dir, config).await;
    let fixture = BoundStartFixture::create(&daemon).await;
    let agent = AgentInstanceId(uuid::Uuid::now_v7());
    let started: SessionStartResult = serde_json::from_value(
        daemon
            .call(methods::SESSION_START, &fixture.start_params(agent))
            .await
            .unwrap(),
    )
    .unwrap();
    let session_id = started.session.session_id;
    let token = started.resume_token.unwrap();
    let (dir, config) = daemon.stop().await;
    let observer = open_pool_at(&config.db_path()).await.unwrap();

    hooks.pause_at(WriteCheckpoint::PreCommit);
    let startup = tokio::spawn(TestDaemon::start_with(dir, config));
    hooks.wait_until_reached(WriteCheckpoint::PreCommit).await;
    let before_recovery = sessions::get_by_id(&observer, &session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before_recovery.state, "active");
    assert_eq!(before_recovery.binding_mode, "project_bound");
    assert!(session_bindings::get(&observer, &session_id.to_string())
        .await
        .unwrap()
        .is_some());
    assert!(!startup.is_finished());
    hooks.resume(WriteCheckpoint::PreCommit);
    let daemon = startup.await.unwrap();

    hooks.pause_at(WriteCheckpoint::PreCommit);
    let mut client = daemon.client().await;
    let reattach = tokio::spawn(async move {
        client
            .call(
                methods::SESSION_REATTACH,
                &SessionReattachParams {
                    session_id,
                    agent_instance_id: agent,
                    resume_token: token,
                },
            )
            .await
    });
    hooks.wait_until_reached(WriteCheckpoint::PreCommit).await;
    let during_reattach = sessions::get_by_id(&observer, &session_id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(during_reattach.state, "recovering");
    assert_eq!(during_reattach.binding_mode, "project_bound");
    assert_eq!(
        session_bindings::list_all(&observer)
            .await
            .unwrap()
            .into_iter()
            .filter(|binding| binding.session_id == session_id.to_string())
            .count(),
        1
    );
    assert!(!reattach.is_finished());
    hooks.resume(WriteCheckpoint::PreCommit);
    let result: SessionReattachResult =
        serde_json::from_value(reattach.await.unwrap().unwrap()).unwrap();
    assert_eq!(result.session.state, cairn_domain::SessionState::Active);
    assert_eq!(
        result.session.scope,
        SessionScopeDto::ProjectBound {
            project_id: fixture.project_id,
            task_revision_id: fixture.revision_id,
        }
    );
    observer.close().await;
    daemon.stop().await;
}

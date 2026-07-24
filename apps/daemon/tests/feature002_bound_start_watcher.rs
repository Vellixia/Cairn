mod support;

use cairn_domain::{AgentInstanceId, WatcherStartStage};
use cairn_protocol::*;
use cairn_storage_local::{
    events, open_pool_at, session_bindings, sessions, WriteCheckpoint, WriteTestHooks,
};
use support::binding::BoundStartFixture;
use support::{test_config, TestDaemon};

#[tokio::test(flavor = "multi_thread")]
async fn bound_start_is_atomically_invisible_then_reconciles_dropped_and_coalesced_changes() {
    let dir = tempfile::TempDir::new().unwrap();
    let hooks = WriteTestHooks::default();
    let mut config = test_config(&dir);
    config.session_test_hooks = Some(hooks.clone());
    let daemon = TestDaemon::start_with(dir, config).await;
    let fixture = BoundStartFixture::create(&daemon).await;
    let observer = open_pool_at(&daemon.db_path()).await.unwrap();
    let controls = daemon.watcher_controls();
    hooks.pause_at(WriteCheckpoint::PreCommit);
    controls.pause_before_install();
    controls.pause_before_reconcile();
    controls.drop_notifications();

    let mut client = daemon.client().await;
    let params = fixture.start_params(AgentInstanceId(uuid::Uuid::now_v7()));
    let start = tokio::spawn(async move { client.call(methods::SESSION_START, &params).await });

    hooks.wait_until_reached(WriteCheckpoint::PreCommit).await;
    assert!(
        sessions::list(&observer, Some(&fixture.repository_id), None)
            .await
            .unwrap()
            .is_empty(),
        "the session projection became visible before its transaction committed"
    );
    let before_commit = events::list_events(&observer, None, None, None, None, 1_000)
        .await
        .unwrap();
    assert!(!before_commit.iter().any(|event| matches!(
        event.event_type.as_str(),
        "session.started" | "session.bound"
    )));
    hooks.resume(WriteCheckpoint::PreCommit);

    controls.wait_before_install().await;
    assert!(
        !start.is_finished(),
        "watcher installation is a success boundary"
    );
    fixture
        .repository
        .write("install-window.txt", "created while install paused\n")
        .unwrap();
    controls.release_install();

    controls.wait_before_reconcile().await;
    for index in 0..32 {
        fixture
            .repository
            .write("burst.txt", &format!("coalesced value {index}\n"))
            .unwrap();
    }
    fixture.repository.delete("README.md").unwrap();
    assert!(!start.is_finished(), "reconciliation is a success boundary");
    controls.release_reconcile();

    let started: SessionStartResult =
        serde_json::from_value(start.await.unwrap().unwrap()).unwrap();
    assert_eq!(
        started.session.scope,
        SessionScopeDto::ProjectBound {
            project_id: fixture.project_id,
            task_revision_id: fixture.revision_id,
        }
    );
    let authoritative = cairn_git::fingerprint::fingerprint_state(fixture.repository.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    assert_eq!(
        started.session.current_snapshot.snapshot_fp, authoritative,
        "Git reconciliation must recover every dropped/coalesced notification"
    );

    let stored = events::list_events(
        &observer,
        None,
        None,
        Some(&started.session.session_id.to_string()),
        None,
        100,
    )
    .await
    .unwrap();
    let types: Vec<_> = stored
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(types[0..2], ["session.started", "session.bound"]);
    let repository_events = events::list_events(
        &observer,
        Some(&fixture.repository_id),
        None,
        None,
        None,
        1_000,
    )
    .await
    .unwrap();
    assert_eq!(
        repository_events
            .iter()
            .filter(|event| event.event_type == "repository.state_changed")
            .count(),
        1,
        "authoritative reconciliation records one logical final delta"
    );
    assert_eq!(
        session_bindings::list_all(&observer)
            .await
            .unwrap()
            .into_iter()
            .filter(|binding| binding.session_id == started.session.session_id.to_string())
            .count(),
        1
    );
    observer.close().await;
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_failures_preserve_committed_binding_and_append_interruption_in_order() {
    for stage in [WatcherStartStage::Install, WatcherStartStage::Reconcile] {
        let daemon = TestDaemon::start().await;
        let fixture = BoundStartFixture::create(&daemon).await;
        let controls = daemon.watcher_controls();
        match stage {
            WatcherStartStage::Install => controls.force_install_failure(),
            WatcherStartStage::Reconcile => controls.force_reconcile_failure(),
        }
        let error = daemon
            .call(
                methods::SESSION_START,
                &fixture.start_params(AgentInstanceId(uuid::Uuid::now_v7())),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::WatcherStartFailed);
        assert_eq!(
            error.data.and_then(|data| data.watcher_stage_ref()),
            Some(stage)
        );

        let listed: SessionListResult = serde_json::from_value(
            daemon
                .call(
                    methods::SESSION_LIST,
                    &SessionListParams {
                        repository_id: Some(fixture.repository_id.clone()),
                        state: None,
                        project_id: None,
                        task_revision_id: None,
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(listed.sessions.len(), 1);
        assert_eq!(
            listed.sessions[0].scope,
            SessionScopeDto::ProjectBound {
                project_id: fixture.project_id,
                task_revision_id: fixture.revision_id,
            }
        );
        assert_eq!(
            listed.sessions[0].state,
            cairn_domain::SessionState::Interrupted
        );
        let session_id = listed.sessions[0].session_id;
        let pool = open_pool_at(&daemon.db_path()).await.unwrap();
        let event_types: Vec<_> =
            events::list_events(&pool, None, None, Some(&session_id.to_string()), None, 100)
                .await
                .unwrap()
                .into_iter()
                .map(|event| event.event_type)
                .collect();
        assert_eq!(
            event_types,
            vec!["session.started", "session.bound", "session.interrupted"]
        );
        assert!(session_bindings::get(&pool, &session_id.to_string())
            .await
            .unwrap()
            .is_some());
        pool.close().await;

        let (dir, config) = daemon.stop().await;
        let daemon = TestDaemon::start_with(dir, config).await;
        let shown: SessionGetResult = serde_json::from_value(
            daemon
                .call(
                    methods::SESSION_GET,
                    &SessionGetParams {
                        path: None,
                        repository_id: None,
                        session_id: Some(session_id),
                        agent_instance_id: None,
                        agent_type: None,
                    },
                )
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            shown.session.unwrap().scope,
            SessionScopeDto::ProjectBound { .. }
        ));
        daemon.stop().await;
    }
}

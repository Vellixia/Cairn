//! T052: recovery paths — recovering on restart, authenticated reattach,
//! reject-only mismatch, restart-proof grace anchor, expiry, authenticated
//! stop, corruption honesty.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::{test_config, TestDaemon};

fn path_of(repo: &FixtureRepo) -> String {
    repo.root().to_string_lossy().to_string()
}

async fn start_session(daemon: &TestDaemon, repo: &FixtureRepo, inst: &str) -> (String, String) {
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(repo)}),
        )
        .await
        .unwrap();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(repo), "agent_type": "recov", "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    (
        started["session"]["session_id"]
            .as_str()
            .unwrap()
            .to_string(),
        started["resume_token"].as_str().unwrap().to_string(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_moves_active_to_recovering_and_valid_reattach_resumes() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let (sid, token) = start_session(&daemon, &repo, &inst).await;

    let (dir, config) = daemon.stop().await;
    // Repo changes while the daemon is down (US4-4).
    repo.write("while-down.txt", "offline change\n").unwrap();
    let daemon = TestDaemon::start_with(dir, config).await;

    let get = daemon
        .call(methods::SESSION_GET, &json!({"session_id": sid}))
        .await
        .unwrap();
    assert_eq!(get["session"]["state"], json!("recovering"));
    assert!(get["session"]["recovering_since"].is_string());

    let reattached = daemon
        .call(
            methods::SESSION_REATTACH,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": token}),
        )
        .await
        .unwrap();
    assert_eq!(reattached["session"]["state"], json!("active"));
    assert!(reattached["session"]["recovering_since"].is_null());
    assert!(
        reattached["resume_token"].is_string(),
        "fresh token on recovery"
    );
    // Fresh snapshot reflects the change made during downtime.
    assert_ne!(
        reattached["fresh_snapshot"]["snapshot_fp"],
        reattached["session"]["start_snapshot"]["snapshot_fp"]
    );

    let events = daemon
        .call(methods::EVENTS_LIST, &json!({"session_id": sid}))
        .await
        .unwrap();
    assert!(events["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["event_type"] == "session.recovered"));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_reinstalls_watcher_and_reconciles_before_daemon_readiness() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let (sid, _token) = start_session(&daemon, &repo, &inst).await;
    let controls = daemon.watcher_controls();
    let installed_before = controls.installed_count();

    let (dir, config) = daemon.stop().await;
    repo.write("restart-window.txt", "must be reconciled\n")
        .unwrap();
    controls.pause_before_reconcile();
    let restart = tokio::spawn(TestDaemon::start_with(dir, config));

    controls.wait_before_reconcile().await;
    assert!(
        controls.installed_count() > installed_before,
        "the operating-system watcher was reinstalled"
    );
    assert!(
        !restart.is_finished(),
        "daemon readiness must wait for restart reconciliation"
    );
    controls.release_reconcile();
    let daemon = restart.await.unwrap();

    let get = daemon
        .call(methods::SESSION_GET, &json!({"session_id": sid}))
        .await
        .unwrap();
    let authoritative = cairn_git::fingerprint::fingerprint_state(repo.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    assert_eq!(get["session"]["state"], json!("recovering"));
    assert_eq!(
        get["session"]["current_snapshot"]["snapshot_fp"],
        authoritative
    );
    let events = daemon
        .call(methods::EVENTS_LIST, &json!({"limit": 1000}))
        .await
        .unwrap();
    assert_eq!(
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["event_type"] == "repository.state_changed")
            .count(),
        1,
        "restart reconciliation records the offline change once"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_rejects_only_and_session_stays_recovering() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let (sid, token) = start_session(&daemon, &repo, &inst).await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;

    let err = daemon
        .call(
            methods::SESSION_REATTACH,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": "wrong-token"}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(err.code).unwrap(),
        json!("LEASE_MISMATCH")
    );

    // Session untouched (analysis I3) + audit event without token values.
    let get = daemon
        .call(methods::SESSION_GET, &json!({"session_id": sid}))
        .await
        .unwrap();
    assert_eq!(get["session"]["state"], json!("recovering"));

    let events = daemon
        .call(methods::EVENTS_LIST, &json!({"session_id": sid}))
        .await
        .unwrap();
    let audit = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event_type"] == "session.reattach_rejected")
        .expect("audit event recorded");
    let payload = serde_json::to_string(&audit["payload"]).unwrap();
    assert!(
        !payload.contains("wrong-token"),
        "audit event must not contain token values"
    );

    // The legitimate owner can still reattach afterwards.
    let ok = daemon
        .call(
            methods::SESSION_REATTACH,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": token}),
        )
        .await
        .unwrap();
    assert_eq!(ok["session"]["state"], json!("active"));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_restarts_do_not_extend_the_grace_deadline() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let (sid, _token) = start_session(&daemon, &repo, &inst).await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let get1 = daemon
        .call(methods::SESSION_GET, &json!({"session_id": sid}))
        .await
        .unwrap();
    let since1 = get1["session"]["recovering_since"]
        .as_str()
        .unwrap()
        .to_string();

    // Second restart: recovering_since must be preserved verbatim (A2).
    let (dir, config) = daemon.stop().await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let daemon = TestDaemon::start_with(dir, config).await;
    let get2 = daemon
        .call(methods::SESSION_GET, &json!({"session_id": sid}))
        .await
        .unwrap();
    let since2 = get2["session"]["recovering_since"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        since1, since2,
        "grace anchor must survive restarts unchanged"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn grace_expiry_interrupts_with_reason_codes() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&dir);
    config.session.grace_secs = 1;
    config.sweep_interval_ms = 100;
    let daemon = TestDaemon::start_with(dir, config).await;

    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let (sid, token) = start_session(&daemon, &repo, &inst).await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;

    // Sweeper interrupts once recovering_since + 1s passes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let get = daemon
            .call(methods::SESSION_GET, &json!({"session_id": sid}))
            .await
            .unwrap();
        if get["session"]["state"] == json!("interrupted") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "sweeper never interrupted the session"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let events = daemon
        .call(methods::EVENTS_LIST, &json!({"session_id": sid}))
        .await
        .unwrap();
    let interrupted = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event_type"] == "session.interrupted")
        .expect("interrupted event");
    assert_eq!(interrupted["payload"]["reason"], json!("grace_expired"));
    assert_eq!(
        interrupted["payload"]["liveness_detail"],
        json!("reattach_timeout")
    );

    // Late reattach with the (now useless) token: session is terminal.
    let err = daemon
        .call(
            methods::SESSION_REATTACH,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": token}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(err.code).unwrap(),
        json!("SESSION_NOT_RECOVERING")
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn authenticated_stop_of_recovering_session() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let inst = uuid::Uuid::new_v4().to_string();
    let (sid, token) = start_session(&daemon, &repo, &inst).await;

    let (dir, config) = daemon.stop().await;
    let daemon = TestDaemon::start_with(dir, config).await;

    // Stop without token → rejected; with token → recovering→stopped.
    let err = daemon
        .call(methods::SESSION_STOP, &json!({"session_id": sid}))
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(err.code).unwrap(),
        json!("LEASE_MISMATCH")
    );

    let stopped = daemon
        .call(
            methods::SESSION_STOP,
            &json!({"session_id": sid, "resume_token": token}),
        )
        .await
        .unwrap();
    assert_eq!(stopped["session"]["state"], json!("stopped"));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupted_db_is_reported_never_fabricated() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap();
    let (dir, config) = daemon.stop().await;

    // Corrupt the DB header deterministically: garbage over the first page
    // makes the file "not a database" for any subsequent open. Late-exiting
    // connection tasks can checkpoint the WAL over our corruption, so wait
    // for handles to drain and verify the garbage actually persisted.
    let db = config.db_path();
    let corrupt = || {
        let _ = std::fs::remove_file(db.with_extension("db-wal"));
        let _ = std::fs::remove_file(db.with_extension("db-shm"));
        let mut bytes = std::fs::read(&db).unwrap();
        for b in bytes.iter_mut().take(512) {
            *b = 0xAB;
        }
        std::fs::write(&db, &bytes).unwrap();
    };
    for _ in 0..20 {
        corrupt();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let head = std::fs::read(&db).unwrap();
        if head[..512].iter().all(|&b| b == 0xAB) && !db.with_extension("db-wal").exists() {
            break;
        }
    }

    let daemon = TestDaemon::start_with(dir, config).await;
    let err = daemon
        .call(
            methods::REPOSITORY_INSPECT,
            &json!({"path": path_of(&repo)}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(err.code).unwrap(),
        json!("STATE_CORRUPTED")
    );

    // daemon.status still answers honestly.
    let status = daemon
        .call(methods::DAEMON_STATUS, &json!({}))
        .await
        .unwrap();
    assert_eq!(status["db_healthy"], json!(false));
    daemon.stop().await;
}

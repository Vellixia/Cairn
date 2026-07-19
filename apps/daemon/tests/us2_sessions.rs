//! T039: US2 session lifecycle over real IPC — idempotent start, stale
//! takeover semantics (lease-based, PID-aware), coexistence, scope isolation.

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::{test_config, TestDaemon};

fn path_of(repo: &FixtureRepo) -> String {
    repo.root().to_string_lossy().to_string()
}

async fn register(daemon: &TestDaemon, repo: &FixtureRepo) -> String {
    let reg = daemon
        .call(
            methods::REPOSITORY_REGISTER,
            &json!({"path": path_of(repo)}),
        )
        .await
        .unwrap();
    reg["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn instance() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn start_captures_start_snapshot_and_all_attributes() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    register(&daemon, &repo).await;

    let inst = instance();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "test-agent",
                    "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    assert_eq!(started["outcome"], json!("created"));
    assert!(
        started["resume_token"].is_string(),
        "token issued once on create"
    );
    let s = &started["session"];
    assert_eq!(s["state"], json!("active"));
    assert_eq!(s["agent_type"], json!("test-agent"));
    assert_eq!(s["agent_instance_id"].as_str().unwrap(), inst);
    assert!(!s["local_user"].as_str().unwrap().is_empty());
    assert_eq!(
        s["start_snapshot"]["snapshot_fp"], s["current_snapshot"]["snapshot_fp"],
        "unchanged repo: start == current (SC-002 anchor)"
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn double_start_same_instance_is_idempotent_without_new_token() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    register(&daemon, &repo).await;
    let inst = instance();
    let params = json!({"path": path_of(&repo), "agent_type": "a", "agent_instance_id": inst});

    let first = daemon.call(methods::SESSION_START, &params).await.unwrap();
    let second = daemon.call(methods::SESSION_START, &params).await.unwrap();

    assert_eq!(second["outcome"], json!("existing"));
    assert!(
        second["resume_token"].is_null(),
        "no new token on idempotent start (FR-034)"
    );
    assert_eq!(
        second["session"]["session_id"],
        first["session"]["session_id"]
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn expired_lease_with_unknown_pid_takes_over_via_lease_only() {
    // Lease-based staleness: no PID recorded => process_unknown; takeover
    // happens ONLY because the lease expired (analysis A1).
    let dir = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&dir);
    config.session.initial_lease_secs = 1; // expire almost immediately
    let daemon = TestDaemon::start_with(dir, config).await;

    let repo = FixtureRepo::new().unwrap();
    register(&daemon, &repo).await;
    let inst = instance();
    let params = json!({"path": path_of(&repo), "agent_type": "a", "agent_instance_id": inst});

    let first = daemon.call(methods::SESSION_START, &params).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let second = daemon.call(methods::SESSION_START, &params).await.unwrap();
    assert_eq!(second["outcome"], json!("takeover"));
    assert!(second["resume_token"].is_string());
    assert_ne!(
        second["session"]["session_id"],
        first["session"]["session_id"]
    );

    // Prior session interrupted with liveness detail recorded.
    let events = daemon.call(methods::EVENTS_LIST, &json!({})).await.unwrap();
    let interrupted = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event_type"] == "session.interrupted")
        .expect("interrupted event");
    assert_eq!(interrupted["payload"]["reason"], json!("stale_takeover"));
    assert_eq!(
        interrupted["payload"]["liveness_detail"],
        json!("process_unknown")
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unexpired_lease_with_live_pid_never_takes_over() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    register(&daemon, &repo).await;
    let inst = instance();
    // Record OUR pid: verifiably alive.
    let params = json!({"path": path_of(&repo), "agent_type": "a",
                        "agent_instance_id": inst, "agent_pid": std::process::id()});
    let first = daemon.call(methods::SESSION_START, &params).await.unwrap();
    let second = daemon.call(methods::SESSION_START, &params).await.unwrap();
    assert_eq!(second["outcome"], json!("existing"));
    assert_eq!(
        second["session"]["session_id"],
        first["session"]["session_id"]
    );
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn two_instances_coexist_and_stay_isolated() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    register(&daemon, &repo).await;

    let inst_a = instance();
    let inst_b = instance();
    let a = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "same-agent", "agent_instance_id": inst_a}),
        )
        .await
        .unwrap();
    let b = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "same-agent", "agent_instance_id": inst_b}),
        )
        .await
        .unwrap();
    assert_eq!(a["outcome"], json!("created"));
    assert_eq!(b["outcome"], json!("created"));
    assert_ne!(a["session"]["session_id"], b["session"]["session_id"]);

    // Adaptive resolution: no selector + two live sessions => ambiguous,
    // never a recency pick (FR-036).
    let get = daemon
        .call(methods::SESSION_GET, &json!({"path": path_of(&repo)}))
        .await
        .unwrap();
    assert_eq!(get["resolution"], json!("ambiguous"));
    assert_eq!(get["candidates"].as_array().unwrap().len(), 2);

    // Instance selector resolves to exactly its own session (scope isolation).
    let get_a = daemon
        .call(
            methods::SESSION_GET,
            &json!({"path": path_of(&repo), "agent_instance_id": inst_a}),
        )
        .await
        .unwrap();
    assert_eq!(get_a["resolution"], json!("single"));
    assert_eq!(get_a["session"]["session_id"], a["session"]["session_id"]);

    // Stopping A leaves B untouched.
    daemon
        .call(
            methods::SESSION_STOP,
            &json!({"path": path_of(&repo), "agent_instance_id": inst_a}),
        )
        .await
        .unwrap();
    let get_b = daemon
        .call(methods::SESSION_GET, &json!({"path": path_of(&repo)}))
        .await
        .unwrap();
    assert_eq!(get_b["resolution"], json!("single"));
    assert_eq!(get_b["session"]["session_id"], b["session"]["session_id"]);
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_records_event_with_final_snapshot() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let repo_id = register(&daemon, &repo).await;
    let inst = instance();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "a", "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let sid = started["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Mutate so the final snapshot differs from the start snapshot.
    repo.write("late.txt", "bye\n").unwrap();

    let stopped = daemon
        .call(methods::SESSION_STOP, &json!({"session_id": sid}))
        .await
        .unwrap();
    assert_eq!(stopped["session"]["state"], json!("stopped"));
    assert_ne!(
        stopped["session"]["current_snapshot"]["snapshot_fp"],
        started["session"]["start_snapshot"]["snapshot_fp"],
        "final snapshot reflects the late change"
    );

    let events = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": repo_id, "session_id": sid}),
        )
        .await
        .unwrap();
    let types: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"session.started"));
    assert!(types.contains(&"session.stopped"));
    daemon.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeat_extends_lease_and_rejects_bad_token() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    register(&daemon, &repo).await;
    let inst = instance();
    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path_of(&repo), "agent_type": "a", "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let sid = started["session"]["session_id"].as_str().unwrap();
    let token = started["resume_token"].as_str().unwrap();

    let hb = daemon
        .call(
            methods::SESSION_HEARTBEAT,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": token}),
        )
        .await
        .unwrap();
    assert_eq!(hb["state"], json!("active"));

    let bad = daemon
        .call(
            methods::SESSION_HEARTBEAT,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": "wrong"}),
        )
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(bad.code).unwrap(),
        json!("LEASE_MISMATCH")
    );
    daemon.stop().await;
}

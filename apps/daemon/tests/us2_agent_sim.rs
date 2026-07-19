//! T040: deterministic agent simulation — a scripted client drives
//! start → heartbeat → edit → reconcile → stop over real IPC and the
//! resulting event-type sequence must match a golden exactly
//! (constitution: deterministic agent simulations).

mod support;

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;
use support::TestDaemon;

#[tokio::test(flavor = "multi_thread")]
async fn scripted_session_produces_golden_event_sequence() {
    let daemon = TestDaemon::start().await;
    let repo = FixtureRepo::new().unwrap();
    let path = repo.root().to_string_lossy().to_string();
    let inst = uuid::Uuid::new_v4().to_string();

    // -- scripted agent begins --
    let reg = daemon
        .call(methods::REPOSITORY_REGISTER, &json!({"path": path}))
        .await
        .unwrap();
    let repo_id = reg["repository"]["repository_id"]
        .as_str()
        .unwrap()
        .to_string();

    let started = daemon
        .call(
            methods::SESSION_START,
            &json!({"path": path, "agent_type": "sim-agent", "agent_instance_id": inst}),
        )
        .await
        .unwrap();
    let sid = started["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let token = started["resume_token"].as_str().unwrap().to_string();
    let controls = daemon.watcher_controls();

    daemon
        .call(
            methods::SESSION_HEARTBEAT,
            &json!({"session_id": sid, "agent_instance_id": inst, "resume_token": token}),
        )
        .await
        .unwrap();

    // Session-start success is the readiness acknowledgement. An edit made
    // immediately after it returns must reach the event path; wait on the
    // explicit reconciliation acknowledgement, not a timing delay.
    let reconciled_before_edit = controls.reconciled_count();
    repo.write("agent-work.txt", "the agent wrote this\n")
        .unwrap();
    controls.wait_reconciled_after(reconciled_before_edit).await;
    let get = daemon
        .call(methods::SESSION_GET, &json!({"session_id": sid}))
        .await
        .unwrap();
    assert_ne!(
        get["session"]["start_snapshot"]["snapshot_fp"],
        get["session"]["current_snapshot"]["snapshot_fp"]
    );

    daemon
        .call(
            methods::SESSION_STOP,
            &json!({"session_id": sid, "resume_token": token}),
        )
        .await
        .unwrap();
    // -- scripted agent ends --

    let events = daemon
        .call(
            methods::EVENTS_LIST,
            &json!({"repository_id": repo_id, "limit": 100}),
        )
        .await
        .unwrap();
    let types: Vec<String> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_type"].as_str().unwrap().to_string())
        .collect();

    // Golden sequence. The watcher may legitimately observe the edit before
    // or while stop's final snapshot is being taken, so the state-change
    // block is fixed but the trailing snapshot.created (from stop's dedupe
    // miss) may or may not appear — the sequence below is the exact golden
    // for the scripted timing (stop after reconcile has landed).
    let golden = vec![
        "repository.registered",
        "worktree.registered",
        "snapshot.created", // start snapshot
        "session.started",
        "snapshot.created", // post-edit snapshot
        "repository.state_changed",
        "session.stopped",
    ];
    assert_eq!(types, golden, "event sequence diverged from golden");
    eprintln!(
        "feature001_scenario=2 result=pass event_total={} repository_state_changed={} session_started={} session_stopped={}",
        types.len(),
        types
            .iter()
            .filter(|kind| kind.as_str() == "repository.state_changed")
            .count(),
        types
            .iter()
            .filter(|kind| kind.as_str() == "session.started")
            .count(),
        types
            .iter()
            .filter(|kind| kind.as_str() == "session.stopped")
            .count(),
    );

    // Seq strictly increasing (total order).
    let seqs: Vec<i64> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_i64().unwrap())
        .collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
    daemon.stop().await;
}

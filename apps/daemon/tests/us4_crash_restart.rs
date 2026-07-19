//! T051: crash/restart harness — kill the REAL daemon binary at randomized
//! points, restart, and prove zero committed-event loss plus recovering
//! sessions (SC-005). Iterations scale via CAIRN_CRASH_ITERS; the dedicated
//! acceptance job sets exactly 100 while ordinary developer runs default to 8.

mod support;

use std::process::{Child, Command, Stdio};

use cairn_protocol::methods;
use fixtures_repositories::FixtureRepo;
use serde_json::json;

struct RealDaemon {
    child: Child,
    config: cairn_daemon::DaemonConfig,
}

impl RealDaemon {
    fn spawn(config: &cairn_daemon::DaemonConfig) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_cairnd"))
            .env("CAIRN_DATA_DIR", &config.data_dir)
            .env("CAIRN_SOCKET_PATH", &config.socket_path)
            .env("CAIRN_PIPE_NAME", &config.pipe_name)
            .env(
                "CAIRN_SWEEP_INTERVAL_MS",
                config.sweep_interval_ms.to_string(),
            )
            .env(
                "CAIRN_DEBOUNCE_QUIESCENCE_MS",
                config.debounce_quiescence_ms.to_string(),
            )
            .env("CAIRN_GRACE_SECS", config.session.grace_secs.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cairnd binary");
        Self {
            child,
            config: config.clone(),
        }
    }

    async fn client(&self) -> support::Ipc {
        for _ in 0..150 {
            if let Some(c) = try_connect(&self.config).await {
                return c;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        panic!("real daemon did not become reachable");
    }

    /// SIGKILL / TerminateProcess — no cleanup, no flush.
    fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
async fn try_connect(config: &cairn_daemon::DaemonConfig) -> Option<support::Ipc> {
    support::Ipc::connect_unix(&config.socket_path).await
}

#[cfg(windows)]
async fn try_connect(config: &cairn_daemon::DaemonConfig) -> Option<support::Ipc> {
    support::Ipc::connect_pipe(&config.pipe_name).await
}

fn iterations() -> usize {
    let count = std::env::var("CAIRN_CRASH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    assert!(count > 0, "CAIRN_CRASH_ITERS must be greater than zero");
    if let Some(expected) = std::env::var("CAIRN_CRASH_EXPECTED_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        assert_eq!(
            count, expected,
            "configured crash iterations must match the acceptance expectation"
        );
    }
    count
}

fn assert_recovery_contract(sessions: &serde_json::Value, iteration: &str) {
    for session in sessions["sessions"].as_array().unwrap() {
        let state = session["state"].as_str().unwrap();
        assert!(
            ["recovering", "stopped", "interrupted"].contains(&state),
            "{iteration}: pre-kill session must recover or terminate explicitly, got {state}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn randomized_kills_lose_no_committed_events_and_sessions_recover() {
    let dir = tempfile::TempDir::new().unwrap();
    let config = support::test_config(&dir);
    std::fs::create_dir_all(&config.data_dir).unwrap();
    let repo = FixtureRepo::new().unwrap();
    let path = repo.root().to_string_lossy().to_string();
    let iteration_count = iterations();

    let mut committed_event_ids: Vec<String> = Vec::new();
    let mut pending_recovery: Option<(String, String, String)> = None;

    for iter in 0..iteration_count {
        let daemon = RealDaemon::spawn(&config);
        let mut client = daemon.client().await;

        // --- verify nothing previously committed was lost (SC-005) ---
        let events = client
            .call(methods::EVENTS_LIST, &json!({"limit": 1000}))
            .await
            .expect("events.list after restart");
        let ids: std::collections::HashSet<String> = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        for known in &committed_event_ids {
            assert!(
                ids.contains(known),
                "iteration {iter}: committed event {known} was lost"
            );
        }

        // --- sessions that were live pre-kill are recovering, never lost ---
        let sessions = client
            .call(methods::SESSION_LIST, &json!({}))
            .await
            .expect("session.list");
        assert_recovery_contract(&sessions, &format!("iteration {iter}"));

        // Every session that was active at the forced kill is proved
        // recoverable, then explicitly stopped before the next kill. This is
        // stronger than merely observing the recovering projection.
        if let Some((session_id, instance_id, resume_token)) = pending_recovery.take() {
            let reattached = client
                .call(
                    methods::SESSION_REATTACH,
                    &json!({
                        "session_id": session_id,
                        "agent_instance_id": instance_id,
                        "resume_token": resume_token,
                    }),
                )
                .await
                .expect("killed active session reattaches");
            assert_eq!(reattached["session"]["state"], "active");
            let refreshed_token = reattached["resume_token"]
                .as_str()
                .expect("reattach returns a refreshed token");
            let stopped = client
                .call(
                    methods::SESSION_STOP,
                    &json!({"session_id": session_id, "resume_token": refreshed_token}),
                )
                .await
                .expect("reattached session stops explicitly");
            assert_eq!(stopped["session"]["state"], "stopped");
        }

        // --- generate new work, capturing what commits ---
        client
            .call(methods::REPOSITORY_REGISTER, &json!({"path": path}))
            .await
            .expect("register");
        let inst = uuid::Uuid::new_v4().to_string();
        let started = client
            .call(
                methods::SESSION_START,
                &json!({"path": path, "agent_type": "crash-sim", "agent_instance_id": inst}),
            )
            .await
            .expect("session starts only after watcher readiness");
        pending_recovery = Some((
            started["session"]["session_id"]
                .as_str()
                .unwrap()
                .to_string(),
            inst,
            started["resume_token"].as_str().unwrap().to_string(),
        ));
        repo.write(&format!("crash-{iter}.txt"), &format!("iteration {iter}\n"))
            .unwrap();
        let _ = client
            .call(methods::SNAPSHOT_CREATE, &json!({"path": path}))
            .await;

        // Snapshot the committed set: everything visible now IS committed.
        let events = client
            .call(methods::EVENTS_LIST, &json!({"limit": 1000}))
            .await
            .expect("events.list pre-kill");
        committed_event_ids = events["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();

        // Randomized-point kill: jitter before terminating so the daemon may
        // be mid-reconcile, mid-txn, or idle.
        let jitter = (iter * 37) % 120;
        tokio::time::sleep(std::time::Duration::from_millis(jitter as u64)).await;
        daemon.kill();
    }

    // Final restart: full history intact.
    let daemon = RealDaemon::spawn(&config);
    let mut client = daemon.client().await;
    let events = client
        .call(methods::EVENTS_LIST, &json!({"limit": 1000}))
        .await
        .unwrap();
    let ids: std::collections::HashSet<String> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect();
    for known in &committed_event_ids {
        assert!(
            ids.contains(known),
            "final restart lost committed event {known}"
        );
    }
    let sessions = client
        .call(methods::SESSION_LIST, &json!({}))
        .await
        .expect("session.list after final restart");
    assert_recovery_contract(&sessions, "final restart");
    if let Some((session_id, instance_id, resume_token)) = pending_recovery.take() {
        let reattached = client
            .call(
                methods::SESSION_REATTACH,
                &json!({
                    "session_id": session_id,
                    "agent_instance_id": instance_id,
                    "resume_token": resume_token,
                }),
            )
            .await
            .expect("final killed session reattaches");
        let refreshed_token = reattached["resume_token"].as_str().unwrap();
        client
            .call(
                methods::SESSION_STOP,
                &json!({"session_id": session_id, "resume_token": refreshed_token}),
            )
            .await
            .expect("final recovered session stops explicitly");
    }
    // Repo mutated during downtime is reflected on next snapshot (US4-4).
    repo.write("post-crash.txt", "written between restarts\n")
        .unwrap();
    let snap = client
        .call(methods::SNAPSHOT_CREATE, &json!({"path": path}))
        .await
        .unwrap();
    let authoritative = cairn_git::fingerprint::fingerprint_state(repo.root())
        .await
        .unwrap()
        .components
        .final_fingerprint();
    assert_eq!(
        snap["snapshot"]["snapshot_fp"].as_str().unwrap(),
        authoritative
    );
    eprintln!(
        "SC-005 acceptance: configured_iterations={iteration_count} completed_forced_kills={iteration_count} committed_event_loss=0 invalid_session_outcomes=0"
    );
    daemon.kill();
}

//! T027 — session boundaries, concurrency and fail-soft behaviour
//! (FR-009, FR-010, FR-015, FR-047, D16).

use cairn_e2e::Sandbox;
use serde_json::json;

#[test]
fn stop_is_a_turn_checkpoint_not_a_session_boundary() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "turn", "source": "startup" }),
    );
    s.settle_session_count(1);
    s.hook(
        "PostToolUse",
        json!({ "session_id": "turn", "tool_name": "Read", "tool_input": { "file_path": "README.md" } }),
    );
    s.hook("Stop", json!({ "session_id": "turn" }));
    s.settle_turn_checkpoint();

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(sessions.as_array().unwrap().len(), 1);
    assert_eq!(
        sessions[0]["status"], "active",
        "Stop must not end the session (D16)"
    );
    assert!(
        sessions[0]["last_turn_ended_at"].is_string(),
        "the turn checkpoint should be recorded"
    );

    // No durable handoff for a turn boundary (FR-032).
    let err = s.json_err(&["handoff", "show"]);
    assert_eq!(err["code"], "not_found");

    // A second turn continues the same session rather than starting a new one.
    s.hook(
        "PostToolUse",
        json!({ "session_id": "turn", "tool_name": "Edit", "tool_input": { "file_path": "a.txt" } }),
    );
    s.hook("Stop", json!({ "session_id": "turn" }));
    let after = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(
        after.as_array().unwrap().len(),
        1,
        "a turn must not fork the session"
    );
    assert_eq!(after[0]["status"], "active");
}

#[test]
fn session_end_completes_the_session_and_records_its_reason() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "ending", "source": "startup" }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "ending", "reason": "prompt_input_exit" }),
    );

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(sessions[0]["status"], "completed");
    assert!(s.handoff_after_close(&[])["trigger"] == "session_end");
}

#[test]
fn a_session_active_at_daemon_start_is_reconciled_and_can_resume() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "crashed", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "crashed", "tool_name": "Edit", "tool_input": { "file_path": "x.rs" } }),
    );

    s.settle_observations(1);

    // No liveness signal exists, so daemon start is the boundary (FR-009, D16).
    s.restart_daemon();

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(sessions[0]["status"], "interrupted");
    let handoff = s.handoff_after_close(&[]);
    assert_eq!(handoff["trigger"], "recovered");
    let recovered_id = handoff["id"].clone();

    // A later event resumes it; the handoff already written stands.
    //
    // Capture is fire-and-forget and a missed deadline is a *dropped* event by
    // contract (FR-015, FR-193), so the event is re-sent while waiting rather
    // than sent once and waited on. Under a loaded machine the first one can
    // legitimately be dropped, and asserting otherwise would be asserting a
    // guarantee Cairn deliberately does not make. What is under test is that
    // an event which arrives resumes the session.
    s.settle("the session to resume", |s| {
        s.hook(
            "PostToolUse",
            json!({ "session_id": "crashed", "tool_name": "Read", "tool_input": { "file_path": "README.md" } }),
        );
        s.json(&["session", "list"])["sessions"][0]["status"] == "active"
    });
    let resumed = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(
        resumed[0]["status"], "active",
        "a later event resumes the session"
    );
    assert_eq!(
        s.handoff_after_close(&[])["id"],
        recovered_id,
        "the recovery handoff is retained"
    );
}

#[test]
fn idle_time_is_reported_but_never_reclassifies() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "idle", "source": "startup" }),
    );
    s.settle_session_count(1);
    std::thread::sleep(std::time::Duration::from_millis(50));

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert!(sessions[0]["idle_seconds"].is_number());
    assert_eq!(
        sessions[0]["status"], "active",
        "idleness is not death (D16)"
    );
}

#[test]
fn concurrent_sessions_in_one_worktree_stay_distinct() {
    // FR-010: the worktree is scope, not the uniqueness key.
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "agent-a", "source": "startup" }),
    );
    s.hook(
        "SessionStart",
        json!({ "session_id": "agent-b", "source": "startup" }),
    );
    s.settle_session_count(2);

    s.hook(
        "PostToolUse",
        json!({ "session_id": "agent-a", "tool_name": "Edit", "tool_input": { "file_path": "a.rs" } }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "agent-b", "tool_name": "Edit", "tool_input": { "file_path": "b.rs" } }),
    );

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    let active: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["status"] == "active")
        .collect();
    assert_eq!(
        active.len(),
        2,
        "two agents in one checkout are two sessions"
    );

    // Ending one must not touch the other.
    s.hook(
        "SessionEnd",
        json!({ "session_id": "agent-a", "reason": "clear" }),
    );
    let after = s.json(&["session", "list"])["sessions"].clone();
    let still_active: Vec<_> = after
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["status"] == "active")
        .collect();
    assert_eq!(still_active.len(), 1);

    // Each session's observations went to its own session.
    let a_id = after
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["status"] == "completed")
        .unwrap()["id"]
        .clone();
    let handoff = s.handoff_after_close(&["--session", a_id.as_str().unwrap()]);
    let changed = handoff["changed_files"].as_array().unwrap();
    assert!(changed.iter().any(|f| f.as_str() == Some("a.rs")));
    assert!(
        !changed.iter().any(|f| f.as_str() == Some("b.rs")),
        "an observation was routed to the wrong session"
    );
}

#[test]
fn an_ambiguous_worktree_reports_rather_than_guessing() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "one", "source": "startup" }),
    );
    s.hook(
        "SessionStart",
        json!({ "session_id": "two", "source": "startup" }),
    );
    s.settle_session_count(2);

    let err = s.json_err(&["session", "show"]);
    assert_eq!(err["code"], "ambiguous_session");
    assert!(err["message"].as_str().unwrap().contains("--session"));
}

#[test]
fn hooks_exit_zero_and_never_fail_the_agent_when_cairn_is_unavailable() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "soft", "source": "startup" }),
    );

    // Point the hook at a socket nothing is listening on and a daemon binary
    // that does not exist: the worst case Cairn can be in.
    let dead_socket = std::env::temp_dir().join("cairn-nonexistent.sock");
    let out = std::process::Command::new(cairn_e2e::binary("cairn"))
        .args(["hook", "PostToolUse"])
        .current_dir(s.repo_path())
        .env("CAIRN_HOME", s.home.path())
        .env("CAIRN_SOCKET", &dead_socket)
        .env("CAIRND_BIN", "/nonexistent/cairnd")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(br#"{"session_id":"soft","tool_name":"Read"}"#)?;
            child.wait_with_output()
        })
        .expect("hook runs");

    assert_eq!(
        out.status.code(),
        Some(0),
        "a hook must always exit 0 (FR-015)"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).is_empty(),
        "a hook must not write errors the agent would surface"
    );
}

#[test]
fn a_daemon_restart_mid_session_loses_no_acknowledged_writes() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "durable", "source": "startup" }),
    );
    for i in 0..5 {
        s.hook(
            "PostToolUse",
            json!({
                "session_id": "durable",
                "tool_name": "Edit",
                "tool_input": { "file_path": format!("f{i}.rs") }
            }),
        );
    }
    // Capture is fire-and-forget, so wait for the writes before measuring them.
    s.settle_observations(5);
    let before = s.json(&["status"])["observation_count"].clone();
    s.restart_daemon();
    let after = s.json(&["status"])["observation_count"].clone();
    assert_eq!(
        before, after,
        "acknowledged writes must survive a restart (FR-047)"
    );
}

/// PIDs of daemons serving this sandbox's socket.
///
/// Identified by the `CAIRN_SOCKET` they were started with: macOS `lsof` does
/// not report a renamed Unix socket's path, so the environment is the reliable
/// signal.
fn daemons_for(s: &Sandbox) -> Vec<i32> {
    let socket = s.socket.display().to_string();
    let listed = std::process::Command::new("pgrep")
        .args(["-f", "cairnd"])
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|p| p.trim().parse::<i32>().ok())
        .filter(|pid| {
            let env = std::process::Command::new("ps")
                .args(["eww", "-p", &pid.to_string()])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            env.contains(&socket)
        })
        .collect()
}

#[test]
fn a_real_process_kill_loses_nothing_and_reconciles_the_session() {
    // M4 / FR-047: SIGKILL, not a graceful stop.
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "killed", "source": "startup" }),
    );
    for i in 0..8 {
        s.hook(
            "PostToolUse",
            json!({
                "session_id": "killed",
                "tool_name": "Edit",
                "tool_input": { "file_path": format!("k{i}.rs") }
            }),
        );
    }
    s.settle_observations(8);

    let victims = daemons_for(&s);
    assert!(
        !victims.is_empty(),
        "a daemon should be serving this sandbox"
    );
    for pid in &victims {
        unsafe { libc_kill(*pid, 9) };
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Nothing acknowledged was lost, and the database is intact.
    assert_eq!(
        s.json(&["status"])["observation_count"].as_i64(),
        Some(8),
        "acknowledged writes must survive SIGKILL"
    );
    assert_eq!(
        s.integrity_check(),
        "ok",
        "the store must survive SIGKILL intact"
    );

    // Daemon start is the boundary: the session is reconciled with a handoff.
    s.settle_session_status("interrupted");
    assert_eq!(s.handoff_after_close(&[])["trigger"], "recovered");
}

/// `kill(2)`, so the test can end a process the way a crash does.
unsafe fn libc_kill(pid: i32, sig: i32) {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    kill(pid, sig);
}

#[test]
fn repeated_concurrent_starts_leave_exactly_one_daemon() {
    // H2: superseded daemons must notice they no longer own the socket and go.
    let s = Sandbox::new();
    for _ in 0..3 {
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let repo = s.repo_path().to_path_buf();
                let home = s.home.path().to_path_buf();
                let socket = s.socket.clone();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new(cairn_e2e::binary("cairn"))
                        .args(["--json", "status"])
                        .current_dir(&repo)
                        .env("CAIRN_HOME", &home)
                        .env("CAIRN_SOCKET", &socket)
                        .env("CAIRND_BIN", cairn_e2e::binary("cairnd"))
                        .output();
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread");
        }
    }

    // The supervisor ticks every 2s; give it room to reap the losers.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    let mut count = daemons_for(&s).len();
    while std::time::Instant::now() < deadline && count > 1 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        count = daemons_for(&s).len();
    }
    assert_eq!(count, 1, "superseded daemons must exit, not linger (H2)");
}

#[test]
fn an_idle_daemon_exits_when_a_timeout_is_configured() {
    // H2: the advertised `--idle-timeout` does what it says.
    //
    // Deliberately no `Sandbox`: this is about one daemon's lifecycle, and a
    // sandbox's own daemon competing for the same socket would decide the
    // outcome by the ownership path instead of the idle path.
    let home = tempfile::TempDir::new().expect("home");
    let socket = std::env::temp_dir().join(format!(
        "cairn-idle-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let mut child = std::process::Command::new(cairn_e2e::binary("cairnd"))
        .args([
            "--socket",
            &socket.display().to_string(),
            "--idle-timeout",
            "1",
        ])
        .env("CAIRN_HOME", home.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("cairnd runs");

    // It comes up…
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline && !socket.exists() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(socket.exists(), "daemon should have bound its socket");

    // …and with no requests and no active session, it goes away on its own.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match child.try_wait().expect("wait") {
            Some(_) => break,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&socket);
                panic!("an idle daemon did not exit within its timeout");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    let _ = std::fs::remove_file(&socket);
}

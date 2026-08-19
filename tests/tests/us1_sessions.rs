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

// The two tests below identify and hard-kill `cairnd` processes by PID.
// Originally Unix-only (matched the process by its `CAIRN_SOCKET` env var
// and sent `SIGKILL`), they now share `cairn_sys` so they run on Windows
// too: `cairn_sys::daemons_for_socket` enumerates by named pipe, and
// `cairn_sys::kill` calls `TerminateProcess`, the nearest analogue of
// `SIGKILL` on Windows. The recovery logic these tests exercise
// (FR-047, H2) is plain, non-platform-specific code that other tests
// already drive, but covering the crash path on every platform is what
// the windows-support branch is about.

/// PIDs of daemons serving this sandbox's socket.
fn daemons_for(s: &Sandbox) -> Vec<i64> {
    cairn_sys::daemons_for_socket(&s.socket)
}

#[test]
fn a_real_process_kill_loses_nothing_and_reconciles_the_session() {
    // M4 / FR-047: a hard kill, not a graceful stop.
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
        assert!(cairn_sys::kill(*pid), "kill should succeed for {pid}");
    }
    // Give the OS time to reap and the socket to go quiet.
    for pid in &victims {
        assert!(
            cairn_sys::wait_for_exit(*pid, std::time::Duration::from_millis(1500)),
            "daemon {pid} should exit after a hard kill"
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Nothing acknowledged was lost, and the database is intact.
    assert_eq!(
        s.json(&["status"])["observation_count"].as_i64(),
        Some(8),
        "acknowledged writes must survive a hard kill"
    );
    assert_eq!(
        s.integrity_check(),
        "ok",
        "the store must survive a hard kill intact"
    );

    // Daemon start is the boundary: the session is reconciled with a handoff.
    s.settle_session_status("interrupted");
    assert_eq!(s.handoff_after_close(&[])["trigger"], "recovered");
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
    // Guarded, so the daemon `cairn` spawns below is stopped even when the
    // request it was spawned for fails.
    let socket = cairn_e2e::DaemonSocket::new();

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
    while std::time::Instant::now() < deadline && !cairn_e2e::daemon_listening(&socket) {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        cairn_e2e::daemon_listening(&socket),
        "daemon should have bound its socket"
    );

    // …and with no requests and no active session, it goes away on its own.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        match child.try_wait().expect("wait") {
            Some(_) => break,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                #[cfg(unix)]
                {
                    let _ = std::fs::remove_file(&socket);
                }
                panic!("an idle daemon did not exit within its timeout");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    // Named pipes leave no filesystem entry to clean up; only Unix does.
    #[cfg(unix)]
    {
        let _ = std::fs::remove_file(&socket);
    }
}

#[test]
fn a_second_session_end_on_an_already_completed_session_is_idempotent() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "double-end", "source": "startup" }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "double-end", "reason": "clear" }),
    );
    s.settle_session_status("completed");

    // A second SessionEnd for the same session must not error or create a
    // second handoff.
    s.hook(
        "SessionEnd",
        json!({ "session_id": "double-end", "reason": "clear" }),
    );
    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(sessions[0]["status"], "completed");
}

/// A `SessionStart` hook creates the session; a second one with the same key
/// rejoins it rather than forking (FR-006).
#[test]
fn a_session_start_with_an_existing_key_rejoins_rather_than_creating_a_second() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "rejoin", "source": "startup" }),
    );
    s.settle_session_count(1);

    // A second SessionStart with the same key must not create a new session.
    s.hook(
        "SessionStart",
        json!({ "session_id": "rejoin", "source": "startup" }),
    );
    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(
        sessions.as_array().unwrap().len(),
        1,
        "a second SessionStart with the same key must rejoin, not fork"
    );
    assert_eq!(sessions[0]["status"], "active");
}

#[test]
fn hook_events_for_a_never_started_session_are_dropped_cleanly() {
    let s = Sandbox::new();
    // Fire a PostToolUse for a session that was never started. The hook
    // must exit 0 and not crash the daemon.
    let result = s.hook(
        "PostToolUse",
        json!({ "session_id": "ghost", "tool_name": "Read", "tool_input": { "file_path": "x" } }),
    );
    assert!(result.ok(), "hook for a ghost session must exit 0");

    // The daemon should still be healthy.
    let status = s.json(&["status"]);
    assert_eq!(status["daemon"], "running");
}

#[test]
fn a_stop_turn_checkpoint_persists_across_a_daemon_restart() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "turn-persist", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "turn-persist", "tool_name": "Read", "tool_input": { "file_path": "README.md" } }),
    );
    s.hook("Stop", json!({ "session_id": "turn-persist" }));

    // Stop is fire-and-forget; wait for the turn checkpoint to land.
    s.settle("turn checkpoint recorded", |s| {
        s.json(&["session", "list"])["sessions"][0]["last_turn_ended_at"].is_string()
    });

    let before = s.json(&["session", "list"])["sessions"].clone();
    assert!(before[0]["last_turn_ended_at"].is_string());

    s.restart_daemon();

    // FR-009: daemon start reconciles active sessions to interrupted.
    // The turn checkpoint (last_turn_ended_at) should survive.
    let after = s.json(&["session", "list"])["sessions"].clone();
    assert!(
        after[0]["last_turn_ended_at"].is_string(),
        "the turn checkpoint should survive a daemon restart"
    );
    assert_eq!(after[0]["status"], "interrupted");

    // A new event resumes it to active.
    s.hook(
        "PostToolUse",
        json!({ "session_id": "turn-persist", "tool_name": "Read", "tool_input": { "file_path": "README.md" } }),
    );
    s.settle_session_status("active");
}

/// Writing memory without naming a session opens **one** session, not one per
/// write.
///
/// `ensure_session_for_memory` minted a fresh random key on every call, and
/// `start_session` is idempotent per key — so each keyless write created
/// another session. Two `cairn memory add` calls left two sessions, and the
/// third, along with every `cairn context` after it, failed with
/// `ambiguous_session`. The command that opened the sessions was the command
/// that broke the worktree.
#[test]
fn keyless_memory_writes_share_one_session() {
    let s = Sandbox::new();

    for i in 0..4 {
        let r = s.cairn(&[
            "memory",
            "add",
            &format!("A durable fact number {i}."),
            "--type",
            "fact",
            "--scope",
            "project",
        ]);
        assert!(
            r.ok(),
            "keyless write {i} failed: {} {}",
            r.stdout,
            r.stderr
        );
    }

    let sessions = s.json(&["session", "list"]);
    let active = sessions["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .filter(|x| x["status"] == "active")
        .count();
    assert_eq!(
        active, 1,
        "four keyless writes left {active} active sessions: {sessions}"
    );

    // And the worktree still answers, which is the property the duplicates took
    // away.
    let c = s.cairn(&["context"]);
    assert!(
        c.ok(),
        "context broke after keyless writes: {} {}",
        c.stdout,
        c.stderr
    );
}

/// The same, in parallel: concurrent keyless writes converge on one session.
///
/// `start_session` reads by key and then inserts, and `sessions` carries a
/// unique index on `(project_id, agent_session_key)`. Concurrent callers all
/// see nothing and all insert; one wins. Starting a session that already exists
/// is the idempotency the key contract promises, so the losers must read the
/// winner's session rather than failing the caller's write.
#[test]
fn concurrent_keyless_writes_converge_on_one_session() {
    let s = Sandbox::new();

    let outcomes: Vec<cairn_e2e::CliResult> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..12)
            .map(|i| {
                let s = &s;
                scope.spawn(move || {
                    s.cairn(&[
                        "memory",
                        "add",
                        &format!("Parallel fact number {i}."),
                        "--type",
                        "fact",
                        "--scope",
                        "project",
                    ])
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });

    let failed: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.ok())
        .map(|o| format!("exit {} · out {:?}", o.code, o.stdout))
        .collect();
    assert!(
        failed.is_empty(),
        "{} of 12 failed: {failed:?}",
        failed.len()
    );

    let sessions = s.json(&["session", "list"]);
    let active = sessions["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .filter(|x| x["status"] == "active")
        .count();
    assert_eq!(
        active, 1,
        "twelve concurrent keyless writes left {active} active sessions"
    );
}

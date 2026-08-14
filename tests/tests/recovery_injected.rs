//! T058 — the boundary fails, and the session is still finished (SC-129).
//!
//! Tested separately from the nominal performance suite on purpose. SC-128 is
//! "the normal case is fast enough"; this is "the abnormal case still
//! converges", and the two failing for the same reason would be a coincidence
//! that hid a real defect.
//!
//! Three induced conditions, each a real thing that happens on a developer's
//! machine: the handler runs out of time under load, the handler dies before
//! it can report, and the daemon is not there at all. In every one of them,
//! the agent's session must be unaffected — the hook exits zero and says
//! nothing — and the *Cairn* session must still end up reconciled with a
//! durable handoff, by one of the recovery paths that exist for exactly this.

use cairn_e2e::{binary, Sandbox};
use serde_json::{json, Value};

/// Do some work worth summarizing, so a handoff has something to say.
fn work(s: &Sandbox, key: &str) {
    s.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    s.settle_session_count(1);
    s.write_file("src/pool.rs", "pub struct Pool;\n");
    s.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/pool.rs" }
        }),
    );
    s.settle_observations(1);
}

fn sessions(s: &Sandbox) -> Vec<Value> {
    s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Every session reconciled, and every one of them holding a handoff.
fn assert_every_session_finished(s: &Sandbox, after: &str) {
    s.settle(&format!("reconciliation {after}"), |s| {
        sessions(s).iter().all(|x| x["status"] != "active")
    });
    let all = sessions(s);
    assert!(!all.is_empty(), "no session exists {after}");
    for session in &all {
        let id = session["id"].as_str().expect("a session id");
        let handoff = s.handoff_after_close(&["--session", id]);
        assert!(
            handoff["next_step"].is_string(),
            "a session was left permanently without a handoff {after}: {session}"
        );
    }
}

#[test]
fn a_handler_that_runs_out_of_time_does_not_lose_the_session() {
    // The deadline exists so a slow Cairn never holds up an agent (FR-193).
    // What it must not do is lose the boundary: the session still reaches a
    // terminal state with a handoff, by the deterministic route.
    let s = Sandbox::new();
    work(&s, "timeout-1");

    // One millisecond: no request can complete inside it.
    s.set_deadlines(1, 1);
    let out = s.hook(
        "SessionEnd",
        json!({ "session_id": "timeout-1", "reason": "clear" }),
    );
    assert_eq!(out.code, 0, "the hook failed the agent's session");
    assert!(
        out.stderr.trim().is_empty(),
        "the hook disrupted the agent with output: {}",
        out.stderr
    );

    // Back to a workable deadline, and the deterministic boundary reconciles
    // what the timed-out handler could not deliver (FR-009, D16).
    s.set_deadlines(5000, 15000);
    s.restart_daemon();
    assert_every_session_finished(&s, "after a handler timeout");
}

#[test]
fn a_handler_killed_mid_flight_does_not_lose_the_session() {
    // The hook is a separate process, and a developer's machine kills
    // processes: an editor restart, a shell exit, an out-of-memory sweep.
    let s = Sandbox::new();
    work(&s, "crash-1");

    use std::io::Write;
    use std::process::Stdio;
    let mut child = std::process::Command::new(binary("cairn"))
        .args(["hook", "SessionEnd"])
        .current_dir(s.repo_path())
        .env("CAIRN_HOME", s.cairn_home())
        // The sandbox's own daemon — see the note in `perf_capture`: a
        // hand-spelled path names nothing on Windows, and the hook's
        // fail-soft path would hide that.
        .env("CAIRN_SOCKET", &s.socket)
        .env("CAIRND_BIN", binary("cairnd"))
        .env("HOME", s.fake_home())
        .env("XDG_CONFIG_HOME", s.fake_home().join(".config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook starts");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(
            json!({
                "session_id": "crash-1",
                "reason": "clear",
                "cwd": s.repo_dir().display().to_string()
            })
            .to_string()
            .as_bytes(),
        )
        .expect("write payload");

    // Killed before it can report anything back to the agent.
    let _ = child.kill();
    let _ = child.wait();

    // Whether the seal landed before the kill or not, the session converges:
    // a sealed boundary is swept, and an unsealed one is reconciled at daemon
    // start. Both routes exist precisely because the caller can vanish.
    s.restart_daemon();
    assert_every_session_finished(&s, "after a killed handler");
}

#[test]
fn a_boundary_with_no_daemon_at_all_does_not_lose_the_session() {
    // The daemon is not running, cannot be started, and the boundary arrives
    // anyway. The hook must be silent and successful, and the work must not be
    // orphaned.
    let s = Sandbox::new();
    work(&s, "gone-1");
    s.stop_daemon();

    use std::io::Write;
    use std::process::Stdio;
    // A well-formed endpoint with nothing listening on it, on either
    // platform — the case a developer actually hits. A malformed one would
    // prove a different thing, and only by accident.
    let dead_socket = cairn_e2e::sandbox_socket();
    let mut child = std::process::Command::new(binary("cairn"))
        .args(["hook", "SessionEnd"])
        .current_dir(s.repo_path())
        .env("CAIRN_HOME", s.cairn_home())
        .env("CAIRN_SOCKET", &dead_socket)
        // The worst case: not even a daemon binary to start.
        .env("CAIRND_BIN", "/nonexistent/cairnd")
        .env("HOME", s.fake_home())
        .env("XDG_CONFIG_HOME", s.fake_home().join(".config"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook starts");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(
            json!({
                "session_id": "gone-1",
                "reason": "clear",
                "cwd": s.repo_dir().display().to_string()
            })
            .to_string()
            .as_bytes(),
        )
        .expect("write payload");
    let out = child.wait_with_output().expect("hook completes");

    assert_eq!(
        out.status.code(),
        Some(0),
        "an unreachable daemon failed the agent's session"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).trim().is_empty(),
        "an unreachable daemon disrupted the agent: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The daemon comes back — a new session, an upgrade, the next command —
    // and start-time reconciliation finishes what nobody was there to
    // finish (FR-009).
    s.cairn(&["daemon", "start"]);
    assert_every_session_finished(&s, "after the daemon was unavailable");
}

#[test]
fn every_induced_failure_leaves_zero_sessions_without_a_handoff() {
    // SC-129 stated as one number over all three conditions, because "zero
    // sessions left permanently without one" is the claim, not "each case
    // works when run alone".
    let s = Sandbox::new();

    // Condition one, in its own session.
    work(&s, "all-1");
    s.set_deadlines(1, 1);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "all-1", "reason": "clear" }),
    );
    s.set_deadlines(5000, 15000);

    // Condition two: a boundary that never arrives at all, because the daemon
    // was not there to receive it.
    s.stop_daemon();
    s.cairn(&["daemon", "start"]);
    s.hook(
        "SessionStart",
        json!({ "session_id": "all-2", "source": "startup" }),
    );
    s.settle_session_count(2);
    s.stop_daemon();
    s.cairn(&["daemon", "start"]);

    // Condition three: an ordinary close, to show the recovery paths did not
    // break the path that works.
    s.hook(
        "SessionStart",
        json!({ "session_id": "all-3", "source": "startup" }),
    );
    s.settle_session_count(3);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "all-3", "reason": "clear" }),
    );

    s.restart_daemon();
    assert_every_session_finished(&s, "across all three induced conditions");
    assert_eq!(
        sessions(&s).len(),
        3,
        "the recovery paths invented or merged a session"
    );

    // And the daemon reports no outstanding debt at the end (FR-240).
    let status = s.json(&["status"]);
    assert_eq!(status["sessions_awaiting_handoff"], 0, "{status}");
    assert_eq!(
        status["handoff_synthesis_failures"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        0,
        "a synthesis failure was left unresolved: {status}"
    );
}

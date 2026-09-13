//! T065 — session-end work inside the vendor's own budget (FR-208, SC-128).
//!
//! Codex documents what it will wait for at a session boundary:
//! `SESSION_END_DEFAULT_TIMEOUT_SEC = 1`, `SESSION_END_MAX_TIMEOUT_SEC = 3`
//! (D31). Cairn's session-end work has to fit inside the *default*, because a
//! developer who has not raised the timeout is the normal case, and exceeding
//! it means the agent kills the handler mid-boundary.
//!
//! This measures what a vendor actually waits for: the hook process, from
//! spawn to exit, at a real session close. That is where the sealed boundary
//! earns its keep — the seal is acknowledged and the handoff is synthesized
//! after the hook returns (D22, FR-240), so the vendor's budget buys a durable
//! termination record rather than a summarization.
//!
//! `capability::budget_demonstrated` claims this is true of this build. This
//! benchmark is that claim's proof, and asserts it directly.

use cairn_e2e::Sandbox;
use serde_json::json;
use std::time::Instant;

/// SC-128 requires at least 100 boundaries per agent that imposes a deadline.
const BOUNDARIES: usize = 100;

/// Codex's own budget, from `codex-rs/hooks/src/events/session_end.rs`.
const DEFAULT_BUDGET_MS: f64 = 1_000.0;
const MAX_BUDGET_MS: f64 = 3_000.0;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

#[test]
fn codex_session_end_fits_inside_the_vendors_own_budget() {
    let s = Sandbox::new();
    s.install_agent("codex");
    s.must(&["init"]);
    s.must(&["connect", "codex", "--yes"]);

    // The production deadlines, not the relaxed ones the rest of the suite
    // runs under: a budget claim is only meaningful at the values that ship.
    s.set_deadlines(250, 1500);

    let mut samples: Vec<f64> = Vec::with_capacity(BOUNDARIES);
    for i in 0..BOUNDARIES {
        let key = format!("close-{i}");
        s.hook_as(
            "codex",
            "SessionStart",
            json!({ "session_id": key, "source": "startup" }),
        );
        // Real work in the session, so the boundary has something to seal and
        // the handoff has something to summarize. A boundary measured on an
        // empty session would be measuring nothing.
        s.hook_as(
            "codex",
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "apply_patch",
                "tool_input": { "file_path": format!("src/f{i}.rs") },
                "tool_response": { "exit_code": 0 }
            }),
        );

        let started = Instant::now();
        let out = s.hook_as(
            "codex",
            "SessionEnd",
            json!({ "session_id": key, "reason": "other" }),
        );
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(out.code, 0, "a session-end hook failed the agent's session");
    }

    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let median = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let max = *samples.last().expect("samples");
    let over_default = samples.iter().filter(|v| **v >= DEFAULT_BUDGET_MS).count();

    println!(
        "SC-128 ({BOUNDARIES} Codex session-end boundaries, {} build)\n  \
         median {median:.1} ms  p95 {p95:.1} ms  max {max:.1} ms  \
         over the {DEFAULT_BUDGET_MS:.0} ms default: {over_default}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );

    // Every boundary landed: the measurement is of real closes, not of a hook
    // that returned early because nothing happened.
    //
    // **Sized to a hundred reconciliations on the slowest runner, and it was
    // not.** Thirty seconds allots three hundred milliseconds to each of the
    // hundred boundaries, and the Windows runner — debug build, slowest in the
    // matrix — recorded a single boundary at 1524 ms in the very run that timed
    // out here, while its median stayed at 24 ms and its p95 at 33 ms. The
    // budget SC-128 asserts was met; the wait for the work to settle afterwards
    // was not long enough for the machine it ran on. Raising it changes no
    // claim: the assertions below still hold the shipped build to the vendor's
    // own timeout, and a reconciliation that genuinely stalls still fails here.
    //
    // The shortfall comes with the failure, because "none reconciled" and
    // "ninety-nine reconciled" are different defects and a bare timeout cannot
    // tell them apart.
    let reconciled = |s: &Sandbox| -> (usize, usize) {
        let sessions = s.json(&["session", "list"])["sessions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let done = sessions.iter().filter(|x| x["status"] != "active").count();
        (sessions.len(), done)
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline {
        let (seen, done) = reconciled(&s);
        if seen >= BOUNDARIES && done == seen {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let (seen, done) = reconciled(&s);
    assert!(
        seen >= BOUNDARIES && done == seen,
        "only {done} of {seen} sessions reconciled, against {BOUNDARIES} \
         boundaries driven: the latency above was measured over closes whose \
         work had not finished"
    );

    // And every one of them is owed nothing.
    let debt = |s: &Sandbox| -> i64 {
        s.json(&["status"])["sessions_awaiting_handoff"]
            .as_i64()
            .unwrap_or(-1)
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    while std::time::Instant::now() < deadline && debt(&s) != 0 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(
        debt(&s),
        0,
        "{} boundaries are still owed a handoff after three minutes",
        debt(&s)
    );

    if cfg!(debug_assertions) {
        println!(
            "  (debug build: an unoptimised binary pays several extra milliseconds per \
             spawn; run `cargo test --release` to assert the budget)"
        );
        return;
    }

    assert_eq!(
        over_default, 0,
        "{over_default} of {BOUNDARIES} boundaries reached Codex's {DEFAULT_BUDGET_MS:.0} ms \
         default timeout"
    );
    assert!(
        max < MAX_BUDGET_MS,
        "a boundary took {max:.1} ms, past Codex's {MAX_BUDGET_MS:.0} ms maximum"
    );
    // The claim the level derivation makes about this build (FR-208).
    assert!(
        cairn_integrate::capability::budget_demonstrated(cairn_integrate::AgentId::Codex),
        "the derivation claims this budget is undemonstrated while the benchmark passes"
    );
}

#[test]
fn the_seal_is_what_the_vendor_waits_for_not_the_handoff() {
    // D22, FR-240: the boundary acknowledges once termination is durably
    // recorded, and synthesis happens after. That is the design that makes a
    // one-second budget survivable without giving up the completion guarantee,
    // and this is the assertion that it is what actually happens.
    let s = Sandbox::new();
    s.install_agent("codex");
    s.must(&["init"]);
    s.must(&["connect", "codex", "--yes"]);
    s.set_deadlines(250, 1500);

    s.hook_as(
        "codex",
        "SessionStart",
        json!({ "session_id": "seal-1", "source": "startup" }),
    );
    s.settle_session_count(1);
    for i in 0..20 {
        s.hook_as(
            "codex",
            "PostToolUse",
            json!({
                "session_id": "seal-1",
                "tool_name": "apply_patch",
                "tool_input": { "file_path": format!("src/seal{i}.rs") },
                "tool_response": { "exit_code": 0 }
            }),
        );
    }
    s.settle_observations(20);

    let started = Instant::now();
    s.hook_as(
        "codex",
        "SessionEnd",
        json!({ "session_id": "seal-1", "reason": "other" }),
    );
    let acknowledged = started.elapsed().as_secs_f64() * 1000.0;

    // Termination is already durable when the hook returns.
    let sessions = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_ne!(
        sessions[0]["status"], "active",
        "the boundary returned before termination was recorded"
    );

    // The handoff arrives afterwards, within the documented bound.
    let id = sessions[0]["id"].as_str().expect("a session id");
    let handoff = s.handoff_after_close(&["--session", id]);
    assert!(handoff["next_step"].is_string());
    assert_eq!(handoff["trigger"], "session_end");

    println!("  seal acknowledged in {acknowledged:.1} ms with 20 observations to summarize");
}

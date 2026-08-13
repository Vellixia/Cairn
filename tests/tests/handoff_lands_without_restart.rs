//! T057 — the sealed close actually lands (SC-136, FR-240, D22).
//!
//! SC-128 and SC-136 measure different things and are deliberately separate:
//! SC-128 is whether the *acknowledgment* fits inside the vendor's handler
//! budget, and SC-136 is whether the durable handoff then arrives **without a
//! daemon restart**. A close that acknowledges quickly and silently loses the
//! handoff would pass the first and fail the second, which is exactly the
//! failure this file exists to catch.

use cairn_e2e::Sandbox;
use serde_json::json;
use std::collections::BTreeSet;

/// The documented bound: a recoverable boundary has its handoff inside this
/// interval on a running daemon.
const BOUND: std::time::Duration = std::time::Duration::from_secs(5);

fn session_ids(s: &Sandbox) -> BTreeSet<String> {
    s.json(&["session", "list", "--json"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x["id"].as_str().map(str::to_string))
        .collect()
}

/// Drive one complete session through the hook path and seal it, returning the
/// Cairn session id it created.
fn sealed_session(s: &Sandbox, key: &str) -> String {
    let before = session_ids(s);
    s.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    let id = session_ids(s)
        .difference(&before)
        .next()
        .cloned()
        .unwrap_or_else(|| panic!("no session was created for {key}"));

    s.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/lib.rs" }
        }),
    );
    // The hook path never waits: this is the budgeted boundary.
    s.hook(
        "SessionEnd",
        json!({ "session_id": key, "reason": "other" }),
    );
    id
}

/// Poll until the session has its durable handoff, or the bound passes.
///
/// Deliberately tolerant of `not_found` while polling: the whole point of the
/// sealed close is that the boundary is acknowledged *before* the handoff
/// exists, so its absence for a moment is the expected state, not a failure.
fn handoff_within_bound(s: &Sandbox, id: &str) -> bool {
    handoff_delay(s, id).is_some()
}

/// How long after the seal the handoff became readable, or `None` if it never
/// did inside the bound.
///
/// The polling interval floors what this can resolve, which is fine: the
/// number it feeds is a documented characteristic, not a budget.
fn handoff_delay(s: &Sandbox, id: &str) -> Option<f64> {
    let started = std::time::Instant::now();
    let deadline = started + BOUND;
    loop {
        let result = s.cairn(&["--json", "handoff", "show", "--session", id]);
        if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(&result.stdout) {
            if envelope["ok"] == true
                && envelope["data"]
                    .get("handoff")
                    .map(|h| !h.is_null())
                    .unwrap_or(false)
            {
                return Some(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() as f64) * p).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

fn status_of(s: &Sandbox, id: &str) -> String {
    s.json(&["session", "list", "--json"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .find(|x| x["id"] == id)
        .and_then(|x| x["status"].as_str().map(str::to_string))
        .unwrap_or_default()
}

#[test]
fn a_sealed_close_lands_its_handoff_without_a_restart() {
    // SC-136: the daemon runs throughout and is never restarted.
    let s = Sandbox::new();
    s.must(&["init"]);
    s.write_file("src/lib.rs", "pub fn work() {}\n");

    let id = sealed_session(&s, "sealed-1");

    assert!(
        handoff_within_bound(&s, &id),
        "the sealed boundary never produced its durable handoff"
    );
    // And the session is terminal, not merely quiet.
    assert_ne!(
        status_of(&s, &id),
        "active",
        "the seal did not terminate it"
    );
}

#[test]
fn every_sealed_close_in_a_batch_lands() {
    // The volume half of SC-136: at least 100 sealed closes, all landing
    // their handoff inside the bound, with the daemon running throughout.
    let s = Sandbox::new();
    s.must(&["init"]);
    s.write_file("src/lib.rs", "pub fn work() {}\n");

    const BOUNDARIES: usize = 100;
    let ids: Vec<String> = (0..BOUNDARIES)
        .map(|i| sealed_session(&s, &format!("batch-{i}")))
        .collect();

    let mut delays: Vec<f64> = Vec::with_capacity(BOUNDARIES);
    for id in &ids {
        if let Some(ms) = handoff_delay(&s, id) {
            delays.push(ms);
        }
    }
    assert_eq!(
        delays.len(),
        BOUNDARIES,
        "{} of {BOUNDARIES} sealed boundaries never produced a handoff",
        BOUNDARIES - delays.len()
    );

    // Reported for the record (SC-136, quickstart §Measurements on record):
    // how long after the acknowledgment the durable handoff became readable.
    // Not a budget — the bound above is the assertion — but the number a
    // developer experiences as "how soon can I read it".
    delays.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    println!(
        "SC-136 handoff after seal ({BOUNDARIES} boundaries)  \
         p50 {:.0} ms  p99 {:.0} ms  max {:.0} ms",
        percentile(&delays, 0.50),
        percentile(&delays, 0.99),
        delays.last().copied().unwrap_or_default(),
    );
}

#[test]
fn nothing_is_left_owed_after_the_boundaries_land() {
    // FR-240 clause 3: a terminal session never sits silently owing a handoff,
    // so `cairn status` reports the debt and it reaches zero.
    let s = Sandbox::new();
    s.must(&["init"]);
    s.write_file("src/lib.rs", "pub fn work() {}\n");

    let ids: Vec<String> = (0..5)
        .map(|i| sealed_session(&s, &format!("owed-{i}")))
        .collect();
    for id in &ids {
        assert!(handoff_within_bound(&s, id));
    }

    let status = s.json(&["status", "--json"]);
    assert_eq!(
        status["sessions_awaiting_handoff"], 0,
        "a boundary is still owed after every handoff landed"
    );
    assert!(
        status["handoff_synthesis_failures"]
            .as_array()
            .map(|a| a.is_empty())
            .unwrap_or(true),
        "a synthesis failure was reported where none happened"
    );
}

#[test]
fn the_command_line_close_still_waits_for_its_handoff() {
    // D22: `cairn session end` keeps Feature 001's behavior — nothing holds a
    // deadline over it, so it returns with the handoff already written.
    let s = Sandbox::new();
    s.must(&["init"]);
    s.write_file("src/lib.rs", "pub fn work() {}\n");

    s.hook(
        "SessionStart",
        json!({ "session_id": "cli-close", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "cli-close",
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/lib.rs" }
        }),
    );

    let v = s.json(&["session", "end", "--status", "completed", "--json"]);
    assert!(
        v.get("handoff").map(|h| !h.is_null()).unwrap_or(false),
        "the waiting close returned without its handoff: {v}"
    );
}

#[test]
fn a_quiescence_before_the_close_leaves_the_session_active() {
    // The Feature 001 guarantee the sealed close must not disturb: `Stop` is a
    // checkpoint, not a boundary (FR-032, FR-230).
    let s = Sandbox::new();
    s.must(&["init"]);
    let before = session_ids(&s);
    s.hook(
        "SessionStart",
        json!({ "session_id": "quiesce-then-close", "source": "startup" }),
    );
    let id = session_ids(&s)
        .difference(&before)
        .next()
        .cloned()
        .expect("a session");
    s.hook("Stop", json!({ "session_id": "quiesce-then-close" }));

    assert_eq!(status_of(&s, &id), "active", "quiescence ended the session");
}

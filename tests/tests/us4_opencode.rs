//! T075 — OpenCode never reaches FULL by being forgotten (US4 #6, US4 #7,
//! FR-229, SC-131).
//!
//! OpenCode signals no session end at all. Cairn still reaches a terminal
//! state for its sessions, by two routes that both exist for Feature 001's own
//! reasons: the inactivity timeout, and daemon-start reconciliation of
//! sessions a previous run left active.
//!
//! Neither is evidence that anything finished. A session that timed out may
//! have been abandoned mid-task; a session reconciled at daemon start ended
//! because the machine was rebooted. Reporting either as a completion would
//! tell a developer that their work was captured to a boundary when it was
//! merely swept up — and the handoff would say the same. So both routes must
//! keep working, and neither may raise the reported level.

use cairn_e2e::Sandbox;
use serde_json::Value;

fn opencode(s: &Sandbox) -> Value {
    s.json(&["agents"])["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["agent"] == "opencode")
        .expect("opencode is detected")
}

/// Everything the report must say, whatever route closed the session.
fn assert_below_full_and_honest(agent: &Value, after: &str) {
    assert_ne!(
        agent["level"], "full",
        "OpenCode reached FULL {after}: {agent}"
    );
    assert_eq!(
        agent["completion_guarantee"], "not_demonstrated",
        "the completion guarantee was claimed {after}"
    );
    let missing = agent["missing_behaviors"].to_string();
    assert!(
        missing.contains("automatic session completion"),
        "the missing behavior was not named {after}: {missing}"
    );
    // FR-229: the report says how sessions here actually end.
    let note = agent["session_completion"].as_str().unwrap_or_default();
    assert!(
        note.contains("inactivity"),
        "the report does not say sessions are closed by inactivity {after}: {note:?}"
    );
    assert!(
        note.contains("rather than completed"),
        "the report does not distinguish a timeout from a completion {after}: {note:?}"
    );
    assert!(
        agent["capabilities"]["lifecycle_session_close"]["availability"] == "absent",
        "OpenCode's absent session-close capability changed {after}"
    );
}

#[test]
fn idle_reaper_never_grants_full() {
    let s = Sandbox::new();
    s.install_agent("opencode");
    s.must(&["init"]);
    s.must(&["connect", "opencode", "--yes"]);

    // Before anything has happened.
    assert_below_full_and_honest(&opencode(&s), "before any session");

    // An ordinary OpenCode session: it opens, works, and goes quiet. Every
    // signal the vendor actually sends.
    s.hook_as(
        "opencode",
        "session.created",
        serde_json::json!({ "sessionID": "ses_full_attempt" }),
    );
    s.settle_session_count(1);
    s.hook_as(
        "opencode",
        "tool.execute.after",
        serde_json::json!({
            "sessionID": "ses_full_attempt",
            "tool": "edit",
            "args": { "file_path": "src/pool.rs" },
            "output": { "exit_code": 0 }
        }),
    );
    s.hook_as(
        "opencode",
        "session.idle",
        serde_json::json!({ "sessionID": "ses_full_attempt" }),
    );
    s.settle_turn_checkpoint();
    assert_below_full_and_honest(&opencode(&s), "after a complete session went quiet");

    // Daemon-start reconciliation — the second route to a terminal state
    // (FR-009, D16). The session reaches a terminal state and gets a handoff,
    // and the level is unmoved.
    s.restart_daemon();
    s.settle("the session to be reconciled", |s| {
        s.json(&["session", "list"])["sessions"][0]["status"].as_str() != Some("active")
    });
    let sessions = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        sessions.iter().all(|x| x["status"] != "active"),
        "reconciliation left a session active: {sessions:?}"
    );
    assert_below_full_and_honest(&opencode(&s), "after daemon-start reconciliation");

    // And the recovery genuinely happened: this is a backstop that works, not
    // a capability that was withheld by disabling it (FR-229).
    let id = sessions[0]["id"].as_str().expect("a session id");
    let handoff = s.handoff_after_close(&["--session", id]);
    assert!(
        handoff["next_step"].is_string(),
        "reconciliation did not leave a durable handoff: {handoff}"
    );
}

#[test]
fn opencode_reports_what_it_does_provide_rather_than_only_what_it_lacks() {
    // US4 #7: below FULL is not "broken". The capture that does work is
    // reported as working, and the conditional entries name their conditions.
    let s = Sandbox::new();
    s.install_agent("opencode");
    s.must(&["init"]);
    s.must(&["connect", "opencode", "--yes"]);

    let agent = opencode(&s);
    assert_eq!(agent["level"], "mcp_plus", "{agent}");

    let coverage = &agent["lifecycle_coverage"];
    let guaranteed = coverage["guaranteed"].to_string();
    for expected in [
        "lifecycle_session_open",
        "lifecycle_tool_success",
        "lifecycle_quiesce",
    ] {
        assert!(
            guaranteed.contains(expected),
            "OpenCode provides {expected} and the report does not say so: {guaranteed}"
        );
    }
    assert!(coverage["absent"].to_string().contains("session_close"));

    // Every conditional entry carries what it depends on (FR-241). The rule is
    // the assertion; the number of conditionals is not, because adding a
    // capability OpenCode genuinely qualifies for should not read as a failure.
    let conditional = agent["conditional_behaviors"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !conditional.is_empty(),
        "OpenCode has conditional capabilities and the report names none: {conditional:?}"
    );
    for entry in conditional {
        let text = entry.as_str().unwrap_or_default();
        assert!(
            text.contains(" only where "),
            "a conditional behavior did not name its condition: {text}"
        );
    }
}

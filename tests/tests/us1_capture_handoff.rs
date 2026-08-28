//! T026 — a real hook sequence produces typed observations and an accurate
//! handoff (SC-002, FR-041).

use cairn_e2e::Sandbox;
use serde_json::json;

fn work_a_session(s: &Sandbox, key: &str) {
    s.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );

    s.hook(
        "PostToolUse",
        json!({ "session_id": key, "tool_name": "Read", "tool_input": { "file_path": "README.md" } }),
    );
    s.write_file("src/lib.rs", "pub fn work() {}\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": key, "tool_name": "Edit", "tool_input": { "file_path": "src/lib.rs" } }),
    );
    s.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test --workspace" },
            "tool_response": { "exit_code": 101 }
        }),
    );
    // Failures arrive on their own event and are never inferred (D16).
    s.hook(
        "PostToolUseFailure",
        json!({
            "session_id": key,
            "tool_name": "Read",
            "tool_input": { "file_path": "missing.rs" },
            "error": { "message": "no such file" }
        }),
    );
}

#[test]
fn handoff_names_the_changed_file_the_failing_test_and_a_next_step() {
    let s = Sandbox::new();
    work_a_session(&s, "sess-1");
    s.hook(
        "SessionEnd",
        json!({ "session_id": "sess-1", "reason": "clear" }),
    );

    let handoff = s.handoff_after_close(&[]);

    let changed = handoff["changed_files"].as_array().expect("changed_files");
    assert!(
        changed.iter().any(|f| f.as_str() == Some("src/lib.rs")),
        "changed files missing src/lib.rs: {changed:?}"
    );

    let tests = handoff["tests_executed"]
        .as_array()
        .expect("tests_executed");
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0]["outcome"], "failed");
    // `runner`, not `command`. The field was renamed rather than sanitized in
    // place (FR-532): the server's wire denylist screens *field names* and now
    // recurses, so a key literally called `command` nested inside
    // `tests_executed` was refused on sight however clean its value was — every
    // handoff carrying a completed test run would have been rejected. The value
    // is now the runner's name with flags and paths already stripped.
    assert_eq!(
        tests[0]["runner"].as_str().unwrap(),
        "cargo test",
        "the runner name should be the invocation with its flags and paths removed"
    );
    assert!(
        tests[0].get("command").is_none(),
        "a `command` key is back on a test run; the recursive wire denylist will \
         refuse every handoff that carries one"
    );

    let failures = handoff["failures"].as_array().expect("failures");
    assert!(failures
        .iter()
        .any(|f| f.as_str().unwrap_or_default().contains("cargo test")));
    assert!(failures
        .iter()
        .any(|f| f.as_str().unwrap_or_default().contains("no such file")));

    let next = handoff["next_step"].as_str().unwrap_or_default();
    assert!(!next.is_empty(), "a handoff must recommend a next step");
    assert!(next.to_lowercase().contains("fix"), "next step was: {next}");

    assert_eq!(handoff["trigger"], "session_end");
}

#[test]
fn observations_are_typed_and_carry_no_transcript() {
    let s = Sandbox::new();
    work_a_session(&s, "sess-2");

    s.settle_observations(4);
    let status = s.json(&["status"]);
    assert_eq!(
        status["observation_count"], 4,
        "one observation per hook event"
    );

    // The success events produced their own types; the failure produced `error`.
    let bytes = s.db_bytes();
    let text = String::from_utf8_lossy(&bytes);
    for expected in ["file_read", "file_changed", "test_run", "error"] {
        assert!(
            text.contains(expected),
            "no {expected} observation was stored"
        );
    }
    assert!(
        !text.contains("hookSpecificOutput"),
        "hook plumbing must not be persisted as content"
    );
}

#[test]
fn every_stored_payload_is_within_the_configured_bound() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "sess-3", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "sess-3",
            "tool_name": "Bash",
            "tool_input": { "command": format!("echo {}", "x".repeat(200_000)) },
            "tool_response": { "exit_code": 0 }
        }),
    );

    let db = s.db_path();
    let out = std::process::Command::new("sqlite3")
        .arg(&db)
        .arg("SELECT MAX(payload_bytes) FROM observations;")
        .output();

    match out {
        Ok(o) if o.status.success() => {
            let max: i64 = String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or_default();
            assert!(
                max <= 4096,
                "stored payload {max} exceeded the 4 KB cap (FR-013)"
            );
        }
        // No sqlite3 CLI here: fall back to the file itself never carrying the
        // full blob.
        _ => {
            let text = String::from_utf8_lossy(&s.db_bytes()).to_string();
            assert!(
                !text.contains(&"x".repeat(50_000)),
                "an unbounded payload was stored"
            );
        }
    }
}

#[test]
fn a_precompact_boundary_writes_a_handoff_and_keeps_the_session_active() {
    let s = Sandbox::new();
    work_a_session(&s, "sess-4");
    s.hook(
        "PreCompact",
        json!({ "session_id": "sess-4", "trigger": "auto" }),
    );
    s.settle_handoff("pre_compact");

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    assert_eq!(
        sessions[0]["status"], "active",
        "compaction is not a session boundary"
    );

    let handoff = s.handoff_after_close(&[]);
    assert_eq!(handoff["trigger"], "pre_compact");
}

// ---------------------------------------------------------------------------
// T049 — the canonical rename changed no stored behavior (FR-114, US2 #5)
// ---------------------------------------------------------------------------

/// Feature 002 renamed the vendor events into a canonical vocabulary and
/// routed them through a new request. That is a translation layer, and the
/// only honest test of a translation layer is that the thing on the other side
/// is unchanged.
///
/// So this asserts the *stored* result of one ordinary Claude session, field
/// by field, against what Feature 001 produced for the same actions: the
/// observation rows and their types, the turn checkpoint, the compaction
/// handoff, and the session-end handoff. Anything the canonical rename altered
/// would show up here as a changed type, a lost field, or a handoff that
/// summarizes differently.
#[test]
fn a_full_claude_session_stores_exactly_what_feature_001_stored() {
    let s = Sandbox::new();
    let key = "equivalence-1";

    // Read, edit, run a failing test, hit a tool failure — the same four
    // actions `work_a_session` drives, which is Feature 001's own fixture.
    work_a_session(&s, key);
    s.settle_observations(4);

    // Four observations, one per event, with Feature 001's types and outcomes.
    let status = s.json(&["status"]);
    assert_eq!(status["observation_count"], 4);

    // Ordered by type rather than by arrival: capture is fire-and-forget and
    // four separate hook processes land in whatever order the scheduler gives
    // them, which is Feature 001's behavior too and not what is under test.
    let mut kinds = s.query_column("SELECT type FROM observations WHERE deleted_at IS NULL");
    kinds.sort();
    assert_eq!(
        kinds,
        vec!["error", "file_changed", "file_read", "test_run"],
        "the canonical rename changed which observation types are stored"
    );

    let row = |kind: &str, column: &str| -> String {
        s.query_column(&format!(
            "SELECT CAST(COALESCE({column}, '-') AS TEXT) FROM observations \
             WHERE deleted_at IS NULL AND type = '{kind}'"
        ))
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("no {kind} observation"))
    };

    // Each type carries the fields Feature 001 gave it, and no others.
    assert_eq!(row("file_read", "path"), "README.md");
    assert_eq!(row("file_read", "outcome"), "-");
    assert_eq!(row("file_changed", "path"), "src/lib.rs");
    assert_eq!(row("file_changed", "outcome"), "-");
    assert_eq!(row("test_run", "command"), "cargo test --workspace");
    assert_eq!(
        row("test_run", "outcome"),
        "failed",
        "a failing test run was not recorded as failed"
    );
    assert_eq!(row("test_run", "exit_code"), "101");
    assert_eq!(row("error", "path"), "missing.rs");
    assert_eq!(
        row("error", "outcome"),
        "error",
        "the failure event's own outcome changed"
    );

    // The turn checkpoint. `Stop` is a checkpoint and never a session end
    // (FR-015, FR-193), and it stores no observation of its own.
    s.hook("Stop", json!({ "session_id": key }));
    s.settle_turn_checkpoint();
    assert_eq!(
        s.json(&["status"])["observation_count"],
        4,
        "quiescence invented an observation"
    );
    let sessions = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        sessions[0]["status"], "active",
        "a checkpoint ended a session"
    );
    assert!(sessions[0]["last_turn_ended_at"].is_string());

    // The compaction handoff: written before compaction, leaving the session
    // usable (FR-119).
    s.hook(
        "PreCompact",
        json!({ "session_id": key, "trigger": "auto" }),
    );
    s.settle_handoff("pre_compact");
    let compaction = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(compaction["trigger"], "pre_compact");
    assert!(compaction["next_step"].is_string());
    assert!(compaction["changed_files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .any(|f| f.as_str() == Some("src/lib.rs")));
    assert_eq!(
        s.json(&["session", "list"])["sessions"][0]["status"],
        "active",
        "a compaction handoff ended the session"
    );

    // And the session-end handoff, with the same substance Feature 001's own
    // assertion above requires of it.
    s.hook(
        "SessionEnd",
        json!({ "session_id": key, "reason": "clear" }),
    );
    // A pre-compaction handoff already exists for this session, so waiting for
    // "a handoff" would return that one: the wait is for the second.
    s.settle_handoff("session_end");
    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(handoff["trigger"], "session_end");

    let tests = handoff["tests_executed"]
        .as_array()
        .expect("tests_executed");
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0]["outcome"], "failed");
    assert!(tests[0]["runner"]
        .as_str()
        .unwrap_or_default()
        .contains("cargo test"));
    assert!(
        tests[0].get("command").is_none(),
        "a `command` key is back on a test run (FR-532)"
    );

    let failures = handoff["failures"].as_array().expect("failures");
    assert!(failures
        .iter()
        .any(|f| f.as_str().unwrap_or_default().contains("cargo test")));
    assert!(
        failures
            .iter()
            .any(|f| f.as_str().unwrap_or_default().contains("no such file")),
        "the failure event's own reason was lost in translation: {failures:?}"
    );
    assert!(handoff["changed_files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .any(|f| f.as_str() == Some("src/lib.rs")));
    assert!(handoff["repository_state"].is_object());

    // Two handoffs, not three: quiescence produces none (FR-114, FR-230).
    let count = s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM handoffs");
    assert_eq!(
        count.first().map(String::as_str),
        Some("2"),
        "the canonical rename changed how many handoffs a session produces"
    );

    // The session is terminal, and its provenance still names the agent that
    // did the work.
    let sessions = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(sessions[0]["status"], "completed");
    assert_eq!(sessions[0]["agent"], "claude-code");
}

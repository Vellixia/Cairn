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

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();

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
    assert!(tests[0]["command"].as_str().unwrap().contains("cargo test"));

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

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(handoff["trigger"], "pre_compact");
}

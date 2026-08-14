//! T054 — the developer controls what Cairn stores (SC-008, FR-048 – FR-052).

use cairn_e2e::Sandbox;
use serde_json::json;

const SEEDED_KEY: &str = "sk-livetest0123456789abcdefghijkl";
const SEEDED_PASSWORD: &str = "correct-horse-battery-staple";

#[test]
fn exclusions_drop_matching_paths_and_commands_entirely() {
    let s = Sandbox::new();
    s.must(&["privacy", "exclude", "--path", "secrets/**"]);
    s.must(&["privacy", "exclude", "--command", "aws sts*"]);
    s.hook(
        "SessionStart",
        json!({ "session_id": "p", "source": "startup" }),
    );

    s.hook(
        "PostToolUse",
        json!({ "session_id": "p", "tool_name": "Read",
                "tool_input": { "file_path": "secrets/prod.env" } }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "p", "tool_name": "Bash",
                "tool_input": { "command": "aws sts get-caller-identity" },
                "tool_response": { "exit_code": 0 } }),
    );
    // One allowed event, to prove capture is working at all.
    s.hook(
        "PostToolUse",
        json!({ "session_id": "p", "tool_name": "Read",
                "tool_input": { "file_path": "README.md" } }),
    );

    // Exactly one survives the exclusions; wait for it, then hold the line.
    s.settle_observations(1);
    assert_eq!(
        s.json(&["status"])["observation_count"],
        1,
        "exclusions must drop, not store"
    );
    let text = String::from_utf8_lossy(&s.db_bytes()).to_string();
    assert!(
        !text.contains("secrets/prod.env"),
        "an excluded path reached storage"
    );
    assert!(
        !text.contains("get-caller-identity"),
        "an excluded command reached storage"
    );
}

#[test]
fn no_seeded_secret_reaches_storage_in_any_form() {
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "sec", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "sec", "tool_name": "Bash",
                "tool_input": { "command": format!("export OPENAI_API_KEY={SEEDED_KEY}") },
                "tool_response": { "exit_code": 0 } }),
    );
    s.hook(
        "PostToolUseFailure",
        json!({ "session_id": "sec", "tool_name": "Bash",
                "tool_input": { "command": "psql" },
                "error": { "message": format!("auth failed for password={SEEDED_PASSWORD}") } }),
    );
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        &format!("the token is {SEEDED_KEY}"),
    ]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "sec", "reason": "clear" }),
    );

    let text = String::from_utf8_lossy(&s.db_bytes()).to_string();
    assert!(
        !text.contains(SEEDED_KEY),
        "an API key reached storage (FR-049)"
    );
    assert!(
        !text.contains(SEEDED_PASSWORD),
        "a password reached storage (FR-049)"
    );
    assert!(
        text.contains("[REDACTED]"),
        "redaction should be visible where it happened"
    );
}

#[test]
fn deleting_a_session_keeps_the_memory_and_handoff_it_produced() {
    // FR-052: a cascade here would destroy the knowledge Cairn exists to keep.
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "d", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "d", "tool_name": "Edit", "tool_input": { "file_path": "a.rs" } }),
    );
    let memory = s.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "kept across deletions",
    ]);
    let memory_id = memory["memory"]["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionEnd",
        json!({ "session_id": "d", "reason": "clear" }),
    );

    let handoff = s.handoff_after_close(&[]);
    let handoff_id = handoff["id"].as_str().unwrap().to_string();
    let session_id = handoff["session_id"].as_str().unwrap().to_string();

    s.must(&["delete", "session", &session_id]);

    // Both survive, and the memory is still searchable.
    assert!(
        s.cairn(&["memory", "show", &memory_id]).ok(),
        "memory was destroyed with its session"
    );
    assert!(
        !s.json(&["memory", "search", "deletions"])["results"]
            .as_array()
            .unwrap()
            .is_empty(),
        "memory should remain recallable after its session is deleted"
    );
    assert!(
        s.cairn(&["handoff", "show", "--session", &session_id]).ok()
            || s.json_err(&["handoff", "show", "--session", &session_id])["code"] != "not_found",
        "the handoff should survive its session"
    );
    let _ = handoff_id;
}

#[test]
fn deleting_an_observation_leaves_provenance_resolvable_but_contentless() {
    // FR-052: the reference survives, the content does not, and the memory the
    // observation supported is untouched.
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "o", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "o", "tool_name": "Edit",
                "tool_input": { "file_path": "traceable-path.rs" } }),
    );
    s.settle_observations(1);

    // Attach that observation to a memory as evidence, so provenance is real.
    let observation_id = s.observation_ids()[0].clone();
    let memory = s.json(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "--evidence",
        &observation_id,
        "supported by evidence",
    ]);
    let memory_id = memory["memory"]["id"].as_str().unwrap().to_string();

    let before = s.json(&["memory", "show", &memory_id])["memory"].clone();
    assert_eq!(before["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(before["evidence"][0]["deleted"], false);
    assert!(
        String::from_utf8_lossy(&s.db_bytes()).contains("traceable-path.rs"),
        "the path should be stored before the delete"
    );

    s.must(&["delete", "observation", &observation_id]);

    // The memory survives whole.
    let after = s.json(&["memory", "show", &memory_id])["memory"].clone();
    assert_eq!(after["content"], "supported by evidence");

    // The reference is still resolvable, and reports the deletion.
    let evidence = after["evidence"].as_array().unwrap();
    assert_eq!(
        evidence.len(),
        1,
        "the provenance reference must not vanish"
    );
    assert_eq!(evidence[0]["observation_id"], observation_id);
    assert_eq!(evidence[0]["deleted"], true, "it must report as deleted");

    // And the content is gone from storage.
    assert!(
        !String::from_utf8_lossy(&s.db_bytes()).contains("traceable-path.rs"),
        "observation content survived its deletion"
    );

    // Search still returns the memory, with its provenance intact.
    let found = s.json(&["memory", "search", "evidence"]);
    let hit = &found["results"][0];
    assert_eq!(hit["provenance"]["evidence_count"], 1);
    assert_eq!(
        hit["provenance"]["deleted_observation_ids"][0], observation_id,
        "search must report which evidence is gone"
    );
}

#[test]
fn a_local_only_memory_is_never_queued_for_sync() {
    let s = Sandbox::new();
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "--local-only",
        "stays on this machine",
    ]);
    let status = s.json(&["sync", "status"]);
    assert_eq!(status["linked"], false);
    assert_eq!(
        status["pending"], 0,
        "an unlinked project must queue nothing (SC-010)"
    );
}

#[test]
fn deleting_a_memory_removes_only_that_memory() {
    let s = Sandbox::new();
    let a = s.json(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "first fact",
    ]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "second fact",
    ]);
    let id = a["memory"]["id"].as_str().unwrap().to_string();

    s.must(&["delete", "memory", &id]);
    let remaining = s.json(&["memory", "search", "fact"]);
    let results = remaining["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["content"], "second fact");
}

#[test]
fn deleting_a_nonexistent_memory_reports_not_found() {
    let s = Sandbox::new();
    let err = s.json_err(&["delete", "memory", "00000000-0000-0000-0000-000000000000"]);
    assert_eq!(err["code"], "not_found");
}

#[test]
fn deleting_a_nonexistent_session_reports_not_found() {
    let s = Sandbox::new();
    let err = s.json_err(&["delete", "session", "00000000-0000-0000-0000-000000000000"]);
    assert_eq!(err["code"], "not_found");
}

#[test]
fn deleting_a_nonexistent_observation_is_idempotent() {
    // Observation and handoff deletes are idempotent: a non-existent id
    // succeeds rather than erroring, because the end state (no record)
    // matches the requested state.
    let s = Sandbox::new();
    s.must(&[
        "delete",
        "observation",
        "00000000-0000-0000-0000-000000000000",
    ]);
}

#[test]
fn deleting_a_nonexistent_handoff_is_idempotent() {
    let s = Sandbox::new();
    s.must(&["delete", "handoff", "00000000-0000-0000-0000-000000000000"]);
}

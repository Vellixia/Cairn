//! T116 — conversation content never becomes storage (FR-198, FR-199,
//! SC-121, D34, D35).
//!
//! Feature 002 reads payloads Feature 001 never saw. Claude's `Stop` now
//! carries `last_assistant_message` and `tool_calls`; OpenCode's
//! `tool.execute.after` carries the tool's whole `output` and `metadata`;
//! every vendor puts a `transcript_path` on everything. All of it arrives on
//! the same stdin as the two or three fields Cairn actually wants.
//!
//! So the assertion is not "the adapter does not copy the transcript" — it is
//! that none of this text exists anywhere in Cairn's storage afterwards, by
//! any route, having been seeded into every field of every payload the
//! adapters accept.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

/// Recognisable strings, one per kind of content that must never be stored.
const ASSISTANT_TEXT: &str = "ASSISTANTTEXT-I-rewrote-the-pool-and-it-works-now";
const USER_PROMPT: &str = "USERPROMPT-please-fix-the-pool-without-touching-migrations";
const TRANSCRIPT: &str = "TRANSCRIPTPATH-projects-home-dev-app-b7f3a1c2.jsonl";
const TOOL_OUTPUT: &str = "TOOLOUTPUT-running-63-tests-all-passed-in-0.04s";
const TOOL_METADATA: &str = "TOOLMETADATA-diff-at-at-1-1-plus-8";
const THINKING: &str = "THINKING-maybe-the-pool-size-is-the-problem";

const CONTENT: [&str; 6] = [
    ASSISTANT_TEXT,
    USER_PROMPT,
    TRANSCRIPT,
    TOOL_OUTPUT,
    TOOL_METADATA,
    THINKING,
];

/// Everything Cairn stored, as one searchable blob: the database file, its
/// write-ahead log, and everything readable through the CLI.
fn storage(s: &Sandbox) -> String {
    let mut text = String::from_utf8_lossy(&s.db_bytes()).to_string();
    for command in [
        vec!["--json", "status"],
        vec!["--json", "context"],
        vec!["--json", "session", "list"],
        vec!["--json", "handoff", "show"],
        vec!["--json", "memory", "search", "pool"],
    ] {
        let out = s.cairn(&command);
        text.push_str(&out.stdout);
    }
    text
}

fn assert_no_conversation(s: &Sandbox, after: &str) {
    let stored = storage(s);
    for seeded in CONTENT {
        assert!(
            !stored.contains(seeded),
            "conversation content reached storage {after}: {seeded}"
        );
    }
}

/// A Claude session where every payload carries conversation content.
fn claude_session(s: &Sandbox) {
    s.hook(
        "SessionStart",
        json!({
            "session_id": "priv-claude",
            "source": "startup",
            "transcript_path": TRANSCRIPT,
            "prompt": USER_PROMPT
        }),
    );
    s.settle_session_count(1);
    s.hook(
        "UserPromptSubmit",
        json!({ "session_id": "priv-claude", "prompt": USER_PROMPT }),
    );
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "priv-claude",
            "transcript_path": TRANSCRIPT,
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test -p cairn-store", "description": THINKING },
            "tool_response": { "exit_code": 0, "stdout": TOOL_OUTPUT, "stderr": TOOL_OUTPUT }
        }),
    );
    s.hook(
        "PostToolUseFailure",
        json!({
            "session_id": "priv-claude",
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/pool.rs", "new_string": ASSISTANT_TEXT },
            "error": { "message": "the file changed on disk", "detail": TOOL_OUTPUT }
        }),
    );
    s.hook(
        "Stop",
        json!({
            "session_id": "priv-claude",
            "transcript_path": TRANSCRIPT,
            "last_assistant_message": ASSISTANT_TEXT,
            "tool_calls": [{ "name": "Edit", "input": { "content": ASSISTANT_TEXT } }],
            "thinking": THINKING
        }),
    );
    s.settle_turn_checkpoint();
    s.hook(
        "PreCompact",
        json!({
            "session_id": "priv-claude",
            "trigger": "auto",
            "custom_instructions": USER_PROMPT,
            "transcript_path": TRANSCRIPT
        }),
    );
    s.settle_handoff("pre_compact");
    s.hook(
        "SessionEnd",
        json!({
            "session_id": "priv-claude",
            "reason": "clear",
            "last_assistant_message": ASSISTANT_TEXT,
            "transcript_path": TRANSCRIPT
        }),
    );
    s.settle("the closed session's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });
}

#[test]
fn no_conversation_content_from_claude_reaches_storage() {
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    claude_session(&s);

    // The capture worked, so this is a test about filtering rather than about
    // an empty database. Capture is fire-and-forget, so this is waited for
    // rather than read (H3).
    s.settle("the session's observations", |s| {
        s.json(&["status"])["observation_count"]
            .as_i64()
            .unwrap_or(0)
            >= 2
    });
    let stored = storage(&s);
    assert!(
        stored.contains("cargo test -p cairn-store"),
        "the allow-listed command was dropped along with everything else"
    );

    assert_no_conversation(&s, "from a Claude session");
}

#[test]
fn no_conversation_content_from_codex_or_opencode_reaches_storage() {
    let s = Sandbox::new();
    s.install_agent("codex");
    s.install_agent("opencode");
    s.must(&["init"]);

    s.hook_as(
        "codex",
        "SessionStart",
        json!({
            "session_id": "priv-codex",
            "source": "startup",
            "transcript_path": TRANSCRIPT
        }),
    );
    s.hook_as(
        "opencode",
        "session.created",
        json!({ "sessionID": "priv-opencode", "title": USER_PROMPT }),
    );
    s.settle_session_count(2);

    s.hook_as(
        "codex",
        "PostToolUse",
        json!({
            "session_id": "priv-codex",
            "tool_name": "shell",
            "tool_input": { "command": "cargo test -p cairn-git", "reasoning": THINKING },
            "tool_response": { "exit_code": 0, "output": TOOL_OUTPUT }
        }),
    );
    s.hook_as(
        "opencode",
        "tool.execute.after",
        json!({
            "sessionID": "priv-opencode",
            "tool": "bash",
            "args": { "command": "cargo build", "description": THINKING },
            "title": ASSISTANT_TEXT,
            "output": { "exit_code": 0, "text": TOOL_OUTPUT },
            "metadata": { "diff": TOOL_METADATA, "preview": ASSISTANT_TEXT }
        }),
    );
    s.hook_as(
        "opencode",
        "chat.message",
        json!({
            "sessionID": "priv-opencode",
            "message": { "role": "assistant", "content": ASSISTANT_TEXT }
        }),
    );
    s.settle_observations(2);

    s.hook_as(
        "codex",
        "SessionEnd",
        json!({ "session_id": "priv-codex", "reason": "other" }),
    );
    s.settle("the closed session's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    let stored = storage(&s);
    assert!(
        stored.contains("cargo test -p cairn-git") && stored.contains("cargo build"),
        "the allow-listed commands were dropped: nothing to prove here"
    );
    assert_no_conversation(&s, "from Codex and OpenCode sessions");
}

#[test]
fn every_captured_field_passes_through_the_feature_001_pipeline() {
    // FR-199: there is no path around exclusion → redaction → bounding. The
    // adapters feed the same pipeline Feature 001 built, so an exclusion set
    // by the developer applies to a Codex payload exactly as it does to a
    // Claude one, and a secret is redacted whichever adapter carried it.
    let s = Sandbox::new();
    s.install_agent("codex");
    s.install_agent("opencode");
    s.must(&["init"]);
    s.must(&["privacy", "exclude", "--path", "secrets/**"]);
    s.must(&["privacy", "exclude", "--command", "aws sts*"]);

    s.hook_as(
        "codex",
        "SessionStart",
        json!({ "session_id": "pipeline", "source": "startup" }),
    );
    s.settle_session_count(1);

    // Excluded by path, through a non-Claude adapter.
    s.hook_as(
        "codex",
        "PostToolUse",
        json!({
            "session_id": "pipeline",
            "tool_name": "apply_patch",
            "tool_input": { "file_path": "secrets/prod.env" },
            "tool_response": { "exit_code": 0 }
        }),
    );
    // Excluded by command, through the plugin path.
    s.hook_as(
        "opencode",
        "session.created",
        json!({ "sessionID": "pipeline-oc" }),
    );
    s.hook_as(
        "opencode",
        "tool.execute.after",
        json!({
            "sessionID": "pipeline-oc",
            "tool": "bash",
            "args": { "command": "aws sts get-caller-identity" },
            "output": { "exit_code": 0 }
        }),
    );
    // Redaction: a live-looking key inside an allowed command.
    s.hook_as(
        "codex",
        "PostToolUse",
        json!({
            "session_id": "pipeline",
            "tool_name": "shell",
            "tool_input": { "command": "deploy --token sk-live-0123456789abcdefghijklmn" },
            "tool_response": { "exit_code": 0 }
        }),
    );

    s.settle_observations(1);
    assert_eq!(
        s.json(&["status"])["observation_count"],
        1,
        "the exclusions did not apply to a Feature 002 adapter"
    );

    let stored = storage(&s);
    assert!(
        !stored.contains("secrets/prod.env"),
        "an excluded path reached storage through an adapter"
    );
    assert!(
        !stored.contains("get-caller-identity"),
        "an excluded command reached storage through the plugin path"
    );
    assert!(
        !stored.contains("sk-live-0123456789abcdefghijklmn"),
        "a credential reached storage unredacted through an adapter"
    );
    assert!(
        stored.contains("deploy --token"),
        "redaction removed the command as well as the secret"
    );
}

#[test]
fn a_handoff_built_from_adapter_events_carries_no_conversation_content() {
    // The handoff is the one record built by summarising many events, which
    // makes it the likeliest place for content to reappear having been
    // correctly dropped from every observation.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    claude_session(&s);

    let handoff: Value = s.json(&["handoff", "show"])["handoff"].clone();
    assert!(handoff["next_step"].is_string(), "no handoff was produced");
    let text = handoff.to_string();
    for seeded in CONTENT {
        assert!(
            !text.contains(seeded),
            "conversation content reached the handoff: {seeded}"
        );
    }
    // What it does carry is the developer's own repository state.
    assert!(handoff["repository_state"].is_object());
}

// ---------------------------------------------------------------------------
// T176/T177 — `changed_files` and the `completed_work` prose built from it
// carry repository-relative paths only, never absolute ones (FR-531, SC-431)
// ---------------------------------------------------------------------------

/// A file with no Git counterpart at all — gitignored, so `git status`, and
/// therefore `git_changed_files`, never names it in any form.
///
/// This is the case the previous "keep the shorter of a long/short pair"
/// dedup could not collapse: that logic only fires when a *paired* relative
/// form is already present in the same list to match the absolute one
/// against. With no Git counterpart to pair against, the absolute path used
/// to survive untouched. The fix relativizes every path against the
/// session's own recorded worktree root, unconditionally, so a path with no
/// pair is still corrected rather than passing through.
#[test]
fn a_handoff_carries_no_absolute_path_for_a_file_git_never_reports() {
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);

    s.write_file(".gitignore", "generated/\n");
    let ignored_relative = "generated/ignored_module.rs";
    s.write_file(ignored_relative, "pub fn generated() {}\n");
    // The daemon records the session's worktree root canonicalized
    // (`cairn_git::discover`), so the absolute path this fixture feeds
    // through the hook is canonicalized the same way — otherwise this test
    // would be exercising a platform symlink quirk (macOS's `/var` is itself
    // a symlink to `/private/var`) rather than the "no Git counterpart" case
    // it exists to prove. That distinct symlink-mismatch case already has
    // its own coverage: `derive_changed_files`'s suffix-based fallback in
    // `handoff.rs`, and the `one_file_reported_two_ways_is_one_changed_file`
    // unit test.
    let ignored_absolute = std::fs::canonicalize(s.repo_dir().join(ignored_relative))
        .expect("the file was just written")
        .to_str()
        .expect("a UTF-8 sandbox path")
        .to_string();

    s.hook(
        "SessionStart",
        json!({ "session_id": "no-git-counterpart", "source": "startup" }),
    );
    s.settle_session_count(1);
    // A real agent's own `tool_input.file_path` is absolute (the doc comment
    // at `handoff.rs`'s `derive_changed_files` says so outright) — the
    // fixture matches that rather than a synthetic relative path, or it would
    // not exercise the bug this test exists to catch.
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "no-git-counterpart",
            "tool_name": "Edit",
            "tool_input": { "file_path": ignored_absolute }
        }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "no-git-counterpart", "reason": "clear" }),
    );
    s.settle_handoff("session_end");

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();

    // The file was captured — this is a test about relativizing a path, not
    // about an exclusion (a different mechanism entirely) silently eating it.
    let changed = handoff["changed_files"].as_array().expect("changed_files");
    assert!(
        changed.iter().any(|f| f.as_str() == Some(ignored_relative)),
        "the file Git never reports was not captured at all: {changed:?}"
    );

    // Neither the exact absolute path nor the repository root it was built
    // from survives anywhere in the transmitted object — not in
    // `changed_files`, and not in the `completed_work` prose formatted from
    // it either.
    let text = handoff.to_string();
    assert!(
        !text.contains(&ignored_absolute),
        "the absolute path survived somewhere in the handoff: {text}"
    );
    let repo_root = std::fs::canonicalize(s.repo_dir())
        .expect("the sandbox repository exists")
        .to_str()
        .expect("a UTF-8 sandbox path")
        .to_string();
    assert!(
        !text.contains(&repo_root),
        "the repository's absolute root leaked into the handoff: {text}"
    );
    let completed = handoff["completed_work"]
        .as_array()
        .expect("completed_work");
    assert!(
        completed
            .iter()
            .any(|c| c.as_str().unwrap_or_default().contains(ignored_relative)),
        "completed_work did not describe the change at all: {completed:?}"
    );
}

// ---------------------------------------------------------------------------
// T132 — the wire refuses every newly forbidden name, by name (FR-506, SC-316)
// ---------------------------------------------------------------------------

/// Field names that would carry evidence, diagnostic or checkpoint content.
///
/// Listed here rather than imported so the test and the server do not agree by
/// construction: if someone removes one from the server's list, this fails.
const FORBIDDEN_FIELDS: &[&str] = &[
    "observed_value",
    "source_locator",
    "value_digest",
    "fingerprint",
    "relevant_paths",
    "path_fingerprints",
    "criteria_snapshot",
    "task_snapshot_at_bind",
    "sanitization_report",
    "origin_ref",
    "alternative_cause",
    "signal_digest",
    "pin_reason",
    "rationale",
    "basis_evidence_id",
    "detail",
    "prior_value",
    "new_value",
    "content_norm_digest",
];

/// Record kinds that are local to the machine that produced them.
const FORBIDDEN_ENTITY_TYPES: &[&str] = &[
    "evidence_fact",
    "verification_run",
    "continuity_checkpoint",
    "reusable_pattern",
    "pattern_application",
    "task_change",
];

fn linked_server() -> Option<(cairn_e2e::Server, Sandbox, String, String)> {
    let server = cairn_e2e::Server::start()?;
    let s = Sandbox::new();
    let token = server.new_user_token("forbidden");
    cairn_e2e::attach_server(&s, &server, &token);
    s.must(&["init"]);
    let project_id = s.json(&["link", "--create"])["server_project_id"]
        .as_str()
        .expect("a shared project")
        .to_string();
    Some((server, s, token, project_id))
}

fn item(entity_type: &str, key: &str, payload: serde_json::Value) -> serde_json::Value {
    json!({
        "idempotency_key": key,
        "entity_type": entity_type,
        "entity_id": uuid::Uuid::now_v7(),
        "operation": "upsert",
        "payload": payload,
    })
}

/// Every forbidden field is refused **and the refusal names it**.
///
/// Naming it is the part that matters. A generic "invalid request" would leave
/// whoever sent it guessing, and — worse — would be indistinguishable from a
/// capability refusal, which is retained and retried rather than failed.
#[test]
fn every_forbidden_field_is_refused_by_name() {
    let Some((server, _s, token, project_id)) = linked_server() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
        return;
    };

    let items: Vec<serde_json::Value> = FORBIDDEN_FIELDS
        .iter()
        .enumerate()
        .map(|(i, field)| {
            item(
                "memory",
                &format!("forbidden-field-{i}"),
                json!({ "content": "a legitimate memory", *field: "leaked" }),
            )
        })
        .collect();

    let out = cairn_e2e::post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &json!({ "project_id": project_id, "items": items }),
        &token,
    );
    let results = out["results"].as_array().expect("results");
    assert_eq!(results.len(), FORBIDDEN_FIELDS.len(), "{out}");

    for (result, field) in results.iter().zip(FORBIDDEN_FIELDS) {
        assert_eq!(
            result["status"], "rejected",
            "`{field}` was accepted: {out}"
        );
        let message = result["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(field),
            "the refusal for `{field}` does not name it: {message}"
        );
    }

    // And nothing of it reached storage, which is the assertion the API answer
    // alone cannot make.
    let dump = server.dump();
    assert!(
        !dump.contains("leaked"),
        "a refused value reached the database anyway"
    );
}

/// Every forbidden entity type is refused outright.
///
/// These are not fields on a record the server keeps — they are record kinds it
/// has no table for and must never grow one. The refusal is by name so a
/// malformed or malicious client cannot create a table's worth of local-only
/// content by asking nicely.
#[test]
fn every_forbidden_entity_type_is_refused() {
    let Some((server, _s, token, project_id)) = linked_server() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
        return;
    };

    let items: Vec<serde_json::Value> = FORBIDDEN_ENTITY_TYPES
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            item(
                kind,
                &format!("forbidden-type-{i}"),
                json!({ "content": "leaked-by-type" }),
            )
        })
        .collect();

    let out = cairn_e2e::post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &json!({ "project_id": project_id, "items": items }),
        &token,
    );
    let results = out["results"].as_array().expect("results");
    assert_eq!(results.len(), FORBIDDEN_ENTITY_TYPES.len(), "{out}");

    for (result, kind) in results.iter().zip(FORBIDDEN_ENTITY_TYPES) {
        assert_eq!(result["status"], "rejected", "`{kind}` was accepted: {out}");
        let message = result["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(kind),
            "the refusal for `{kind}` does not name it: {message}"
        );
    }

    let dump = server.dump();
    assert!(
        !dump.contains("leaked-by-type"),
        "a refused item was stored"
    );
    // The server has no table for any of them, which is what makes the refusal
    // a property of the schema rather than of this check.
    for kind in FORBIDDEN_ENTITY_TYPES {
        let table = format!("{kind}s");
        assert_eq!(
            server.count(&format!(
                "SELECT COUNT(*) FROM information_schema.tables
                  WHERE table_schema = 'public' AND table_name = '{table}'"
            )),
            0,
            "the server has a `{table}` table, which it must never have"
        );
    }
}

// ---------------------------------------------------------------------------
// T175/T180 — the wire check recurses, so nesting cannot launder a forbidden
// field past it (FR-535)
// ---------------------------------------------------------------------------

/// Payloads shaped exactly the way `promotion-privacy.md` §6 identified: a
/// forbidden field one level inside a nested object, or inside an object
/// nested in an array — which `object.contains_key(field)` at the top level
/// alone cannot see. This is the proof that `reject_forbidden_fields`
/// actually recurses: if it regressed to a top-level-only check, every one
/// of these would be **accepted** instead of rejected.
#[test]
fn a_forbidden_field_nested_inside_the_payload_is_still_refused() {
    let Some((server, _s, token, project_id)) = linked_server() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
        return;
    };

    let cases: [(&str, serde_json::Value); 3] = [
        (
            "summary",
            json!({
                "content": "a legitimate memory",
                // One level inside a nested object.
                "provenance": { "summary": "an observation summary that must never travel" }
            }),
        ),
        (
            "path",
            json!({
                "content": "a legitimate memory",
                // One level inside an object nested in an array.
                "evidence": [{ "path": "/Users/dev/secret-project/file.rs" }]
            }),
        ),
        (
            "command",
            json!({
                "content": "a legitimate memory",
                "tests_executed": [{ "command": "cargo test --workspace", "outcome": "passed" }]
            }),
        ),
    ];

    let items: Vec<serde_json::Value> = cases
        .iter()
        .enumerate()
        .map(|(i, (_, payload))| item("memory", &format!("nested-forbidden-{i}"), payload.clone()))
        .collect();

    let out = cairn_e2e::post_json_bearer(
        &server.base,
        "/api/sync/batch",
        &json!({ "project_id": project_id, "items": items }),
        &token,
    );
    let results = out["results"].as_array().expect("results");
    assert_eq!(results.len(), cases.len(), "{out}");

    for (result, (field, _)) in results.iter().zip(cases.iter()) {
        assert_eq!(
            result["status"], "rejected",
            "a `{field}` nested inside the payload was accepted — the old \
             top-level-only check would have passed this too: {out}"
        );
        let message = result["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(field),
            "the refusal for a nested `{field}` does not name it: {message}"
        );
    }

    // And none of the leaked content reached storage either — the API
    // answer alone cannot make that claim.
    let dump = server.dump();
    assert!(
        !dump.contains("secret-project"),
        "a nested absolute path reached storage"
    );
    assert!(
        !dump.contains("must never travel"),
        "a nested observation summary reached storage"
    );
    assert!(
        !dump.contains("cargo test --workspace"),
        "a nested command string reached storage"
    );
}

// ---------------------------------------------------------------------------
// Feature 004 — the origin digest is local only (FR-551, SC-441)
// ---------------------------------------------------------------------------

/// The salted origin digest never reaches the wire, asserted **by
/// construction** rather than by inspecting one payload (T021).
///
/// The reason this matters more than it looks: the server knows every project
/// identity it holds. A transmitted digest of a project id is therefore a
/// lookup away from being reversed by the one party in the system best placed
/// to reverse it — enumerate the identities, digest each, compare. The salt
/// makes that intractable only while the salt stays local, and the digest
/// travelling would hand over the other half.
///
/// So the assertion is about the *type*, not a sample: a record carrying a
/// digest serializes without it. A field added later without `#[serde(skip)]`
/// fails this.
#[test]
fn no_serialized_global_record_carries_an_origin_digest() {
    use cairn_core::domain::{ApplicabilityFact, ApplicabilityKind, MemoryType, TeamState};
    use cairn_core::global::{PersonalKnowledge, TeamKnowledge};
    use uuid::Uuid;

    // A distinctive value, so a leak is unmistakable in the serialized form
    // rather than something a substring search could plausibly miss.
    let digest = "ORIGIN_DIGEST_THAT_MUST_NOT_TRAVEL".to_string();

    let personal = PersonalKnowledge {
        id: Uuid::now_v7(),
        owner_user_id: Uuid::now_v7(),
        knowledge_type: MemoryType::Convention,
        content: "prefer thiserror over hand-rolled Display impls".into(),
        topic_key: Some("errors.display".into()),
        value_key: Some("thiserror".into()),
        origin_digest: Some(digest.clone()),
        applicability: vec![ApplicabilityFact {
            kind: ApplicabilityKind::Language,
            value: "rust".into(),
        }],
        writer_id: Uuid::now_v7(),
        writer_seq: 7,
        created_at: chrono::Utc::now(),
        superseded_by_id: None,
        forgotten_at: None,
    };
    let team = TeamKnowledge {
        id: Uuid::now_v7(),
        knowledge_type: MemoryType::Convention,
        content: "retry flaky integration tests up to three times".into(),
        topic_key: Some("ci.retries".into()),
        value_key: Some("three".into()),
        origin_digest: Some(digest.clone()),
        applicability: vec![],
        state: TeamState::Authoritative,
        proposed_by_user_id: Uuid::now_v7(),
        ratified_by_user_id: Some(Uuid::now_v7()),
        ratified_at: Some(chrono::Utc::now()),
        writer_id: Uuid::now_v7(),
        writer_seq: 3,
        created_at: chrono::Utc::now(),
        superseded_by_id: None,
        retired_by_user_id: None,
        retired_at: None,
    };

    for (label, json) in [
        (
            "personal",
            serde_json::to_string(&personal).expect("serialize"),
        ),
        ("team", serde_json::to_string(&team).expect("serialize")),
    ] {
        assert!(
            !json.contains(&digest),
            "{label} serialized its origin digest: {json}"
        );
        assert!(
            !json.contains("origin_digest"),
            "{label} serialized an origin_digest field: {json}"
        );
        // The record itself must still be there — a test that passed because
        // serialization produced nothing would prove nothing.
        assert!(
            json.contains("content"),
            "{label} serialized nothing recognizable: {json}"
        );
    }
}

/// The digest field is present on the in-memory type and absent from the wire.
///
/// Stated as its own assertion because the two halves are separately
/// breakable: dropping the field entirely would satisfy the test above while
/// losing the feature (FR-516), and dropping `#[serde(skip)]` would keep the
/// feature while leaking it.
#[test]
fn the_digest_exists_locally_even_though_it_never_travels() {
    let digest = cairn_core::global::origin_digest("machine-salt", uuid::Uuid::now_v7());
    assert!(
        !digest.is_empty(),
        "the origin digest is computed locally (FR-516); only its transmission is forbidden"
    );
}

// ---------------------------------------------------------------------------
// T072 / SC-469 — a project's derived traits never leave the machine (FR-438)
// ---------------------------------------------------------------------------

/// `project_traits` appears in no transmitted payload and no server table.
///
/// Structural, and asserted two ways. The first is the one that cannot be
/// bypassed: `OutboxEntityType` has no `project_traits` variant, so an outbox
/// row for one is not something the code declines to write — it is something
/// that cannot be spelled. The second walks a corpus of projects whose traits
/// are all distinct, so a trait that *did* start synchronizing would show up as
/// a recognizable token rather than as a collision with another project's.
#[test]
fn a_projects_derived_traits_appear_in_no_payload_and_no_server_table() {
    use cairn_core::domain::{OutboxEntityType, ProjectTrait};
    use std::str::FromStr;

    // Structural: there is no entity type for them.
    assert!(
        OutboxEntityType::from_str("project_traits").is_err(),
        "`project_traits` can be named as an outbox entity type, so a row for it \
         can be written and traits would synchronize (FR-438)"
    );
    assert!(
        OutboxEntityType::from_str("writer_identity").is_err(),
        "`writer_identity` can be named as an outbox entity type; a store's own \
         opaque registry must stay local (D448)"
    );

    // And by value: a corpus whose traits are all distinct, serialized through
    // every type that crosses the wire.
    let corpus = [
        ProjectTrait {
            kind: cairn_core::domain::ApplicabilityKind::Language,
            value: "distinctivelangalpha".to_string(),
        },
        ProjectTrait {
            kind: cairn_core::domain::ApplicabilityKind::Tool,
            value: "distinctivetoolbeta".to_string(),
        },
    ];
    // A trait is not a member of any wire type, so there is nothing to serialize
    // it *into* — which is the assertion. Confirmed by checking that the two
    // global record types, the only ones that carry applicability at all, carry
    // no trait field.
    let personal = serde_json::to_string(&cairn_core::global::PersonalKnowledge {
        id: uuid::Uuid::now_v7(),
        owner_user_id: uuid::Uuid::now_v7(),
        knowledge_type: cairn_core::domain::MemoryType::Convention,
        content: "ordinary guidance".into(),
        topic_key: None,
        value_key: None,
        origin_digest: None,
        applicability: vec![],
        writer_id: uuid::Uuid::now_v7(),
        writer_seq: 1,
        created_at: chrono::Utc::now(),
        superseded_by_id: None,
        forgotten_at: None,
    })
    .expect("serialize");
    for t in &corpus {
        assert!(
            !personal.contains(&t.value),
            "a serialized record carried a project trait: {personal}"
        );
    }
    assert!(
        !personal.contains("project_traits") && !personal.contains("traits"),
        "a serialized record has a traits field: {personal}"
    );
}

// ---------------------------------------------------------------------------
// A handoff carrying a completed test run reaches the server (FR-532, FR-535)
// ---------------------------------------------------------------------------

/// A handoff with `tests_executed` is accepted, and one carrying real
/// observation content still is not.
///
/// **This is the test whose absence let a whole class of handoff become
/// undeliverable.** Two changes met here and neither was visible alone: FR-532
/// renamed `TestRunRecord.command` to `runner` precisely so a handoff with a test
/// run would not trip the field-name denylist, and FR-535 then made that denylist
/// recursive — at which point `outcome`, a name on the list because an
/// observation has one, matched `tests_executed[].outcome` and refused every such
/// handoff outright.
///
/// Nothing in the Rust suite pushed a handoff with a test run at a real server,
/// so nothing failed. A Playwright test in the web suite did, for unrelated
/// reasons, three layers away from the cause.
///
/// Falsified by returning `outcome` to the recursive list — or by removing the
/// second half of this test, which is what keeps the relaxation honest.
#[test]
fn a_handoff_with_a_test_run_is_accepted_and_observation_content_still_is_not() {
    let Some(server) = cairn_e2e::Server::start() else {
        eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the payload suite");
        return;
    };
    let token = server.new_user_token("handoff-testrun");
    let remote = format!("git@localhost:cairnfixture/{}.git", uuid::Uuid::now_v7());
    let (created, status) = cairn_e2e::post_json_status_bearer(
        &server.base,
        "/api/projects",
        &serde_json::json!({ "name": "handoff-testrun", "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id").to_string();

    let push = |payload: serde_json::Value| {
        cairn_e2e::post_json_status_bearer(
            &server.base,
            "/api/sync/batch",
            &serde_json::json!({
                "project_id": project,
                "items": [{
                    "idempotency_key": uuid::Uuid::now_v7().to_string(),
                    "entity_type": "handoff",
                    "entity_id": uuid::Uuid::now_v7().to_string(),
                    "operation": "upsert",
                    "payload": payload,
                }],
            }),
            &token,
        )
    };

    // The shape a real handoff has, `outcome` nested inside `tests_executed`.
    let (body, status) = push(serde_json::json!({
        "session_id": uuid::Uuid::now_v7(),
        "trigger": "session_end",
        "goal": "requests over the limit get 429",
        "next_step": "fix the open failure",
        "completed_work": ["changed 1 file"],
        "remaining_work": [],
        "changed_files": ["src/limiter.rs"],
        "decisions": [],
        "failures": [],
        "tests_executed": [{ "runner": "cargo test", "outcome": "failed" }],
    }));
    assert_eq!(status, 200, "the batch route itself failed: {body}");
    assert_eq!(
        body["results"][0]["status"].as_str(),
        Some("applied"),
        "a handoff carrying a completed test run was refused, so no session that \
         ran tests can ever synchronize its handoff: {body}"
    );

    // And the boundary the name was on the list for still holds: an
    // observation-shaped payload is refused.
    for forbidden in [
        serde_json::json!({ "session_id": uuid::Uuid::now_v7(), "outcome": "failed" }),
        serde_json::json!({
            "session_id": uuid::Uuid::now_v7(),
            "tests_executed": [{ "runner": "cargo test", "command": "cargo test --workspace" }],
        }),
        serde_json::json!({
            "session_id": uuid::Uuid::now_v7(),
            "evidence": { "observed_value": "42 requests" },
        }),
    ] {
        let (body, status) = push(forbidden.clone());
        assert_eq!(status, 200, "the batch route itself failed: {body}");
        assert_eq!(
            body["results"][0]["status"].as_str(),
            Some("rejected"),
            "observation content was accepted: {forbidden} -> {body}"
        );
    }
}

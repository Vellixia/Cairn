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

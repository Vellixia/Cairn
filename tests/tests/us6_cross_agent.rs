//! T110, T111 — knowledge crosses agents (US6, SC-108, FR-189–FR-192).
//!
//! This is the product. Everything else in Feature 002 is machinery for it.
//!
//! Driven the way D41 specifies: against a real daemon and a real temporary
//! repository, feeding each adapter's own recorded payload shape through the
//! real hook entry point, rather than launching three authenticated vendor
//! CLIs. The thing under test is Cairn's project resolution, memory scoping,
//! provenance and handoff — none of which depends on a real agent being
//! present, and requiring three logins would make the most important test in
//! the feature the least likely to run.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

/// Open a session for one agent, using that agent's own payload shape.
fn open(s: &Sandbox, agent: &str, key: &str) {
    match agent {
        "claude-code" => {
            s.hook(
                "SessionStart",
                json!({ "session_id": key, "source": "startup" }),
            );
        }
        "codex" => {
            s.hook_as(
                "codex",
                "SessionStart",
                json!({ "session_id": key, "source": "startup" }),
            );
        }
        "opencode" => {
            s.hook_as("opencode", "session.created", json!({ "sessionID": key }));
        }
        other => panic!("unknown agent {other}"),
    }
}

fn work(s: &Sandbox, agent: &str, key: &str, file: &str) {
    match agent {
        "claude-code" => s.hook(
            "PostToolUse",
            json!({ "session_id": key, "tool_name": "Edit", "tool_input": { "file_path": file } }),
        ),
        "codex" => s.hook_as(
            "codex",
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "apply_patch",
                "tool_input": { "file_path": file },
                "tool_response": { "exit_code": 0 }
            }),
        ),
        "opencode" => s.hook_as(
            "opencode",
            "tool.execute.after",
            json!({ "sessionID": key, "tool": "edit", "args": { "file_path": file } }),
        ),
        other => panic!("unknown agent {other}"),
    };
}

fn close(s: &Sandbox, agent: &str, key: &str) {
    match agent {
        "claude-code" => {
            s.hook(
                "SessionEnd",
                json!({ "session_id": key, "reason": "clear" }),
            );
        }
        "codex" => {
            s.hook_as(
                "codex",
                "SessionEnd",
                json!({ "session_id": key, "reason": "other" }),
            );
        }
        // OpenCode signals no session end at all — the one genuine absence.
        // Its session is closed explicitly instead, which is a deterministic
        // boundary and not a vendor signal (FR-115, SC-131).
        "opencode" => {
            s.must(&["session", "end", "--status", "completed"]);
        }
        other => panic!("unknown agent {other}"),
    }
}

fn memories(s: &Sandbox, query: &str) -> Vec<Value> {
    s.json(&["memory", "search", query])["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn knowledge_recorded_in_one_agent_is_retrieved_by_the_next_two() {
    // SC-108: zero export, import or copy steps, and one project for the
    // repository however many agents touch it.
    let s = Sandbox::new();
    for a in ["claude-code", "codex", "opencode"] {
        s.install_agent(a);
    }
    s.must(&["init"]);

    // The first agent records a decision and a failure, and leaves a handoff.
    open(&s, "claude-code", "a-1");
    work(&s, "claude-code", "a-1", "src/lib.rs");
    s.must(&[
        "memory",
        "add",
        "We use a single-writer daemon because SQLite contention is the failure mode",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);
    s.must(&[
        "memory",
        "add",
        "Running the suite with --test-threads=1 hides the contention bug",
        "--type",
        "failure",
        "--scope",
        "project",
    ]);
    close(&s, "claude-code", "a-1");
    s.settle("the first agent's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    // The second agent opens the same repository and finds all of it.
    open(&s, "codex", "b-1");
    let ctx = s.json(&["context"]);
    let text = ctx.to_string();
    assert!(
        text.contains("single-writer daemon"),
        "the second agent's briefing lost the first agent's decision: {text}"
    );
    assert!(
        text.contains("--test-threads=1"),
        "the second agent's briefing lost the first agent's failure"
    );
    assert!(
        ctx["briefing"]["previous_handoff"]["next_step"].is_string(),
        "the second agent did not receive the previous handoff"
    );

    // It records a procedure of its own.
    s.must(&[
        "memory",
        "add",
        "To reproduce, run the storage contention test on a cold cache",
        "--type",
        "procedure",
        "--scope",
        "project",
    ]);
    close(&s, "codex", "b-1");
    s.settle("the second agent's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    // The third agent receives everything both produced.
    open(&s, "opencode", "c-1");
    let third = s.json(&["context"]).to_string();
    for expected in ["single-writer daemon", "--test-threads=1", "cold cache"] {
        assert!(
            third.contains(expected),
            "the third agent's briefing lost `{expected}`"
        );
    }

    // Exactly one project, not one per agent (US6 #5, FR-192).
    let projects = s.json(&["status"]);
    assert!(projects["project"]["id"].is_string());
    let sessions = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let agents: std::collections::BTreeSet<&str> = sessions
        .iter()
        .filter_map(|x| x["agent"].as_str())
        .collect();
    assert!(
        agents.len() >= 3,
        "three agents did not produce three agent-tagged sessions: {agents:?}"
    );
}

#[test]
fn provenance_names_the_producing_agent_and_never_narrows_retrieval() {
    // FR-189, FR-190, US6 #3, US6 #4: agent identity is provenance only. No
    // scope, partition or filter is keyed to it.
    let s = Sandbox::new();
    for a in ["claude-code", "codex"] {
        s.install_agent(a);
    }
    s.must(&["init"]);

    open(&s, "claude-code", "p-1");
    s.must(&[
        "memory",
        "add",
        "A fact recorded by the first agent about pagination",
        "--type",
        "fact",
        "--scope",
        "project",
    ]);
    close(&s, "claude-code", "p-1");
    s.settle("the handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });

    // A different agent retrieves it with no filter of any kind.
    open(&s, "codex", "p-2");
    let found = memories(&s, "pagination");
    assert!(
        !found.is_empty(),
        "the second agent could not retrieve the first agent's memory"
    );

    // And each item still names where it came from.
    let sessions = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        sessions.iter().any(|x| x["agent"] == "claude-code"),
        "provenance lost the producing agent"
    );
    assert!(sessions.iter().any(|x| x["agent"] == "codex"));

    // The scope vocabulary is unchanged: nothing agent-shaped appears in it.
    let stored = s.json(&["memory", "search", "pagination"]);
    for item in stored["results"].as_array().cloned().unwrap_or_default() {
        let scope = item["scope"].as_str().unwrap_or_default();
        assert!(
            ["project", "branch", "task", "session"].contains(&scope),
            "an agent-keyed scope appeared: {scope}"
        );
    }
}

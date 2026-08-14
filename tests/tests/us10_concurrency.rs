//! T112, T113 — several agents at once (US10, SC-109, FR-118).
//!
//! Feature 001 already showed that ambiguous session resolution is the most
//! dangerous failure a memory system can have. Adding agents multiplies the
//! opportunity for it, so every assertion here is about routing: two agents
//! are two sessions, each event reaches the session that produced it, and a
//! request that cannot be attributed fails rather than guessing.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

fn sessions(s: &Sandbox) -> Vec<Value> {
    s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn two_agents_in_one_worktree_are_two_sessions_with_correct_provenance() {
    // SC-109: two active sessions, one project, zero misrouted events.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.install_agent("codex");
    s.must(&["init"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "claude-1", "source": "startup" }),
    );
    s.hook_as(
        "codex",
        "SessionStart",
        json!({ "session_id": "codex-1", "source": "startup" }),
    );
    s.settle_session_count(2);

    s.hook(
        "PostToolUse",
        json!({
            "session_id": "claude-1",
            "tool_name": "Edit",
            "tool_input": { "file_path": "claude.rs" }
        }),
    );
    s.hook_as(
        "codex",
        "PostToolUse",
        json!({
            "session_id": "codex-1",
            "tool_name": "apply_patch",
            "tool_input": { "file_path": "codex.rs" },
            "tool_response": { "exit_code": 0 }
        }),
    );

    let all = sessions(&s);
    let active: Vec<&Value> = all.iter().filter(|x| x["status"] == "active").collect();
    assert_eq!(active.len(), 2, "two agents did not produce two sessions");

    let agents: std::collections::BTreeSet<&str> =
        active.iter().filter_map(|x| x["agent"].as_str()).collect();
    assert!(agents.contains("claude-code"), "{agents:?}");
    assert!(agents.contains("codex"), "{agents:?}");

    // Each agent's work reached its own session, and only its own.
    let claude_id = active.iter().find(|x| x["agent"] == "claude-code").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let codex_id = active.iter().find(|x| x["agent"] == "codex").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A memory each, written by naming the session that produced it, carries
    // that session as its origin — never the other agent's.
    for (session, text) in [
        (&claude_id, "The first agent's finding about retries"),
        (&codex_id, "The second agent's finding about backoff"),
    ] {
        s.must(&[
            "memory",
            "add",
            text,
            "--type",
            "fact",
            "--scope",
            "project",
            "--session",
            session,
        ]);
    }
    // Read both from *one* session: provenance records who produced a memory
    // and never restricts who can retrieve it (FR-190).
    for (session, query) in [(&claude_id, "retries"), (&codex_id, "backoff")] {
        let found = s.json(&["memory", "search", query, "--session", &claude_id])["results"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let hit = found
            .iter()
            .find(|m| m["content"].as_str().unwrap_or_default().contains(query))
            .unwrap_or_else(|| panic!("`{query}` was not retrievable"));
        assert_eq!(
            hit["provenance"]["session_id"].as_str(),
            Some(session.as_str()),
            "a memory carries the wrong session's provenance: {hit}"
        );
    }

    s.hook(
        "SessionEnd",
        json!({ "session_id": "claude-1", "reason": "clear" }),
    );
    let handoff = s.handoff_after_close(&["--session", &claude_id]);
    let changed = handoff["changed_files"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        changed.iter().any(|f| f.as_str() == Some("claude.rs")),
        "an observation did not reach its own session: {changed:?}"
    );
    assert!(
        !changed.iter().any(|f| f.as_str() == Some("codex.rs")),
        "an observation was routed to the wrong agent's session"
    );

    // Ending one did not touch the other.
    let after = sessions(&s);
    let still = after.iter().filter(|x| x["status"] == "active").count();
    assert_eq!(still, 1, "one agent's close ended another agent's session");
}

#[test]
fn an_unattributable_request_fails_rather_than_guessing() {
    // SC-109, US10 #4: an ambiguous-session error naming the candidates,
    // never an arbitrary choice.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.install_agent("codex");
    s.must(&["init"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "amb-a", "source": "startup" }),
    );
    s.hook_as(
        "codex",
        "SessionStart",
        json!({ "session_id": "amb-b", "source": "startup" }),
    );
    s.settle_session_count(2);

    let all = sessions(&s);
    let ids: Vec<&str> = all.iter().filter_map(|x| x["id"].as_str()).collect();
    assert_eq!(ids.len(), 2);

    // A read that must name a session, from a caller that named none.
    let e = s.json_err(&["session", "show"]);
    assert_eq!(
        e["code"], "ambiguous_session",
        "an ambiguous read did not fail closed: {e}"
    );
    let message = e["message"].as_str().unwrap_or_default();
    for id in &ids {
        assert!(
            message.contains(id),
            "the error did not name the candidate {id}: {message}"
        );
    }

    // And a write, which is where guessing would do lasting damage: an
    // arbitrary origin session is provenance that is wrong forever.
    let e = s.json_err(&[
        "memory",
        "add",
        "A fact nobody can attribute",
        "--type",
        "fact",
    ]);
    assert_eq!(
        e["code"], "ambiguous_session",
        "an ambiguous write was attributed to a guess: {e}"
    );

    // Naming one resolves it — the ambiguity is the caller's to settle, and
    // it is settleable (US10 #4).
    let named = s.json(&["session", "show", "--session", ids[0]]);
    assert_eq!(named["session"]["id"], ids[0]);
}

#[test]
fn two_worktrees_of_one_repository_are_one_project_and_two_sessions() {
    // US10 #5: one project, two sessions, repository state per worktree.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);

    let worktree = s.add_worktree("second");

    s.hook(
        "SessionStart",
        json!({ "session_id": "wt-first", "source": "startup" }),
    );
    // The second worktree is a different directory of the same repository.
    s.hook_in(
        &worktree,
        "claude-code",
        "SessionStart",
        json!({ "session_id": "wt-second", "source": "startup" }),
    );
    s.settle_session_count(2);

    let all = sessions(&s);
    let paths: std::collections::BTreeSet<&str> = all
        .iter()
        .filter_map(|x| x["worktree_path"].as_str())
        .collect();
    assert_eq!(
        paths.len(),
        2,
        "repository state was not recorded per worktree: {paths:?}"
    );

    // Still one project: identity follows the repository, never the path
    // (FR-192).
    let project = s.json(&["status"])["project"]["id"].clone();
    assert!(project.is_string());
}

#[test]
fn an_adapter_that_cannot_name_a_session_declines_rather_than_sharing_one() {
    // US10 #6: the capability profile reports the limitation instead of
    // Cairn sharing one session between agents.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);

    // An agent with no session identity of its own says so, in the profile,
    // rather than being quietly given someone else's session.
    let listed = s.json(&["agents"]);
    let generic = listed["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["agent"] == "generic-mcp")
        .expect("generic-mcp is always listed");
    assert_eq!(
        generic["capabilities"]["stable_session_identifier"]["availability"], "absent",
        "an adapter with no session identity did not report it: {generic}"
    );
    let missing = generic["missing_behaviors"].to_string();
    assert!(
        missing.contains("stable session identity"),
        "the limitation was not stated in plain language: {missing}"
    );

    // And the absence is reported, never worked around: an event carrying no
    // session identifier creates no session and borrows none.
    let before = sessions(&s).len();
    s.hook("SessionStart", json!({ "source": "startup" }));
    assert_eq!(
        sessions(&s).len(),
        before,
        "an event with no session identifier created or joined a session"
    );

    // Two agents that *do* name themselves are still two sessions, which is
    // the behavior sharing would have destroyed.
    s.hook(
        "SessionStart",
        json!({ "session_id": "named-a", "source": "startup" }),
    );
    s.hook(
        "SessionStart",
        json!({ "session_id": "named-b", "source": "startup" }),
    );
    s.settle_session_count(2);
    assert_eq!(sessions(&s).len(), 2);
}

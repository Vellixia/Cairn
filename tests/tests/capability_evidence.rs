//! T117 — the capability evidence lifecycle (FR-242, FR-245, SC-138, D19a).
//!
//! The report is only worth reading if it can be wrong in Cairn's disfavour.
//! Every assertion here is about a way the level could have been overstated:
//! a capability claimed because the vendor documents it rather than because it
//! was seen; evidence from a build that no longer exists surviving an upgrade;
//! a session identity inferred from one event; a context surface credited for
//! carrying nothing.
//!
//! Driven through the real CLI and daemon so the rule is asserted where it
//! actually runs, not only on the type.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

fn agent(s: &Sandbox, id: &str) -> Value {
    s.json(&["doctor"])["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["agent"] == id)
        .unwrap_or_else(|| panic!("{id} is not reported"))
}

fn evidence(s: &Sandbox, id: &str) -> Value {
    agent(s, id)["evidence"].clone()
}

/// Set the version the agent reports, the way its own configuration does.
///
/// Merged rather than written over: `~/.claude.json` is also where the
/// user-scope MCP entry lives, and clobbering it would make this a test about
/// a missing integration rather than about a version change.
fn set_claude_version(s: &Sandbox, version: &str) {
    let path = s.fake_home().join(".claude.json");
    let mut value: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    value["version"] = json!(version);
    std::fs::write(&path, value.to_string()).expect("write version");
}

/// One ordinary session: open, use a tool, go quiet, close.
fn full_session(s: &Sandbox, key: &str) {
    s.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    s.settle_session_count(1);
    s.hook(
        "PostToolUse",
        json!({
            "session_id": key,
            "tool_name": "Edit",
            "tool_input": { "file_path": "src/pool.rs" }
        }),
    );
    s.hook("Stop", json!({ "session_id": key }));
    s.settle_turn_checkpoint();
    s.hook(
        "SessionEnd",
        json!({ "session_id": key, "reason": "clear" }),
    );
    s.settle("the sealed boundary's handoff", |s| {
        s.cairn(&["--json", "status"])
            .stdout
            .contains("\"sessions_awaiting_handoff\": 0")
    });
}

#[test]
fn an_unobserved_capability_keeps_the_level_below_full_and_is_named() {
    // SC-138 first clause. Everything is installed; nothing has run.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    set_claude_version(&s, "2.1.220");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    let a = agent(&s, "claude-code");
    assert_ne!(
        a["level"], "full",
        "FULL was granted before anything was observed: {a}"
    );
    let awaited = a["awaited_behaviors"].to_string();
    for expected in [
        "first session opened",
        "first tool call",
        "first session closed",
    ] {
        assert!(
            awaited.contains(expected),
            "the report does not name `{expected}` as awaited: {awaited}"
        );
    }
    // Configuration is established by reading Cairn's own artifacts back, and
    // that half needs no session at all.
    let e = evidence(&s, "claude-code");
    assert_eq!(e["mcp"]["kind"], "introspection", "{e}");
    assert_eq!(e["instructions"]["kind"], "introspection");
}

#[test]
fn full_is_granted_once_every_required_capability_is_established() {
    // SC-138 third clause, and the whole point of the two-dimensional model:
    // one ordinary session is what promotes the integration, not a claim.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    set_claude_version(&s, "2.1.220");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    full_session(&s, "evidence-1");

    let a = agent(&s, "claude-code");
    assert_eq!(
        a["level"], "full",
        "an ordinary session did not establish the integration: {a}"
    );
    assert_eq!(a["completion_guarantee"], "demonstrated");
    assert!(
        a["awaited_behaviors"]
            .as_array()
            .is_none_or(|v| v.is_empty()),
        "FULL was reported with behaviors still awaited: {a}"
    );
    assert!(
        a["session_completion"].is_null(),
        "a demonstrated guarantee still explained itself away: {a}"
    );

    let e = evidence(&s, "claude-code");
    for capability in [
        "lifecycle_session_open",
        "lifecycle_tool_success",
        "lifecycle_quiesce",
        "lifecycle_session_close",
        "context_at_session_open",
        "stable_session_identifier",
    ] {
        assert_eq!(
            e[capability]["kind"], "observation",
            "{capability} was not established by observation: {e}"
        );
    }
}

#[test]
fn an_agent_version_change_discards_observation_evidence_and_only_that() {
    // SC-138 second clause. This is the hole the two-dimensional model was
    // built to close: a vendor update that removed a capability used to leave
    // FULL standing, because static availability never changed and nothing
    // consulted what had actually been seen.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    set_claude_version(&s, "2.1.220");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);
    full_session(&s, "evidence-2");
    assert_eq!(agent(&s, "claude-code")["level"], "full");

    let before = evidence(&s, "claude-code");
    let introspection: Vec<String> = before
        .as_object()
        .expect("evidence")
        .iter()
        .filter(|(_, v)| v["kind"] == "introspection")
        .map(|(k, _)| k.clone())
        .collect();
    assert!(!introspection.is_empty(), "{before}");

    // The developer upgrades their agent.
    set_claude_version(&s, "2.2.0");
    let a = agent(&s, "claude-code");

    assert_ne!(
        a["level"], "full",
        "FULL survived an agent upgrade on evidence from the previous build: {a}"
    );
    assert!(
        !a["awaited_behaviors"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .is_empty(),
        "the level dropped without saying what it is waiting for: {a}"
    );

    let after = evidence(&s, "claude-code");
    for (capability, entry) in before.as_object().expect("evidence") {
        if entry["kind"] == "observation" {
            assert!(
                after[capability].is_null(),
                "observation evidence for {capability} survived a version change"
            );
        }
    }
    for capability in &introspection {
        assert_eq!(
            after[capability]["kind"], "introspection",
            "introspection evidence for {capability} was discarded with the version"
        );
    }

    // And the integration keeps working: nothing was reinstalled, nothing is
    // unsupported, and one more ordinary session restores FULL.
    assert_eq!(a["compatibility"], "compatible_unverified");
    full_session(&s, "evidence-3");
    assert_eq!(
        agent(&s, "claude-code")["level"],
        "full",
        "the integration did not recover after re-observing the new build"
    );
}

#[test]
fn a_session_start_that_delivered_no_context_establishes_nothing() {
    // D19a: the capability is about the agent's context surface carrying
    // Cairn's briefing. A session that opened before there was any project to
    // brief demonstrates nothing about that surface.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    // No project registered: the daemon has nothing to deliver.
    let outside = s.repo_dir().parent().expect("a parent").to_path_buf();
    s.hook_in(
        &outside,
        "claude-code",
        "SessionStart",
        json!({ "session_id": "no-context", "source": "startup" }),
    );

    let e = evidence(&s, "claude-code");
    assert!(
        e["context_at_session_open"].is_null(),
        "context delivery was established without a delivery: {e}"
    );
    assert!(agent(&s, "claude-code")["awaited_behaviors"]
        .to_string()
        .contains("context delivered"));
}

#[test]
fn a_degraded_delivery_establishes_the_capability_and_records_that_it_was_degraded() {
    // The other half of D19a. Something reached the agent, so the surface
    // demonstrably carries context; Cairn's own assembly is what fell short,
    // and the evidence says so rather than quietly reading as a clean result.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    set_claude_version(&s, "2.1.220");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "degraded-1", "source": "startup" }),
    );
    s.settle("the delivery to be recorded", |s| {
        !evidence(s, "claude-code")["context_at_session_open"].is_null()
    });
    let e = evidence(&s, "claude-code");
    assert_eq!(e["context_at_session_open"]["kind"], "observation", "{e}");
    // Recorded either way, and the record says which it was — a delivery that
    // is silently indistinguishable from a full one is the failure mode.
    assert!(
        e["context_at_session_open"]["degraded"].is_boolean(),
        "the delivery outcome was not recorded at all: {e}"
    );
    assert_eq!(
        e["context_at_session_open"]["degraded"], false,
        "an ordinary delivery was recorded as degraded: {e}"
    );

    // And the evidence carries the agent version it belongs to, so the next
    // upgrade discards it rather than leaving a versionless row that reads as
    // evidence about every build (FR-245).
    assert_eq!(
        e["context_at_session_open"]["agent_version"],
        agent(&s, "claude-code")["version"],
        "delivery evidence was recorded against no version: {e}"
    );
}

#[test]
fn one_event_carrying_an_identifier_does_not_establish_a_stable_identity() {
    // D19a: two or more events, of at least two different kinds, on one
    // vendor-supplied key. A single event proves the vendor sent a string
    // once, which is not the same as the identity being stable — and Feature
    // 001's synthesized fallback must never reach this at all.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "single-event", "source": "startup" }),
    );
    s.settle_session_count(1);
    let e = evidence(&s, "claude-code");
    assert!(
        e["stable_session_identifier"].is_null(),
        "one event established a stable session identity: {e}"
    );

    // A second event of the *same* kind is still one kind.
    s.hook(
        "SessionStart",
        json!({ "session_id": "single-event", "source": "resume" }),
    );
    let e = evidence(&s, "claude-code");
    assert!(
        e["stable_session_identifier"].is_null(),
        "two events of one kind established a stable session identity: {e}"
    );

    // A second kind on the same key is what establishes it.
    s.hook(
        "PostToolUse",
        json!({
            "session_id": "single-event",
            "tool_name": "Read",
            "tool_input": { "file_path": "src/pool.rs" }
        }),
    );
    s.settle("the identity to be established", |s| {
        !evidence(s, "claude-code")["stable_session_identifier"].is_null()
    });
}

#[test]
fn a_session_cairn_named_itself_never_establishes_a_stable_identity() {
    // Feature 001 synthesizes a key for a CLI-driven session. That key is
    // Cairn's, not the agent's, and crediting it would let an integration
    // reach FULL on an identity the agent never supplied.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);
    s.must(&["connect", "claude-code", "--yes"]);

    s.must(&["session", "start"]);
    s.must(&[
        "memory",
        "add",
        "A fact recorded from the CLI",
        "--type",
        "fact",
    ]);
    s.must(&["session", "end", "--status", "completed"]);

    let e = evidence(&s, "claude-code");
    assert!(
        e["stable_session_identifier"].is_null(),
        "a Cairn-synthesized session key was credited to the agent: {e}"
    );
    assert_ne!(
        agent(&s, "claude-code")["level"],
        "full",
        "CLI-driven sessions promoted an integration that has observed nothing"
    );
}

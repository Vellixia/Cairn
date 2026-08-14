//! T109 — the plain MCP client (US8, FR-110, FR-131, SC-107).
//!
//! An agent Cairn has never heard of, with no adapter, no hooks and no
//! plugin, connecting over stdio and speaking only the protocol. It must get
//! the whole tool surface, receive the usage contract where the protocol
//! carries it, and be told — plainly and in the developer's terms — that
//! automatic lifecycle and automatic capture are the two things it does not
//! get.
//!
//! The last part is the one worth testing. An integration that works is easy
//! to describe generously, and `MCP_ONLY` described as "connected" would be
//! true and useless: the developer would wonder for a week why nothing was
//! being captured.

use cairn_e2e::{Mcp, Sandbox};
use serde_json::{json, Value};

fn generic(s: &Sandbox) -> Value {
    s.json(&["agents"])["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["agent"] == "generic-mcp")
        .expect("the generic MCP path is always reported")
}

#[test]
fn a_plain_client_initializes_and_receives_the_usage_contract() {
    // FR-131, D34: `InitializeResult.instructions` is where the protocol
    // itself carries usage guidance, so a client with no instruction file and
    // no Skill still learns how to use the tools.
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);

    let init = mcp.call("initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "cairn");

    let instructions = init["instructions"]
        .as_str()
        .expect("the initialize result carries the usage contract");
    assert!(
        instructions.len() > 200,
        "the contract is a stub: {instructions}"
    );
    // It is the contract, not a description of the product: it tells the agent
    // what to do and when.
    for tool in ["cairn_context", "cairn_remember", "cairn_search"] {
        assert!(
            instructions.contains(tool),
            "the contract does not mention {tool}: {instructions}"
        );
    }
    // And it carries no secret and no absolute path from this machine.
    assert!(!instructions.contains(&s.repo_dir().display().to_string()));
}

#[test]
fn all_six_tools_behave_as_feature_001_defines_them() {
    // US8 #2: the tool surface is the whole integration for this client, so
    // every one of the six has to work without a single hook firing.
    let s = Sandbox::new();
    let cwd = s.repo_path().display().to_string();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));

    let names: Vec<String> = mcp.call("tools/list", json!({}))["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    assert_eq!(names.len(), 6, "{names:?}");

    // 1. session
    let started = mcp.tool(
        "cairn_session",
        json!({ "action": "start", "agent": "some-unknown-agent", "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(started.contains("\"status\": \"active\""), "{started}");

    // 2. task
    let task: Value = serde_json::from_str(&mcp.tool(
        "cairn_task",
        json!({ "action": "create", "title": "Generic work", "goal": "prove the plain path",
                "acceptance_criteria": ["it works"] }),
        &cwd,
    ))
    .expect("task json");
    let task_id = task["task"]["id"].as_str().expect("task id").to_string();

    // 3. remember
    let remembered = mcp.tool(
        "cairn_remember",
        json!({ "action": "create", "type": "decision", "scope": "project",
                "content": "The plain client records memory like any other",
                "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(remembered.contains("records memory like any other"));

    // 4. search — and it finds what any other agent recorded, unfiltered.
    let found: Value = serde_json::from_str(&mcp.tool(
        "cairn_search",
        json!({ "query": "plain client", "agent_session_key": "g-1" }),
        &cwd,
    ))
    .expect("search json");
    assert!(
        !found["results"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .is_empty(),
        "the plain client could not retrieve its own memory: {found}"
    );

    // 5. context — and it leads with the bound task.
    mcp.tool(
        "cairn_session",
        json!({ "action": "bind_task", "agent_session_key": "g-1", "task_id": task_id }),
        &cwd,
    );
    let context = mcp.tool(
        "cairn_context",
        json!({ "reason": "session_start", "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(context.contains("prove the plain path"), "{context}");

    // 6. handoff — produced on request, since nothing will produce it
    // automatically here.
    let handoff = mcp.tool(
        "cairn_handoff",
        json!({ "action": "generate", "trigger": "session_end", "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(
        handoff.contains("\"trigger\": \"session_end\""),
        "{handoff}"
    );
}

#[test]
fn the_level_is_mcp_only_and_says_what_is_unavailable() {
    // SC-107, US8 #3: never described as full, and the two missing behaviors
    // are named in plain language rather than as a score.
    let s = Sandbox::new();
    s.must(&["init"]);

    let a = generic(&s);
    assert_eq!(a["level"], "mcp_only", "{a}");
    assert_eq!(a["completion_guarantee"], "not_demonstrated");

    let missing = a["missing_behaviors"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|b| b.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    for expected in [
        "automatic session start",
        "automatic capture of tool calls",
        "automatic session completion",
    ] {
        assert!(
            missing.iter().any(|b| b == expected),
            "the report does not name `{expected}`: {missing:?}"
        );
    }
    // Plain language, never a number.
    for b in &missing {
        assert!(
            !b.chars().any(|c| c.is_ascii_digit()),
            "a missing behavior reads like a score: {b}"
        );
    }

    // Every lifecycle capability is absent, and none is merely unverified —
    // there is nothing here that a future session could establish.
    for capability in [
        "lifecycle_session_open",
        "lifecycle_tool_success",
        "lifecycle_session_close",
        "stable_session_identifier",
    ] {
        assert_eq!(
            a["capabilities"][capability]["availability"], "absent",
            "{capability} is not absent for a client with no adapter: {a}"
        );
    }
}

#[test]
fn nothing_describes_the_generic_path_as_full() {
    // FR-110: the word is reserved. Neither the human report nor the JSON may
    // call this integration full, complete, or fully connected.
    let s = Sandbox::new();
    s.must(&["init"]);

    let text = s.must(&["agents"]).stdout.to_lowercase();
    let line = text
        .lines()
        .find(|l| l.starts_with("generic-mcp"))
        .unwrap_or_default()
        .to_string();
    assert!(!line.is_empty(), "the generic path is not reported: {text}");
    for forbidden in ["full", "complete", "fully connected"] {
        assert!(
            !line.contains(forbidden),
            "the generic path was described as `{forbidden}`: {line}"
        );
    }
    // It says what it is instead.
    assert!(
        line.contains("mcp_only") || line.contains("mcp-only"),
        "{line}"
    );

    let doctor = s.cairn(&["--json", "doctor"]).stdout;
    let value: Value = serde_json::from_str(&doctor).expect("envelope");
    let entry = value["data"]["agents"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|a| a["agent"] == "generic-mcp")
        .expect("reported by doctor too");
    assert_ne!(entry["level"], "full");
}

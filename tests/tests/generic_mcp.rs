//! A plain MCP client works without lifecycle hooks.

use cairn_e2e::{Mcp, Sandbox};
use serde_json::{json, Value};

#[test]
fn a_plain_client_initializes_and_receives_the_usage_contract() {
    let sandbox = Sandbox::new();
    let mut mcp = Mcp::start(&sandbox);

    let init = mcp.call("initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "cairn");
    let instructions = init["instructions"].as_str().expect("usage contract");
    for tool in ["cairn_context", "cairn_remember", "cairn_search"] {
        assert!(instructions.contains(tool), "missing {tool}: {instructions}");
    }
}

#[test]
fn all_five_tools_behave_without_hooks() {
    let sandbox = Sandbox::new();
    let cwd = sandbox.repo_path().display().to_string();
    let mut mcp = Mcp::start(&sandbox);
    mcp.call("initialize", json!({}));

    let names: Vec<String> = mcp.call("tools/list", json!({}))["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect();
    assert_eq!(names.len(), 5, "{names:?}");

    let started = mcp.tool(
        "cairn_session",
        json!({ "action": "start", "agent": "some-unknown-agent", "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(started.contains("\"status\": \"active\""), "{started}");

    let remembered = mcp.tool(
        "cairn_remember",
        json!({ "action": "create", "type": "decision", "scope": "project",
                "content": "The plain client records memory like any other",
                "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(remembered.contains("records memory like any other"));

    let found: Value = serde_json::from_str(&mcp.tool(
        "cairn_search",
        json!({ "query": "plain client", "agent_session_key": "g-1" }),
        &cwd,
    ))
    .expect("search JSON");
    assert!(!found["results"].as_array().unwrap_or(&Vec::new()).is_empty());

    let context = mcp.tool(
        "cairn_context",
        json!({ "reason": "session_start", "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(context.contains("# Cairn context"), "{context}");

    let handoff = mcp.tool(
        "cairn_handoff",
        json!({ "action": "generate", "trigger": "session_end", "agent_session_key": "g-1" }),
        &cwd,
    );
    assert!(handoff.contains("\"trigger\": \"session_end\""), "{handoff}");
}

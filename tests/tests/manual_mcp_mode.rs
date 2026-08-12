//! T079 — an MCP-compatible agent with no lifecycle hooks (FR-040, FR-042).
//!
//! Sessions, tasks, memory, context and handoff generation all work; only
//! automatic observation capture is unavailable.

use cairn_e2e::{Mcp, Sandbox};
use serde_json::{json, Value};

#[test]
fn the_server_advertises_exactly_six_tools() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);

    let init = mcp.call("initialize", json!({ "protocolVersion": "2025-06-18" }));
    assert_eq!(init["serverInfo"]["name"], "cairn");
    assert!(init["capabilities"]["tools"].is_object());

    let listed = mcp.call("tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 6, "FR-040 caps the surface at six: {names:?}");
    assert_eq!(
        names,
        vec![
            "cairn_context",
            "cairn_search",
            "cairn_remember",
            "cairn_session",
            "cairn_task",
            "cairn_handoff"
        ]
    );
}

#[test]
fn an_agent_without_hooks_can_work_end_to_end() {
    let s = Sandbox::new();
    let cwd = s.repo_path().display().to_string();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));

    // Start a session of its own.
    let started = mcp.tool(
        "cairn_session",
        json!({ "action": "start", "agent": "some-mcp-agent", "agent_session_key": "manual-1" }),
        &cwd,
    );
    assert!(started.contains("\"status\": \"active\""), "{started}");

    // Create and bind a task.
    let task = mcp.tool(
        "cairn_task",
        json!({ "action": "create", "title": "Manual work", "goal": "prove manual mode",
                "acceptance_criteria": ["it works"] }),
        &cwd,
    );
    let parsed: Value = serde_json::from_str(&task).expect("task json");
    let task_id = parsed["task"]["id"].as_str().expect("task id").to_string();
    mcp.tool(
        "cairn_session",
        json!({ "action": "bind_task", "agent_session_key": "manual-1", "task_id": task_id }),
        &cwd,
    );

    // Record memory with no evidence — valid in manual mode (FR-019).
    let remembered = mcp.tool(
        "cairn_remember",
        json!({ "action": "create", "type": "convention", "scope": "project",
                "content": "manual mode records memory too",
                "agent_session_key": "manual-1" }),
        &cwd,
    );
    assert!(remembered.contains("manual mode records memory"));

    let found = mcp.tool(
        "cairn_search",
        json!({ "query": "manual mode", "agent_session_key": "manual-1" }),
        &cwd,
    );
    let search: Value = serde_json::from_str(&found).expect("search json");
    assert_eq!(search["results"][0]["provenance"]["evidence_count"], 0);
    assert!(search["results"][0]["provenance"]["session_id"].is_string());

    // Context works, and leads with the bound task.
    let context = mcp.tool(
        "cairn_context",
        json!({ "reason": "refresh", "agent_session_key": "manual-1" }),
        &cwd,
    );
    assert!(context.contains("prove manual mode"), "{context}");

    // Handoff generation works without any hook having fired.
    let handoff = mcp.tool(
        "cairn_handoff",
        json!({ "action": "generate", "trigger": "session_end",
                "agent_session_key": "manual-1" }),
        &cwd,
    );
    assert!(
        handoff.contains("\"trigger\": \"session_end\""),
        "{handoff}"
    );

    let latest = mcp.tool(
        "cairn_handoff",
        json!({ "action": "latest", "agent_session_key": "manual-1" }),
        &cwd,
    );
    assert!(latest.contains("prove manual mode"));
}

#[test]
fn status_reports_which_mode_the_repository_is_in() {
    // FR-042: Cairn says whether hooks are installed.
    let s = Sandbox::new();
    assert_eq!(s.json(&["status"])["integration_mode"], "manual-mcp");

    // Non-interactive runs need the explicit opt-in: the plan is shown and
    // nothing is applied without it (FR-164).
    s.must(&["connect", "claude-code", "--yes"]);
    assert_eq!(s.json(&["status"])["integration_mode"], "claude-code-hooks");

    s.must(&["disconnect", "claude-code"]);
    assert_eq!(s.json(&["status"])["integration_mode"], "manual-mcp");
}

//! Capture the pre-feature baseline (T004).
//!
//! Two Feature 003 suites are no-regression claims against what Cairn produced
//! *before* the feature existed:
//!
//! - `us10_min_safe_context::no_regression` — the `briefing` object,
//!   `estimated_tokens`, `truncated` and `omitted_sections` for a project with
//!   no task, no warnings, no pins and no checkpoint (metric 13, SC-308)
//! - `mcp_backward_compatibility` — every pre-existing field of every Feature
//!   001/002 tool response (metric 36, SC-323)
//!
//! Both are meaningless if the baseline is recaptured after the change it
//! exists to detect. So the capture is an **ignored** test, run once against
//! the pre-feature build, and its output is committed:
//!
//! ```text
//! cargo test -p cairn-e2e --test baseline_capture -- --ignored
//! ```
//!
//! The guard test below is not ignored: it fails if the committed baseline is
//! missing or has been emptied.

use cairn_e2e::{baseline, Mcp, Sandbox};
use serde_json::{json, Value};

#[test]
#[ignore = "regenerates a committed artifact; run deliberately against a pre-feature build"]
fn capture_the_pre_feature_baseline() {
    capture_briefing();
    capture_mcp_calls();
}

/// A project with nothing Feature 003 would add: no task, no warnings, no
/// pins, no checkpoint. This is the assembly that must stay byte-identical,
/// because an unspent Level 0 reserve returns to the general pool (FR-442).
fn capture_briefing() {
    let s = Sandbox::new();
    let out = s.cairn(&["context", "--json"]);
    assert!(out.ok(), "cairn context failed: {}", out.stderr);
    let full: Value = serde_json::from_str(&out.stdout).expect("context --json is JSON");

    let data = if full.get("data").is_some() {
        full["data"].clone()
    } else {
        full.clone()
    };

    let subset = json!({
        "briefing": data.get("briefing").cloned().unwrap_or(Value::Null),
        "estimated_tokens": data.get("estimated_tokens").cloned().unwrap_or(Value::Null),
        "truncated": data.get("truncated").cloned().unwrap_or(Value::Null),
        "omitted_sections": data.get("omitted_sections").cloned().unwrap_or(Value::Null),
    });

    baseline::record("briefing.json", &baseline::normalize(&subset));
}

/// One call per Feature 001/002 surface, in the order a session actually makes
/// them, recorded request and response together.
fn capture_mcp_calls() {
    let s = Sandbox::new();
    let cwd = s.repo_path().display().to_string();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));

    let mut recorded: Vec<Value> = Vec::new();

    // `tools/list` is its own record: the six-tool surface is a Feature 001
    // guarantee, and a seventh tool would show up here first (FR-495).
    let listed = mcp.call("tools/list", json!({}));
    recorded.push(json!({
        "method": "tools/list",
        "params": {},
        "result": baseline::normalize(&listed),
    }));

    let key = "baseline-1";
    let call = |mcp: &mut Mcp, recorded: &mut Vec<Value>, name: &str, args: Value| -> Value {
        let result = mcp.tool_result(name, args.clone(), &cwd);
        recorded.push(json!({
            "method": "tools/call",
            "params": baseline::normalize(&json!({ "name": name, "arguments": args })),
            "result": baseline::normalize(&result),
        }));
        result
    };

    call(
        &mut mcp,
        &mut recorded,
        "cairn_session",
        json!({ "action": "start", "agent": "claude-code", "agent_session_key": key }),
    );

    let task = call(
        &mut mcp,
        &mut recorded,
        "cairn_task",
        json!({ "action": "create", "title": "Baseline work", "goal": "record the baseline",
                "acceptance_criteria": ["the briefing renders", "the tools answer"] }),
    );
    let task_id = task["content"][0]["text"]["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    call(
        &mut mcp,
        &mut recorded,
        "cairn_session",
        json!({ "action": "bind_task", "agent_session_key": key, "task_id": task_id }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_task",
        json!({ "action": "get", "task_id": task_id }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_task",
        json!({ "action": "update", "task_id": task_id, "status": "in_progress" }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_remember",
        json!({ "action": "create", "type": "decision", "scope": "project",
                "content": "The baseline records what Feature 001 already produced.",
                "agent_session_key": key }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_search",
        json!({ "query": "baseline", "agent_session_key": key }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_context",
        json!({ "reason": "session_start", "agent_session_key": key }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_handoff",
        json!({ "action": "generate", "trigger": "session_end", "agent_session_key": key }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_handoff",
        json!({ "action": "latest", "agent_session_key": key }),
    );
    call(
        &mut mcp,
        &mut recorded,
        "cairn_session",
        json!({ "action": "status", "agent_session_key": key }),
    );

    baseline::record("mcp_calls.json", &json!({ "calls": recorded }));
}

#[test]
fn the_committed_baseline_is_present() {
    let briefing = baseline::load("briefing.json");
    assert!(
        briefing.get("briefing").is_some(),
        "briefing.json has no briefing object"
    );
    assert!(
        briefing["estimated_tokens"].is_number(),
        "briefing.json records no estimated_tokens"
    );

    let calls = baseline::load("mcp_calls.json");
    let recorded = calls["calls"].as_array().expect("calls array");
    assert!(
        recorded.len() >= 10,
        "the recorded corpus is too small to be a comparison: {} calls",
        recorded.len()
    );

    // Every one of the six tools appears, or the comparison has a blind spot.
    let names: Vec<&str> = recorded
        .iter()
        .filter_map(|c| c["params"]["name"].as_str())
        .collect();
    for tool in [
        "cairn_session",
        "cairn_task",
        "cairn_remember",
        "cairn_search",
        "cairn_context",
        "cairn_handoff",
    ] {
        assert!(
            names.contains(&tool),
            "the recorded corpus never calls {tool}"
        );
    }
}

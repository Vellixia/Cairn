//! T048 — work organized by task (FR-036 – FR-038).

use cairn_e2e::Sandbox;
use serde_json::json;

#[test]
fn a_task_carries_goal_criteria_and_status() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Add rate limiting",
        "--goal",
        "Requests over the limit get 429",
        "--criterion",
        "429 returned above threshold",
        "--criterion",
        "Limit is configurable",
    ]);
    let task = &created["task"];
    assert_eq!(task["status"], "todo");
    assert_eq!(task["acceptance_criteria"].as_array().unwrap().len(), 2);

    let id = task["id"].as_str().unwrap().to_string();
    s.must(&["task", "set-status", &id, "in_progress"]);
    let listed = s.json(&["task", "list", "--status", "in_progress"]);
    assert_eq!(listed["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn a_task_bound_session_leads_the_briefing_with_its_goal_and_criteria() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Add rate limiting",
        "--goal",
        "Requests over the limit get 429",
        "--criterion",
        "429 returned above threshold",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();

    s.hook(
        "SessionStart",
        json!({ "session_id": "t1", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "t1", "--task", &id]);
    s.write_file("src/limiter.rs", "pub fn limit() {}\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": "t1", "tool_name": "Edit", "tool_input": { "file_path": "src/limiter.rs" } }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "t1", "reason": "clear" }),
    );

    // A second session on the same task resumes with its goal and history.
    s.hook(
        "SessionStart",
        json!({ "session_id": "t2", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "t2", "--task", &id]);

    let ctx = s.json(&["context"]);
    let task = &ctx["briefing"]["task"];
    assert_eq!(task["goal"], "Requests over the limit get 429");
    assert_eq!(task["acceptance_criteria"].as_array().unwrap().len(), 1);
    assert!(
        ctx["briefing"]["previous_handoff"]["session_id"].is_string(),
        "the previous session on this task should be carried forward"
    );
}

#[test]
fn unmet_criteria_appear_as_remaining_work_in_the_handoff() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Rate limit",
        "--goal",
        "429 over the limit",
        "--criterion",
        "429 above threshold",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();

    s.hook(
        "SessionStart",
        json!({ "session_id": "t3", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "t3", "--task", &id]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "t3", "reason": "clear" }),
    );

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(handoff["goal"], "429 over the limit");
    let remaining = serde_json::to_string(&handoff["remaining_work"]).unwrap();
    assert!(remaining.contains("429 above threshold"), "{remaining}");
}

#[test]
fn a_session_with_no_task_remains_valid() {
    // FR-038: sessions without a task are scoped to project and branch only.
    let s = Sandbox::new();
    s.hook(
        "SessionStart",
        json!({ "session_id": "free", "source": "startup" }),
    );
    s.hook(
        "PostToolUse",
        json!({ "session_id": "free", "tool_name": "Read", "tool_input": { "file_path": "README.md" } }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "free", "reason": "clear" }),
    );

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert!(
        handoff["goal"].as_str().unwrap().contains("main"),
        "goal falls back to the branch"
    );
    assert!(s.cairn(&["context"]).ok());
}

#[test]
fn a_task_can_be_updated_via_mcp() {
    // TaskUpdate is wired but the CLI only exposes set-status. The MCP
    // tool exposes the full update surface (title, goal, criteria, status).
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Original title",
        "--goal",
        "Original goal",
        "--criterion",
        "Original criterion",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();

    let mut mcp = cairn_e2e::Mcp::start(&s);
    let _ = mcp.call(
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" }
        }),
    );

    let updated = mcp.tool(
        "cairn_task",
        json!({
            "action": "update",
            "task_id": id,
            "title": "Updated title",
            "goal": "Updated goal",
            "acceptance_criteria": ["Updated criterion A", "Updated criterion B"],
            "status": "in_progress",
        }),
        s.repo_path().to_str().unwrap(),
    );
    assert!(updated.contains("Updated title"), "{updated}");
    assert!(updated.contains("Updated goal"), "{updated}");
    assert!(updated.contains("Updated criterion A"), "{updated}");
    assert!(updated.contains("in_progress"), "{updated}");

    // Verify via CLI.
    let shown = s.json(&["task", "show", &id]);
    assert_eq!(shown["task"]["title"], "Updated title");
    assert_eq!(shown["task"]["goal"], "Updated goal");
    assert_eq!(shown["task"]["status"], "in_progress");
    assert_eq!(
        shown["task"]["acceptance_criteria"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_task_marked_done_empties_remaining_work_in_the_handoff() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Done task",
        "--goal",
        "All done",
        "--criterion",
        "Everything works",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();
    s.must(&["task", "set-status", &id, "done"]);

    s.hook(
        "SessionStart",
        json!({ "session_id": "done-sess", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "done-sess", "--task", &id]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "done-sess", "reason": "clear" }),
    );

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(handoff["goal"], "All done");
    let remaining = handoff["remaining_work"].as_array().unwrap();
    assert!(
        remaining.is_empty(),
        "a done task should have no remaining work: {remaining:?}"
    );
}

#[test]
fn a_task_with_no_criteria_is_valid() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "No criteria",
        "--goal",
        "Just a goal",
    ]);
    let task = &created["task"];
    assert_eq!(task["status"], "todo");
    assert!(task["acceptance_criteria"].as_array().unwrap().is_empty());

    let id = task["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionStart",
        json!({ "session_id": "nc", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "nc", "--task", &id]);
    s.hook(
        "SessionEnd",
        json!({ "session_id": "nc", "reason": "clear" }),
    );

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(handoff["goal"], "Just a goal");
    assert!(handoff["remaining_work"].as_array().unwrap().is_empty());
}

#[test]
fn multiple_tasks_can_coexist_and_be_listed_by_status() {
    let s = Sandbox::new();
    s.json(&["task", "new", "--title", "Task A", "--goal", "Goal A"]);
    s.json(&["task", "new", "--title", "Task B", "--goal", "Goal B"]);
    s.json(&["task", "new", "--title", "Task C", "--goal", "Goal C"]);

    let all = s.json(&["task", "list"]);
    assert_eq!(all["tasks"].as_array().unwrap().len(), 3);

    let created = s.json(&["task", "new", "--title", "In progress", "--goal", "IP"]);
    let ip_id = created["task"]["id"].as_str().unwrap().to_string();
    s.must(&["task", "set-status", &ip_id, "in_progress"]);

    let ip = s.json(&["task", "list", "--status", "in_progress"]);
    assert_eq!(ip["tasks"].as_array().unwrap().len(), 1);

    let todo = s.json(&["task", "list", "--status", "todo"]);
    assert_eq!(todo["tasks"].as_array().unwrap().len(), 3);
}

#[test]
fn a_task_can_be_blocked() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Blocked task",
        "--goal",
        "Waiting on dep",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();
    s.must(&["task", "set-status", &id, "blocked"]);

    let shown = s.json(&["task", "show", &id]);
    assert_eq!(shown["task"]["status"], "blocked");

    let blocked = s.json(&["task", "list", "--status", "blocked"]);
    assert_eq!(blocked["tasks"].as_array().unwrap().len(), 1);
}

#[test]
fn a_task_spanning_multiple_sessions_accumulates_history() {
    let s = Sandbox::new();
    let created = s.json(&[
        "task",
        "new",
        "--title",
        "Multi-session",
        "--goal",
        "Work across sessions",
        "--criterion",
        "Session 1 work",
        "--criterion",
        "Session 2 work",
    ]);
    let id = created["task"]["id"].as_str().unwrap().to_string();

    // Session 1.
    s.hook(
        "SessionStart",
        json!({ "session_id": "ms1", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "ms1", "--task", &id]);
    s.write_file("src/a.rs", "// session 1\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": "ms1", "tool_name": "Edit", "tool_input": { "file_path": "src/a.rs" } }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "ms1", "reason": "clear" }),
    );

    // Session 2 — should carry forward the previous handoff.
    s.hook(
        "SessionStart",
        json!({ "session_id": "ms2", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "ms2", "--task", &id]);
    s.write_file("src/b.rs", "// session 2\n");
    s.hook(
        "PostToolUse",
        json!({ "session_id": "ms2", "tool_name": "Edit", "tool_input": { "file_path": "src/b.rs" } }),
    );
    s.hook(
        "SessionEnd",
        json!({ "session_id": "ms2", "reason": "clear" }),
    );

    let handoff = s.json(&["handoff", "show"])["handoff"].clone();
    assert_eq!(handoff["goal"], "Work across sessions");
    let changed = handoff["changed_files"].as_array().unwrap();
    assert!(changed.iter().any(|f| f.as_str() == Some("src/b.rs")));
}

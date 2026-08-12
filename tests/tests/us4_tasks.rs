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

    let handoff = s.handoff_after_close(&[]);
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

    let handoff = s.handoff_after_close(&[]);
    assert!(
        handoff["goal"].as_str().unwrap().contains("main"),
        "goal falls back to the branch"
    );
    assert!(s.cairn(&["context"]).ok());
}

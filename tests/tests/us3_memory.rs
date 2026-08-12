//! T042 — durable, scoped memory and recall (SC-004 – SC-006, SC-012, FR-019).

use cairn_e2e::{Mcp, Sandbox};
use serde_json::json;

fn seed(s: &Sandbox) -> String {
    let task = s.json(&[
        "task",
        "new",
        "--title",
        "Rate limiting",
        "--goal",
        "429 over the limit",
    ]);
    let task_id = task["task"]["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionStart",
        json!({ "session_id": "m", "source": "startup" }),
    );
    let session = s.json(&["session", "list"])["sessions"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.must(&["session", "start", "--key", "m", "--task", &task_id]);
    let _ = session;
    task_id
}

#[test]
fn recall_favours_task_then_branch_then_project() {
    let s = Sandbox::new();
    let task_id = seed(&s);

    // The project-scoped entry is the strongest lexical match, deliberately.
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "tests tests tests fail without the local queue",
    ]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "failure",
        "--scope",
        "branch",
        "tests need a fixture on this branch",
    ]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "task",
        "--scope-key",
        &task_id,
        "tests here run against the limiter",
    ]);

    let out = s.json(&["memory", "search", "tests"]);
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(
        results[0]["scope"], "task",
        "scope beats relevance (FR-024)"
    );
    assert_eq!(results[1]["scope"], "branch");
    assert_eq!(results[2]["scope"], "project");
}

#[test]
fn a_fact_recorded_earlier_is_recalled_later_in_the_top_results() {
    // SC-004, without embeddings.
    let s = Sandbox::new();
    seed(&s);
    for i in 0..12 {
        s.must(&[
            "memory",
            "add",
            "--type",
            "fact",
            "--scope",
            "project",
            &format!("unrelated background note {i}"),
        ]);
    }
    s.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "Errors are returned, never logged and swallowed",
    ]);

    let out = s.json(&["memory", "search", "swallowed errors"]);
    let results = out["results"].as_array().unwrap();
    let top5: Vec<&str> = results
        .iter()
        .take(5)
        .filter_map(|r| r["content"].as_str())
        .collect();
    assert!(
        top5.iter()
            .any(|c| c.contains("never logged and swallowed")),
        "the fact was not in the top 5: {top5:?}"
    );
}

#[test]
fn every_result_carries_provenance_even_with_no_evidence() {
    // FR-019, SC-012: manual memory has an origin session and zero evidence.
    let s = Sandbox::new();
    seed(&s);
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "a durable fact",
    ]);

    let out = s.json(&["memory", "search", "durable"]);
    let r = &out["results"][0];
    assert!(
        r["provenance"]["session_id"].is_string(),
        "origin session is mandatory"
    );
    assert_eq!(
        r["provenance"]["evidence_count"], 0,
        "zero evidence is valid, not an error"
    );
    assert!(r["provenance"]["observation_ids"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn superseding_retains_the_original_and_the_link() {
    // FR-020: the replacement is created, the original is kept in state
    // `superseded`, and the two stay linked. `forget` is a different operation
    // and is not a substitute for this one.
    let s = Sandbox::new();
    seed(&s);

    let created = s.json(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "project",
        "dual writes chosen for sync",
    ]);
    let original_id = created["memory"]["id"].as_str().unwrap().to_string();

    // Supersede through the tool that offers it, driving the real MCP server.
    let cwd = s.repo_path().display().to_string();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let response = mcp.tool(
        "cairn_remember",
        json!({
            "action": "supersede",
            "memory_id": original_id,
            "type": "decision",
            "scope": "project",
            "content": "outbox chosen for sync; dual writes rejected"
        }),
        &cwd,
    );
    let parsed: serde_json::Value = serde_json::from_str(&response).expect("supersede json");
    let replacement_id = parsed["memory"]["id"]
        .as_str()
        .expect("replacement id")
        .to_string();
    assert_ne!(
        replacement_id, original_id,
        "supersede must create a new memory"
    );
    assert_eq!(parsed["superseded"].as_str(), Some(original_id.as_str()));

    // The original is retained, out of default recall, and linked forward.
    let original = s.json(&["memory", "show", &original_id])["memory"].clone();
    assert_eq!(original["state"], "superseded", "the original must be kept");
    assert_eq!(
        original["content"], "dual writes chosen for sync",
        "content retained"
    );
    assert_eq!(
        original["superseded_by_id"].as_str(),
        Some(replacement_id.as_str()),
        "the link to the replacement must survive"
    );

    // Default recall returns the replacement only.
    let active = s.json(&["memory", "search", "sync"]);
    let results = active["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "only the replacement is active");
    assert_eq!(results[0]["id"].as_str(), Some(replacement_id.as_str()));

    // The original is still retrievable on request.
    let superseded = s.json(&["memory", "search", "dual", "--state", "superseded"]);
    let old = superseded["results"].as_array().unwrap();
    assert_eq!(old.len(), 1);
    assert_eq!(old[0]["id"].as_str(), Some(original_id.as_str()));
    assert_eq!(
        old[0]["superseded_by_id"].as_str(),
        Some(replacement_id.as_str())
    );
}

#[test]
fn only_active_memories_are_returned_by_default() {
    let s = Sandbox::new();
    seed(&s);
    let created = s.json(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "temporary",
    ]);
    let id = created["memory"]["id"].as_str().unwrap().to_string();
    s.must(&["memory", "forget", &id]);

    assert!(s.json(&["memory", "search", "temporary"])["results"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn filters_work_without_a_text_query() {
    let s = Sandbox::new();
    seed(&s);
    s.must(&[
        "memory", "add", "--type", "fact", "--scope", "project", "one",
    ]);
    s.must(&[
        "memory", "add", "--type", "failure", "--scope", "project", "two",
    ]);

    let out = s.json(&["memory", "search", "--type", "failure"]);
    let results = out["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["content"], "two");
}

#[test]
fn everything_local_works_with_no_network() {
    // SC-006. The daemon has no server configured and no token: any network
    // dependency would fail here rather than degrade.
    let s = Sandbox::new();
    seed(&s);
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "offline fact",
    ]);
    assert!(!s.json(&["memory", "search", "offline"])["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(s.cairn(&["context"]).ok());
    assert!(s.cairn(&["status"]).ok());
    s.hook(
        "SessionEnd",
        json!({ "session_id": "m", "reason": "clear" }),
    );
    // The sealed close acknowledges before the handoff is written, so the
    // read waits the documented bound (FR-240, D22). What this test is about
    // is unchanged: all of it works with no network.
    assert!(!s.handoff_after_close(&[]).is_null());
}

#[test]
fn memory_scoped_to_a_deleted_branch_becomes_stale_and_leaves_default_recall() {
    // H4 / FR-018 / US3 scenario 5: marked stale, never deleted, and still
    // retrievable on request.
    let s = Sandbox::new();
    seed(&s);

    s.git(&["switch", "-c", "feature/doomed"]);
    s.must(&[
        "memory",
        "add",
        "--type",
        "failure",
        "--scope",
        "branch",
        "this fixture only exists on the doomed branch",
    ]);
    s.git(&["switch", "main"]);

    // While the branch exists, the memory is ordinary active memory.
    let on_branch = s.json(&[
        "memory",
        "search",
        "doomed",
        "--scope",
        "branch",
        "--scope-key",
        "feature/doomed",
    ]);
    assert_eq!(on_branch["results"].as_array().unwrap().len(), 1);
    assert_eq!(on_branch["results"][0]["state"], "active");

    s.git(&["branch", "-D", "feature/doomed"]);

    // `cairn status` is where Cairn notices the scope no longer resolves.
    s.must(&["status"]);

    let default_recall = s.json(&[
        "memory",
        "search",
        "doomed",
        "--scope",
        "branch",
        "--scope-key",
        "feature/doomed",
    ]);
    assert!(
        default_recall["results"].as_array().unwrap().is_empty(),
        "stale memory must drop out of default recall"
    );

    let on_request = s.json(&[
        "memory",
        "search",
        "doomed",
        "--scope",
        "branch",
        "--scope-key",
        "feature/doomed",
        "--state",
        "stale",
    ]);
    let stale = on_request["results"].as_array().unwrap();
    assert_eq!(stale.len(), 1, "it must be marked stale, not deleted");
    assert_eq!(stale[0]["state"], "stale");
    assert_eq!(
        stale[0]["content"],
        "this fixture only exists on the doomed branch"
    );

    // Project-scoped memory is untouched by branch churn.
    s.must(&[
        "memory",
        "add",
        "--type",
        "fact",
        "--scope",
        "project",
        "durable project fact",
    ]);
    s.must(&["status"]);
    let project = s.json(&["memory", "search", "durable"]);
    assert_eq!(project["results"][0]["state"], "active");
}

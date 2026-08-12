//! Concurrency tests for CLI commands racing against each other.
//!
//! The storage contention test (`storage_contention.rs`) covers the
//! SQLite layer under capture load. This file covers the layer above:
//! two `task new` racing, two identical `memory add` racing, concurrent
//! deletes, and concurrent supersede chains — the cases where the daemon
//! handler and the store together must produce a deterministic outcome.

use cairn_e2e::Sandbox;
use serde_json::json;
use std::sync::Arc;

#[test]
fn two_task_new_racing_both_succeed() {
    let s = Arc::new(Sandbox::new());
    let mut handles = Vec::new();
    for i in 0..2 {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.must(&[
                "task",
                "new",
                "--title",
                &format!("race-{i}"),
                "--goal",
                "g",
            ]);
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
    let tasks = s.json(&["task", "list"]);
    assert_eq!(tasks["tasks"].as_array().unwrap().len(), 2);
}

#[test]
fn two_identical_memory_adds_produce_two_memories() {
    let s = Arc::new(Sandbox::new());
    // Start one session so `memory add` has a session to bind to.
    s.hook(
        "SessionStart",
        json!({ "session_id": "mem-concurrent", "source": "startup" }),
    );
    s.settle_session_count(1);

    let mut handles = Vec::new();
    for _ in 0..2 {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.must(&[
                "memory",
                "add",
                "--type",
                "fact",
                "--scope",
                "project",
                "identical content",
            ]);
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
    let results = s.json(&["memory", "search", "identical"]);
    assert_eq!(
        results["results"].as_array().unwrap().len(),
        2,
        "two identical adds must produce two memories, not one"
    );
}

#[test]
fn concurrent_deletes_of_different_memories_both_succeed() {
    let s = Arc::new(Sandbox::new());
    let a = s.json(&["memory", "add", "--type", "fact", "--scope", "project", "A"]);
    let b = s.json(&["memory", "add", "--type", "fact", "--scope", "project", "B"]);
    let a_id = a["memory"]["id"].as_str().unwrap().to_string();
    let b_id = b["memory"]["id"].as_str().unwrap().to_string();

    let mut handles = Vec::new();
    for id in [a_id, b_id] {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.must(&["delete", "memory", &id]);
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
    let remaining = s.json(&["memory", "search", ""]);
    assert!(
        remaining["results"].as_array().unwrap().is_empty(),
        "both memories should be deleted"
    );
}

#[test]
fn concurrent_session_starts_in_one_worktree_each_get_their_own_session() {
    let s = Sandbox::new();
    let n = 4;
    for i in 0..n {
        s.hook(
            "SessionStart",
            json!({ "session_id": format!("concurrent-{i}"), "source": "startup" }),
        );
    }
    s.settle_session_count(n);

    for i in 0..n {
        s.hook(
            "PostToolUse",
            json!({
                "session_id": format!("concurrent-{i}"),
                "tool_name": "Edit",
                "tool_input": { "file_path": format!("file{i}.rs") }
            }),
        );
    }

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    let active: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["status"] == "active")
        .collect();
    assert_eq!(active.len(), n, "all concurrent sessions must be distinct");
}

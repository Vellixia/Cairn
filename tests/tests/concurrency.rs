//! Concurrency tests for CLI commands racing against each other.
//!
//! The storage contention test (`storage_contention.rs`) covers the
//! SQLite layer under capture load. This file covers the layer above:
//! first use of one repository from several processes at once, plus
//! two `task new` racing, two identical `memory add` racing, concurrent
//! deletes, and concurrent supersede chains — the cases where the daemon
//! handler and the store together must produce a deterministic outcome.

use cairn_e2e::{binary, Sandbox};
use serde_json::json;
use std::process::Command;
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
    // The hooks are fired from threads, not in sequence. A sequential loop
    // proves only that four keys make four sessions, which no amount of
    // locking could get wrong — the race this test is named for is four
    // `SessionStart` handlers arriving at one project row at once.
    let s = Arc::new(Sandbox::new());
    let n = 4;

    let mut handles = Vec::new();
    for i in 0..n {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.hook(
                "SessionStart",
                json!({ "session_id": format!("concurrent-{i}"), "source": "startup" }),
            );
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
    s.settle_session_count(n);

    let mut handles = Vec::new();
    for i in 0..n {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.hook(
                "PostToolUse",
                json!({
                    "session_id": format!("concurrent-{i}"),
                    "tool_name": "Edit",
                    "tool_input": { "file_path": format!("file{i}.rs") }
                }),
            );
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
    s.settle_observations(n as i64);

    let sessions = s.json(&["session", "list"])["sessions"].clone();
    let active: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["status"] == "active")
        .collect();
    assert_eq!(active.len(), n, "all concurrent sessions must be distinct");

    // Distinct sessions, not one session counted four times.
    let keys: std::collections::BTreeSet<_> =
        active.iter().filter_map(|x| x["id"].as_str()).collect();
    assert_eq!(
        keys.len(),
        n,
        "each session must have its own id: {active:?}"
    );
}

#[test]
fn concurrent_first_use_of_one_fresh_repository_never_races() {
    // H1 / FR-002: register-or-reuse must be atomic. Check-then-insert used to
    // answer this with `UNIQUE constraint failed: projects.git_common_dir`.
    let s = Sandbox::new();

    // A cold store, so every thread below is racing the very first insert.
    s.must(&["daemon", "stop"]);
    std::thread::sleep(std::time::Duration::from_millis(200));

    let threads: Vec<_> = (0..12)
        .map(|_| {
            let repo = s.repo_path().to_path_buf();
            let home = s.home.path().to_path_buf();
            let socket = s.socket.clone();
            std::thread::spawn(move || {
                let out = Command::new(binary("cairn"))
                    .args(["--json", "status"])
                    .current_dir(&repo)
                    .env("CAIRN_HOME", &home)
                    .env("CAIRN_SOCKET", &socket)
                    .env("CAIRND_BIN", binary("cairnd"))
                    .output()
                    .expect("cairn runs");
                String::from_utf8_lossy(&out.stdout).to_string()
            })
        })
        .collect();

    let mut project_ids = std::collections::HashSet::new();
    for (i, t) in threads.into_iter().enumerate() {
        let raw = t.join().expect("thread");
        let envelope: serde_json::Value =
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("thread {i}: {e}\n{raw}"));
        assert_eq!(
            envelope["ok"],
            true,
            "concurrent first use failed: {}",
            serde_json::to_string(&envelope["error"]).unwrap_or_default()
        );
        project_ids.insert(
            envelope["data"]["project"]["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    assert_eq!(
        project_ids.len(),
        1,
        "one repository must map to one project"
    );
}

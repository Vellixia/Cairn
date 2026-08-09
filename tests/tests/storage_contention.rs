//! Local writes stay reliable while capture is writing (FR-044, FR-047).
//!
//! A developer runs `cairn memory add` while their agent is firing hooks. Both
//! reach the same SQLite file on different connections. Under a deferred
//! `BEGIN` the memory write had to upgrade a read snapshot the capture writes
//! had already invalidated, and SQLite answered `SQLITE_BUSY_SNAPSHOT` (517) —
//! surfaced to the user as `storage_unavailable: database is locked`, with the
//! memory simply lost.
//!
//! These tests run the real daemon against a real database file and check every
//! command's result, because a swallowed failure here is exactly the defect.

use cairn_e2e::Sandbox;
use serde_json::json;

/// Hook traffic and memory writes, interleaved the way they really are.
///
/// Enough operations that the old implementation failed reliably: at this
/// volume it lost several writes to 517 on every run.
const MEMORIES: usize = 60;
const HOOKS_PER_MEMORY: usize = 3;
/// Two capture streams, because one process firing hooks serially leaves gaps
/// the writer can slip through — and a regression test that only sometimes
/// reproduces the defect is not a regression test.
const CAPTURE_STREAMS: usize = 2;
/// Two writers as well: the defect needs a memory transaction to be open while
/// another connection commits, and one serial writer only sometimes lines up.
const WRITER_STREAMS: usize = 2;

#[test]
fn memory_add_never_loses_a_write_to_capture_contention() {
    let s = Sandbox::new();
    let task = s.json(&["task", "new", "--title", "T", "--goal", "g"]);
    let task_id = task["task"]["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionStart",
        json!({ "session_id": "load", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "load", "--task", &task_id]);

    // Capture keeps writing observations for as long as the memories are being
    // added, on its own connections, exactly as a working agent would.
    let sandbox = &s;
    let contended: Vec<String> = std::thread::scope(|scope| {
        let capture: Vec<_> = (0..CAPTURE_STREAMS)
            .map(|stream| {
                scope.spawn(move || {
                    for i in 0..(MEMORIES * HOOKS_PER_MEMORY) {
                        sandbox.hook(
                            "PostToolUse",
                            json!({ "session_id": "load", "tool_name": "Edit",
                                    "tool_input": { "file_path": format!("f{stream}-{i}.rs") } }),
                        );
                    }
                })
            })
            .collect();

        let writers: Vec<_> = (0..WRITER_STREAMS)
            .map(|stream| {
                scope.spawn(move || {
                    let mut written = Vec::new();
                    for i in 0..MEMORIES {
                        let content = format!("contended memory {stream}-{i}");
                        // Checked, every time: a lost write fails the test here.
                        let result = sandbox.cairn(&[
                            "--json", "memory", "add", "--type", "fact", "--scope", "project",
                            &content,
                        ]);
                        assert!(
                            result.ok(),
                            "`memory add` {stream}-{i} failed under capture load: {}{}",
                            result.stdout.trim(),
                            result.stderr.trim()
                        );
                        let envelope: serde_json::Value =
                            serde_json::from_str(&result.stdout).expect("envelope");
                        assert_eq!(
                            envelope["ok"],
                            true,
                            "`memory add` {stream}-{i} reported an error: {}",
                            serde_json::to_string(&envelope["error"]).unwrap_or_default()
                        );
                        written.push(content);
                    }
                    written
                })
            })
            .collect();

        for stream in capture {
            stream.join().expect("capture finished");
        }
        writers
            .into_iter()
            .flat_map(|w| w.join().expect("writer finished"))
            .collect()
    });

    // Exactly one memory per successful command: no loss, and no duplicate
    // conjured by a retry.
    for content in &contended {
        let found = s.query_column(&format!(
            "SELECT id FROM memories WHERE content = '{content}' AND deleted_at IS NULL"
        ));
        assert_eq!(
            found.len(),
            1,
            "`{content}` should appear exactly once, found {}",
            found.len()
        );
    }

    let total = s.query_column(
        "SELECT id FROM memories WHERE content LIKE 'contended memory %' AND deleted_at IS NULL",
    );
    assert_eq!(
        total.len(),
        MEMORIES * WRITER_STREAMS,
        "wrong number of memories overall"
    );

    // The observations that were racing the writes are all there too.
    let observations = s.query_column("SELECT id FROM observations WHERE deleted_at IS NULL");
    assert!(
        !observations.is_empty(),
        "capture wrote nothing, so nothing was contended"
    );

    assert_eq!(
        s.integrity_check(),
        "ok",
        "database integrity after contention"
    );
}

#[test]
fn every_entity_type_survives_concurrent_writing() {
    // The fix is in the shared write transaction, so the proof cannot be one
    // command: tasks, sessions, memories, handoffs and capture all write
    // through it, and all of them run here at once (FR-044).
    let s = Sandbox::new();
    let task = s.json(&["task", "new", "--title", "root", "--goal", "g"]);
    let root_task = task["task"]["id"].as_str().unwrap().to_string();
    s.hook(
        "SessionStart",
        json!({ "session_id": "mixed", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "mixed", "--task", &root_task]);

    const ROUNDS: usize = 25;

    // A session of this thread's own, so memory writes are attributable while
    // the session thread churns.
    let writer_session = s.json(&["session", "start", "--key", "contention-writer"])["session"]
        ["id"]
        .as_str()
        .expect("session id")
        .to_string();

    std::thread::scope(|scope| {
        let capture = scope.spawn(|| {
            for i in 0..(ROUNDS * 3) {
                s.hook(
                    "PostToolUse",
                    json!({ "session_id": "mixed", "tool_name": "Edit",
                            "tool_input": { "file_path": format!("m{i}.rs") } }),
                );
            }
        });

        // Pinned to its own session: the thread below opens and closes
        // sessions constantly, and a memory that cannot say which session it
        // came from is now refused rather than attributed to a guess.
        let memories = scope.spawn(|| {
            for i in 0..ROUNDS {
                s.must(&[
                    "memory",
                    "add",
                    "--type",
                    "fact",
                    "--scope",
                    "project",
                    "--session",
                    &writer_session,
                    &format!("mixed memory {i}"),
                ]);
            }
        });

        let tasks = scope.spawn(|| {
            for i in 0..ROUNDS {
                let created = s.json(&[
                    "task",
                    "new",
                    "--title",
                    &format!("mixed task {i}"),
                    "--goal",
                    "g",
                ]);
                let id = created["task"]["id"].as_str().expect("task id").to_string();
                s.must(&["task", "set-status", &id, "in_progress"]);
            }
        });

        // A second session, started and ended repeatedly: session writes and
        // handoff synthesis contend with everything above.
        let sessions = scope.spawn(|| {
            for i in 0..ROUNDS {
                let key = format!("mixed-{i}");
                s.hook(
                    "SessionStart",
                    json!({ "session_id": key, "source": "startup" }),
                );
                s.hook(
                    "SessionEnd",
                    json!({ "session_id": key, "reason": "clear" }),
                );
            }
        });

        capture.join().expect("capture finished");
        memories.join().expect("memories finished");
        tasks.join().expect("tasks finished");
        sessions.join().expect("sessions finished");
    });

    let memories = s.query_column(
        "SELECT id FROM memories WHERE content LIKE 'mixed memory %' AND deleted_at IS NULL",
    );
    assert_eq!(memories.len(), ROUNDS, "a memory write was lost");

    let tasks = s.query_column(
        "SELECT id FROM tasks WHERE title LIKE 'mixed task %' AND deleted_at IS NULL",
    );
    assert_eq!(tasks.len(), ROUNDS, "a task write was lost");

    let in_progress = s.query_column(
        "SELECT id FROM tasks WHERE title LIKE 'mixed task %' AND status = 'in_progress'",
    );
    assert_eq!(in_progress.len(), ROUNDS, "a task update was lost");

    let sessions = s.query_column("SELECT id FROM sessions WHERE deleted_at IS NULL");
    assert!(
        sessions.len() > ROUNDS,
        "expected the concurrent sessions to exist, found {}",
        sessions.len()
    );

    assert_eq!(
        s.integrity_check(),
        "ok",
        "database integrity after contention"
    );
}

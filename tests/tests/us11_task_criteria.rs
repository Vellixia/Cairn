//! US11 — evidence-aware tasks (`contracts/task-model.md`).
//!
//! The defect this slice removes is research B3: `acceptance_criteria` was a
//! JSON array of plain strings written in one statement, so two sessions
//! editing different criteria lost one another's work. Criteria now have stable
//! ids, and the negatives below are what keep the fix honest — no assertion
//! lost, no overwrite invisible, and no route by which an agent can certify its
//! own completion.

use cairn_e2e::Sandbox;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers — free functions, so Phase 10's `offline_convergence` can reuse them
// without restructuring this file.
// ---------------------------------------------------------------------------

/// A task with the given criteria, returning its id.
pub fn task_with(s: &Sandbox, criteria: &[&str]) -> String {
    let mut args = vec![
        "task",
        "new",
        "--title",
        "Add rate limiting",
        "--goal",
        "Requests over the limit get 429",
    ];
    for c in criteria {
        args.push("--criterion");
        args.push(c);
    }
    let created = s.json(&args);
    created["task"]["id"]
        .as_str()
        .expect("a created task has an id")
        .to_string()
}

/// The full `task get` payload, including the Feature 003 read-only fields.
pub fn task_get(s: &Sandbox, task_id: &str) -> Value {
    s.json(&["task", "show", task_id])
}

/// One criterion by label.
pub fn criterion(s: &Sandbox, task_id: &str, label: &str) -> Value {
    let v = task_get(s, task_id);
    v["criteria"]
        .as_array()
        .expect("criteria is an array")
        .iter()
        .find(|c| c["label"] == label)
        .unwrap_or_else(|| panic!("no criterion {label} in {}", v["criteria"]))
        .clone()
}

pub fn criterion_id(s: &Sandbox, task_id: &str, label: &str) -> String {
    criterion(s, task_id, label)["id"]
        .as_str()
        .expect("a criterion has an id")
        .to_string()
}

/// A session bound to nothing, so a write has a distinct author.
pub fn session(s: &Sandbox, key: &str) -> String {
    s.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    let started = s.json(&["session", "start", "--key", key]);
    started["session"]["id"]
        .as_str()
        .expect("a started session has an id")
        .to_string()
}

pub fn state_digest(s: &Sandbox, task_id: &str) -> String {
    task_get(s, task_id)["state_digest"]
        .as_str()
        .expect("a task has a derived state digest")
        .to_string()
}

// ---------------------------------------------------------------------------
// T065 — no silent overwrite
// ---------------------------------------------------------------------------

/// The negative B3 cost, removed by construction, plus the two revision cases.
///
/// Three things at once, because they are the same guarantee seen from three
/// angles: disjoint edits never interact; a caller who says what it read is
/// protected; and a caller who does not is still *recorded*, so no overwrite is
/// invisible (FR-337, FR-490, SC-317).
#[test]
fn no_silent_overwrite() {
    let s = Sandbox::new();
    let task = task_with(&s, &["429 above threshold", "Limit is configurable"]);
    let a = session(&s, "sess-a");
    let b = session(&s, "sess-b");

    let ac1 = criterion_id(&s, &task, "AC-1");
    let ac2 = criterion_id(&s, &task, "AC-2");

    // --- Two sessions, different criteria. Both persist, neither resets the
    // other. This is precisely what the old whole-row write lost.
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "satisfied",
        "--session",
        &a,
    ]);
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac2,
        "--state",
        "blocked",
        "--session",
        &b,
    ]);

    assert_eq!(
        criterion(&s, &task, "AC-1")["state"],
        "satisfied",
        "the first session's assertion was reset by the second"
    );
    assert_eq!(
        criterion(&s, &task, "AC-2")["state"],
        "blocked",
        "the second session's assertion did not persist"
    );

    // The counter advanced once per change, and both are in the log.
    let history = s.json(&["task", "history", &task]);
    let changes = history["changes"].as_array().expect("a change log");
    let states: Vec<&Value> = changes
        .iter()
        .filter(|c| c["kind"] == "criterion_state")
        .collect();
    assert_eq!(states.len(), 2, "both state changes must be in the log");

    // --- The same criterion, with `expected_revision` supplied. The loser is
    // refused by name and the winner's value stands.
    let revision = criterion(&s, &task, "AC-1")["revision"]
        .as_i64()
        .expect("a criterion carries its revision");

    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "pending",
        "--expected-revision",
        &revision.to_string(),
        "--session",
        &a,
    ]);

    // `b` still holds the revision it read before `a` wrote.
    let refused = s.json_err(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "waived",
        "--expected-revision",
        &revision.to_string(),
        "--session",
        &b,
    ]);
    assert_eq!(
        refused["code"], "revision_conflict",
        "a stale expected_revision must be refused by name: {refused}"
    );
    assert_eq!(
        criterion(&s, &task, "AC-1")["state"],
        "pending",
        "the refused write must not have applied"
    );

    // --- No revision supplied. Both land, and both are recorded as blind — the
    // prior value, the author and the absence of a token are all in the log.
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac2,
        "--state",
        "pending",
        "--session",
        &a,
    ]);
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac2,
        "--state",
        "satisfied",
        "--session",
        &b,
    ]);

    let history = s.json(&["task", "history", &task]);
    let blind: Vec<&Value> = history["changes"]
        .as_array()
        .expect("a change log")
        .iter()
        .filter(|c| c["blind_write"] == true && c["kind"] == "criterion_state")
        .collect();
    assert!(
        blind.len() >= 2,
        "both writes with no expected_revision must be recorded as blind: {}",
        history["changes"]
    );
    assert!(
        blind.iter().any(|c| c["prior_value"] == "pending"),
        "a blind write must record the value it replaced, or the overwrite is invisible"
    );
    assert_eq!(criterion(&s, &task, "AC-2")["state"], "satisfied");
}

/// A label never changes meaning (FR-481).
#[test]
fn labels_are_never_renumbered() {
    let s = Sandbox::new();
    let task = task_with(&s, &["first", "second"]);
    let a = session(&s, "sess-a");
    let ac2 = criterion_id(&s, &task, "AC-2");

    s.must(&["task", "criterion", "remove", &ac2, "--session", &a]);
    s.must(&[
        "task",
        "criterion",
        "add",
        &task,
        "--text",
        "third",
        "--session",
        &a,
    ]);

    let v = task_get(&s, &task);
    let labels: Vec<&str> = v["criteria"]
        .as_array()
        .expect("criteria")
        .iter()
        .map(|c| c["label"].as_str().unwrap_or("?"))
        .collect();
    assert_eq!(
        labels,
        vec!["AC-1", "AC-3"],
        "removing AC-2 must leave AC-1 and AC-3; AC-3 is not renamed"
    );
}

// ---------------------------------------------------------------------------
// T066 — no self-certification
// ---------------------------------------------------------------------------

/// A criterion never reaches `verified` on attested evidence (FR-483, FR-484).
///
/// Completion readiness is the one derived value with an incentive attached. If
/// an agent could attest its way to `verified`, readiness would become
/// self-certification — so an agent's own observation is usable, labelled, and
/// refused for exactly this consumer.
#[test]
fn attested_is_not_enough() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "config", "--no-gpg-sign"]);

    let task = task_with(&s, &["the port is 8080"]);
    let a = session(&s, "sess-a");
    let ac1 = criterion_id(&s, &task, "AC-1");

    // An agent attests. The fact is stored and labelled `agent`.
    let attested = s.json(&[
        "evidence",
        "add",
        "--type",
        "runtime_state",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
        "--collector",
        "agent",
        "--session",
        &a,
    ]);
    let evidence = attested["fact"]["id"]
        .as_str()
        .or_else(|| attested["evidence"]["id"].as_str())
        .expect("a recorded fact has an id")
        .to_string();

    let refused = s.json_err(&[
        "task",
        "criterion",
        "verify",
        &ac1,
        "--evidence",
        &evidence,
        "--session",
        &a,
    ]);
    assert_eq!(
        refused["code"], "attested_not_sufficient",
        "attested evidence must be refused for a criterion's verification: {refused}"
    );
    assert_eq!(
        criterion(&s, &task, "AC-1")["verification"],
        "unverified",
        "a refused verification must leave the criterion unverified"
    );

    // The work axis is untouched by any of it: the two axes are independent,
    // and `satisfied` + `unverified` is a normal, separately reported state.
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "satisfied",
        "--session",
        &a,
    ]);
    let c = criterion(&s, &task, "AC-1");
    assert_eq!(c["state"], "satisfied");
    assert_eq!(c["verification"], "unverified");

    let readiness = s.json(&["task", "readiness", &task]);
    assert_eq!(
        readiness["completion_readiness"], "ready_unverified",
        "an asserted-but-unchecked criterion is ready_unverified, never ready"
    );
    assert_eq!(readiness["progress"]["satisfied_unverified"], 1);
    assert_eq!(readiness["progress"]["verified"], 0);

    // And readiness changed no status (FR-487).
    assert_eq!(
        task_get(&s, &task)["task"]["status"],
        "todo",
        "readiness must never move a task's status"
    );
}

/// There is nowhere to write a completion percentage (FR-486, SC-328).
///
/// Written as an enumeration rather than as "construct a payload and observe a
/// field is missing" — the latter proves nothing and would pass forever. This
/// walks the real schema, the real serialized payloads and the real CLI tree
/// and intersects each against a denylist, so a field added in a later phase
/// fails here.
#[test]
fn no_percentage_field() {
    let s = Sandbox::new();
    let task = task_with(&s, &["one"]);

    const FORBIDDEN: &[&str] = &[
        "percent",
        "percentage",
        "pct",
        "progress_percent",
        "completion_percent",
        "fraction",
        "ratio",
        "score",
    ];
    // `completion_readiness` is deliberately not on that list: it is an enum of
    // three words, not a number, and it is the value a percentage would have
    // replaced.

    // --- The schema. Every column of every task table.
    let columns = task_table_columns(&s);
    for column in &columns {
        for bad in FORBIDDEN {
            assert!(
                !column.contains(bad),
                "column `{column}` would let an agent store a completion percentage"
            );
        }
    }

    // --- The wire payload, as `task get` actually serializes it.
    let payload = task_get(&s, &task);
    for key in json_keys(&payload) {
        for bad in FORBIDDEN {
            assert!(
                !key.to_ascii_lowercase().contains(bad),
                "the task payload carries `{key}`, in which a percentage could be stored"
            );
        }
    }

    // --- The CLI. Every flag of every `cairn task` subcommand.
    for flag in task_cli_surface(&s) {
        for bad in FORBIDDEN {
            assert!(
                !flag.contains(bad),
                "`cairn task` exposes `{flag}`, through which a percentage could be set"
            );
        }
    }

    // Progress is counts, and nothing else.
    let readiness = s.json(&["task", "readiness", &task]);
    let progress = readiness["progress"].as_object().expect("progress counts");
    for (k, v) in progress {
        assert!(
            v.is_u64() || v.is_i64(),
            "progress.{k} is {v}, not a count — a percentage cannot be derived into existence"
        );
    }
}

// ---------------------------------------------------------------------------
// T075 — the model stays a work model, not a project-management tool
// ---------------------------------------------------------------------------

/// No sprint, epic, story point, assignee, estimate, board or inter-task
/// dependency exists anywhere (FR-491).
///
/// The same enumeration technique as `no_percentage_field`, and the CLI half
/// matters most: this phase adds seven subcommands, and Phase 13 adds four MCP
/// actions. Asserting against the real command tree is what keeps that honest.
#[test]
fn no_project_management() {
    let s = Sandbox::new();
    let task = task_with(&s, &["one"]);

    const FORBIDDEN: &[&str] = &[
        "sprint",
        "epic",
        "story_point",
        "storypoint",
        "story-point",
        "point",
        "assignee",
        "assigned_to",
        "estimate",
        "estimation",
        "board",
        "swimlane",
        "milestone",
        "depends_on",
        "blocked_by_task",
        "parent_task",
        "subtask",
    ];

    let mut surface: Vec<String> = task_table_columns(&s);
    surface.extend(json_keys(&task_get(&s, &task)));
    surface.extend(task_cli_surface(&s));

    for name in &surface {
        let lower = name.to_ascii_lowercase();
        for bad in FORBIDDEN {
            assert!(
                !lower.contains(bad),
                "`{name}` is a project-management concept and Feature 003 adds none (FR-491)"
            );
        }
    }

    // The one thing that *is* in scope and reads adjacent: a blocker is a note
    // with attribution, not an inter-task dependency. It takes a description
    // and never another task's id.
    let help = s.cairn(&["task", "blocker", "open", "--help"]);
    assert!(
        !help.stdout.contains("--task-id") || !help.stdout.contains("--depends"),
        "a blocker must not express a dependency on another task"
    );
}

// ---------------------------------------------------------------------------
// T074 — the five Feature 001 readers still work
// ---------------------------------------------------------------------------

/// The whole-list form keeps ids for unchanged entries and logs each change,
/// and the retained projection stays exactly what five readers expect
/// (FR-492, SC-323).
#[test]
fn the_whole_list_form_still_works() {
    let s = Sandbox::new();
    let task = task_with(&s, &["keep me", "remove me"]);
    let keep_id = criterion_id(&s, &task, "AC-1");

    s.must(&[
        "task",
        "update",
        &task,
        "--acceptance-criteria",
        "keep me",
        "--acceptance-criteria",
        "add me",
    ]);

    // The unchanged entry kept its id and its label.
    assert_eq!(
        criterion_id(&s, &task, "AC-1"),
        keep_id,
        "an unchanged criterion must keep its id — that is the whole point"
    );
    // The removed one is tombstoned, and the new one did not take its label.
    let labels: Vec<String> = task_get(&s, &task)["criteria"]
        .as_array()
        .expect("criteria")
        .iter()
        .map(|c| c["label"].as_str().unwrap_or("?").to_string())
        .collect();
    assert_eq!(labels, vec!["AC-1", "AC-3"]);

    // Reader 1 and 2 — the retained projection, as `task get`/`task list`
    // render it, ordinal-ordered and holding the text values.
    let t = task_get(&s, &task);
    let projection: Vec<&str> = t["task"]["acceptance_criteria"]
        .as_array()
        .expect("the Feature 001 array is retained")
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect();
    assert_eq!(
        projection,
        vec!["keep me", "add me"],
        "the retained projection must equal the ordinal-ordered criterion texts"
    );
    let listed = s.json(&["task", "list"]);
    assert!(listed["tasks"].as_array().expect("tasks").iter().any(|t| {
        t["acceptance_criteria"]
            .as_array()
            .map(|a| a.len() == 2)
            .unwrap_or(false)
    }));

    // Reader 3 — the briefing assembler admits the task's criteria.
    s.hook(
        "SessionStart",
        json!({ "session_id": "brief", "source": "startup" }),
    );
    s.must(&["session", "start", "--key", "brief", "--task", &task]);
    let context = s.json(&["context", "--json"]);
    let rendered = context.to_string();
    assert!(
        rendered.contains("keep me"),
        "the briefing must still admit the task's acceptance criteria"
    );

    // Reader 4 — the sync payload carries the array, and never the local
    // counter (FR-490, D80).
    let payloads = s.query_column("SELECT payload FROM outbox WHERE entity_type = 'task'");
    for p in &payloads {
        assert!(
            !p.contains("local_revision"),
            "the local counter must never be transmitted: {p}"
        );
    }

    // Every change is its own log entry.
    let history = s.json(&["task", "history", &task]);
    let kinds: Vec<&str> = history["changes"]
        .as_array()
        .expect("a change log")
        .iter()
        .map(|c| c["kind"].as_str().unwrap_or("?"))
        .collect();
    assert!(kinds.contains(&"criterion_removed"));
    assert!(kinds.contains(&"criterion_added"));
}

/// The projection is a cache of the rows and never an independent truth
/// (I11, SC-324).
#[test]
fn the_projection_equals_the_rows() {
    let s = Sandbox::new();
    let task = task_with(&s, &["alpha", "beta", "gamma"]);
    let a = session(&s, "sess-a");
    let ac2 = criterion_id(&s, &task, "AC-2");
    s.must(&["task", "criterion", "remove", &ac2, "--session", &a]);
    s.must(&[
        "task",
        "criterion",
        "add",
        &task,
        "--text",
        "delta",
        "--session",
        &a,
    ]);

    let stored = task_get(&s, &task)["task"]["acceptance_criteria"]
        .as_array()
        .expect("the projection")
        .iter()
        .map(|v| v.as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>();
    let rows: Vec<String> = task_get(&s, &task)["criteria"]
        .as_array()
        .expect("criteria")
        .iter()
        .map(|c| c["text"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(
        stored, rows,
        "the retained projection must be rebuildable from the criterion rows"
    );
    assert_eq!(stored, vec!["alpha", "gamma", "delta"]);
}

/// The state digest is derived from the converged records, and the local
/// counter is not one of its inputs (FR-493, SC-330).
#[test]
fn the_digest_ignores_the_local_counter() {
    let s = Sandbox::new();
    let task = task_with(&s, &["alpha", "beta"]);
    let a = session(&s, "sess-a");
    let ac1 = criterion_id(&s, &task, "AC-1");

    let before = state_digest(&s, &task);

    // A change that moves the counter and the criterion.
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "satisfied",
        "--session",
        &a,
    ]);
    let after = state_digest(&s, &task);
    assert_ne!(before, after, "a criterion change must change the digest");

    // Changing it back returns the digest, even though the counter has advanced
    // twice more. Content-addressed, not counter-addressed.
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "pending",
        "--session",
        &a,
    ]);
    assert_eq!(
        state_digest(&s, &task),
        before,
        "the digest must be derived from the records, never from the counter"
    );

    let counter = task_get(&s, &task)["local_revision"]
        .as_i64()
        .expect("a local revision");
    assert!(counter > 1, "the local counter did advance");
}

// ---------------------------------------------------------------------------
// Surface enumeration
// ---------------------------------------------------------------------------

/// Every column of every table the task model owns.
fn task_table_columns(s: &Sandbox) -> Vec<String> {
    let mut out = Vec::new();
    for table in [
        "tasks",
        "task_criteria",
        "task_blockers",
        "task_changes",
        "criterion_evidence",
    ] {
        let rows = s.query_column(&format!("SELECT name FROM pragma_table_info('{table}')"));
        assert!(
            !rows.is_empty(),
            "{table} must exist for this to prove anything"
        );
        out.extend(rows);
    }
    out
}

/// Every key of a JSON payload, recursively.
fn json_keys(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match v {
        Value::Object(m) => {
            for (k, inner) in m {
                out.push(k.clone());
                out.extend(json_keys(inner));
            }
        }
        Value::Array(a) => {
            for inner in a {
                out.extend(json_keys(inner));
            }
        }
        _ => {}
    }
    out
}

/// Every long flag of every `cairn task` subcommand, read from the real
/// command tree rather than from a list of what we think we added.
fn task_cli_surface(s: &Sandbox) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue: Vec<Vec<String>> = vec![vec!["task".to_string()]];

    while let Some(path) = queue.pop() {
        let mut args: Vec<&str> = path.iter().map(String::as_str).collect();
        args.push("--help");
        let help = s.cairn(&args);
        let text = format!("{}{}", help.stdout, help.stderr);

        let mut in_commands = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("--") {
                let flag = trimmed
                    .split([' ', ',', '=', '<'])
                    .next()
                    .unwrap_or("")
                    .to_string();
                if flag.len() > 2 {
                    out.push(flag);
                }
            }
            if trimmed.eq_ignore_ascii_case("commands:") {
                in_commands = true;
                continue;
            }
            if trimmed.is_empty() {
                in_commands = false;
                continue;
            }
            if in_commands && !trimmed.starts_with('-') {
                if let Some(name) = trimmed.split_whitespace().next() {
                    if name != "help" && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                    {
                        let mut child = path.clone();
                        child.push(name.to_string());
                        queue.push(child);
                    }
                }
            }
        }
    }
    assert!(
        out.iter().any(|f| f == "--expected-revision"),
        "the enumeration found no flags, so it proves nothing: {out:?}"
    );
    out
}

// ---------------------------------------------------------------------------
// T072 — a session bound at an earlier state is told the task advanced
// ---------------------------------------------------------------------------

/// The divergence report, derived by diffing the bound snapshot against the
/// current records (FR-489, FR-493, SC-318, D80).
///
/// The critical half is the *origin* of each change. `task_changes` is local, so
/// a log-based report would describe only this machine's edits and would
/// silently omit a criterion that arrived from another machine — even though the
/// criterion row itself is right there. Diffing converged records reports both.
#[test]
fn a_bound_session_learns_what_changed() {
    let s = Sandbox::new();
    let task = task_with(&s, &["alpha", "beta"]);

    // A session binds at the current state.
    s.hook(
        "SessionStart",
        json!({ "session_id": "bound", "source": "startup" }),
    );
    let bound = s.json(&["session", "start", "--key", "bound", "--task", &task]);
    let bound_id = bound["session"]["id"].as_str().expect("id").to_string();

    // Nothing has changed yet, so nothing is reported. An empty report and a
    // missing one must not be confused, so assert the absence explicitly.
    let quiet = s.json(&["context", "--session", &bound_id, "--json"]);
    assert!(
        quiet["task_divergence"].is_null(),
        "a session bound at the current state must not be told the task advanced: {}",
        quiet["task_divergence"]
    );

    // A change this machine made.
    let ac1 = criterion_id(&s, &task, "AC-1");
    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "satisfied",
        "--session",
        &bound_id,
    ]);

    // A change that *arrived* — written as synchronization writes it, by id,
    // with no local change-log row to attribute it.
    s.execute_sql(
        "INSERT INTO task_criteria
            (id, task_id, ordinal, label, text, state, verification, revision,
             created_at, updated_at)
         SELECT '01a00000-0000-7000-8000-00000000beef', id, 9, 'AC-9',
                'production smoke passes', 'satisfied', 'unverified', 1,
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
           FROM tasks LIMIT 1",
    );

    let refreshed = s.json(&["context", "--session", &bound_id, "--json"]);
    let divergence = &refreshed["task_divergence"];
    assert_eq!(
        divergence["advanced"], true,
        "the session must be told the task advanced: {refreshed}"
    );

    let changes = divergence["changes"].as_array().expect("a change list");
    let rendered = serde_json::to_string(changes).expect("serializes");

    assert!(
        rendered.contains("AC-1"),
        "the change this machine made must be reported: {rendered}"
    );
    assert!(
        rendered.contains("AC-9") && rendered.contains("production smoke passes"),
        "a criterion learned through synchronization must be reported too — this \
         is exactly what a change-log-based report would have missed: {rendered}"
    );

    let this_machine = changes
        .iter()
        .find(|c| c["subject"] == "AC-1")
        .expect("AC-1 is reported");
    assert_eq!(
        this_machine["origin"], "this_machine",
        "a change this store can attribute must say so"
    );
    let arrived = changes
        .iter()
        .find(|c| c["subject"] == "AC-9")
        .expect("AC-9 is reported");
    assert_eq!(
        arrived["origin"], "another_machine",
        "a change with no local attribution must read as having arrived, not as local"
    );

    // The bound snapshot is never rewritten; only the report changes.
    let snapshots = s.query_column(&format!(
        "SELECT task_snapshot_at_bind FROM sessions WHERE id = '{bound_id}'"
    ));
    let snapshot = snapshots.first().expect("a snapshot was recorded");
    assert!(
        snapshot.contains("alpha") && !snapshot.contains("production smoke"),
        "the snapshot must still describe the state the session bound at"
    );
    assert!(
        !snapshot.contains("local_revision"),
        "the snapshot is the converged state, not this machine's counter"
    );
}

// ---------------------------------------------------------------------------
// The self-certification hole, closed
// ---------------------------------------------------------------------------

/// A historical `cairn` run must not license a later attested verification
/// (FR-484, D69).
///
/// The first implementation derived the criterion's *state* from the newest run
/// while deriving its *authority* from every run ever recorded. One genuine
/// Cairn-verified run therefore supplied the authority permanently: after the
/// evidence drifted, an agent could attach its own attested "pass" and the gate
/// would admit it — reporting `authority: cairn` for a check Cairn never ran,
/// and moving the task to `ready`.
///
/// Readiness is the one derived value with an incentive attached, so the window
/// the gate sees is the window that matters.
#[test]
fn a_past_cairn_run_does_not_license_a_later_attestation() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    s.git(&["add", "."]);
    s.git(&["commit", "-m", "config", "--no-gpg-sign"]);

    let task = task_with(&s, &["the port is 8080"]);
    let a = session(&s, "sess-a");
    let ac1 = criterion_id(&s, &task, "AC-1");

    // A genuine Cairn-collected check establishes the criterion.
    let cairn_fact = s.json(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
        "--collector",
        "cairn",
        "--session",
        &a,
    ]);
    let cairn_id = cairn_fact["evidence"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    let ok = s.json(&[
        "task",
        "criterion",
        "verify",
        &ac1,
        "--evidence",
        &cairn_id,
        "--session",
        &a,
    ]);
    assert_eq!(
        ok["criterion"]["verification"], "verified",
        "a Cairn-collected check must establish the criterion: {ok}"
    );

    // The world moves out from under it.
    s.write_file("config/app.yml", "server:\n  port: 9000\n");
    let drifted = s.json_err(&["task", "criterion", "verify", &ac1, "--session", &a]);
    assert!(
        !drifted["code"].as_str().unwrap_or("").is_empty(),
        "a drifted check must not report success: {drifted}"
    );
    assert_eq!(
        criterion(&s, &task, "AC-1")["verification"],
        "unverified",
        "a criterion whose evidence drifted returns to unverified"
    );

    // Now the agent attests that everything is fine.
    let attested = s.json(&[
        "evidence",
        "add",
        "--type",
        "runtime_state",
        "--subject",
        "API port",
        "--value",
        "passed",
        "--locator",
        "config/app.yml#server.port",
        "--collector",
        "agent",
        "--session",
        &a,
    ]);
    let attested_id = attested["evidence"]["id"].as_str().expect("id").to_string();
    let refused = s.json_err(&[
        "task",
        "criterion",
        "verify",
        &ac1,
        "--evidence",
        &attested_id,
        "--session",
        &a,
    ]);

    assert!(
        refused["code"].is_string(),
        "an attestation must not verify a criterion whose Cairn check drifted: {refused}"
    );
    let after = criterion(&s, &task, "AC-1");
    assert_eq!(
        after["verification"], "unverified",
        "the criterion must stay unverified — this is the self-certification the \
         gate exists to prevent: {after}"
    );

    let readiness = s.json(&["task", "readiness", &task]);
    assert_ne!(
        readiness["completion_readiness"], "ready",
        "a task must not reach `ready` on an agent's own attestation: {readiness}"
    );
}

/// A blind write is blind in both halves of a two-field change (FR-490).
#[test]
fn a_two_field_blind_write_is_recorded_as_blind_throughout() {
    let s = Sandbox::new();
    let task = task_with(&s, &["one"]);
    let a = session(&s, "sess-a");
    let ac1 = criterion_id(&s, &task, "AC-1");

    s.must(&[
        "task",
        "criterion",
        "set",
        &ac1,
        "--state",
        "satisfied",
        "--text",
        "reworded",
        "--session",
        &a,
    ]);

    let history = s.json(&["task", "history", &task]);
    let changes = history["changes"].as_array().expect("a change log");
    for kind in ["criterion_state", "criterion_text"] {
        let row = changes
            .iter()
            .find(|c| c["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} row in {changes:?}"));
        assert_eq!(
            row["blind_write"], true,
            "{kind} was written with no expected_revision and must be recorded as blind: {row}"
        );
    }
}

/// A task update that changes nothing does not move the counter (FR-488).
#[test]
fn a_no_op_update_does_not_advance_the_counter() {
    let s = Sandbox::new();
    let task = task_with(&s, &["one"]);
    let before = task_get(&s, &task)["local_revision"].as_i64().expect("rev");

    s.must(&["task", "update", &task, "--title", "Add rate limiting"]);

    assert_eq!(
        task_get(&s, &task)["local_revision"].as_i64().expect("rev"),
        before,
        "setting a field to the value it already held is not a change; the counter \
         must not report one"
    );
}

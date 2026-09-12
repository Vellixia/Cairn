//! The end-to-end acceptance scenario, in one continuous run (T156).
//!
//! `spec.md` §End-to-End Acceptance Scenario defines five phases and requires
//! them to be demonstrated on a real repository rather than improvised later.
//! This is that demonstration, driven the way a developer's machine is driven:
//! vendor hooks, the daemon, a real server, the public control-plane API, a
//! real outage, and a real deletion of the local store.
//!
//! # The rule that makes phases 1 and 2 mean anything
//!
//! **No Cairn tool is invoked in phases 1 and 2.** Not `cairn_remember`, not
//! `cairn_search`, not `cairn_context`. The spec says invoking one to make the
//! demonstration pass invalidates it, and it is right: the feature's claim is
//! that knowledge accumulates from work nobody annotated. Every event below is
//! a vendor lifecycle hook the agent emits anyway.
//!
//! # What phase 3 asserts here, and what asserts it elsewhere
//!
//! Phase 3 is "using only the web interface". A Rust test cannot use a web
//! interface, and a second copy of the browser suite here would prove nothing
//! the browser suite does not. So this file reconstructs the reviewer's trail
//! through the **same public control-plane API those pages call** — session,
//! events, consolidation run, candidate decisions, the accepted record with its
//! provenance, the retrieval and its delivery — and `web/e2e/`'s
//! `feature005-control-plane.spec.ts` covers the screens themselves. The split
//! is deliberate and is the same one T107 settled.

use cairn_e2e::{
    attach_server, binary, get_json_status_bearer, post_json_status_bearer, Sandbox, Server,
};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SETTLE: Duration = Duration::from_secs(45);

/// A second `cairn-server` on the same database, started only for its
/// consolidation task.
///
/// The fixture's own server runs a pool too small to earn a share — below five
/// connections consolidation deliberately does not run, so a small deployment
/// never starves request serving. The way to get a consolidation task is to run
/// a server that qualifies for one, not to widen the fixture.
struct Worker {
    child: Child,
}

impl Worker {
    fn start(database_url: &str) -> Self {
        let child = Command::new(binary("cairn-server"))
            .args([
                "--addr",
                "127.0.0.1:0",
                "--database-url",
                database_url,
                "--max-connections",
                "5",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cairn-server runs");
        Worker { child }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

/// The starting state the spec names: a fresh project, zero durable memories, a
/// supported agent configured, the server reachable, no cached briefing.
struct Fresh {
    sandbox: Sandbox,
    project: Uuid,
    token: String,
}

fn fresh(server: &Server, label: &str) -> Fresh {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let token = server.new_user_token(label);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");
    attach_server(&sandbox, server, &token);
    sandbox.must(&["link", "--project", &project.to_string()]);

    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{project}'"
        )),
        0,
        "the scenario starts from zero durable memories, or it is not the \
         scenario the spec describes"
    );
    Fresh {
        sandbox,
        project,
        token,
    }
}

/// One vendor lifecycle event, through the agent's own hook entry point.
fn hook(f: &Fresh, event: &str, payload: Value) {
    let out = f.sandbox.hook_as("claude-code", event, payload);
    assert!(
        out.ok(),
        "a hook always exits zero — Cairn is never the reason a session breaks \
         (FR-193): {event} said {}",
        out.stderr
    );
}

/// The five phases, in one run.
#[test]
fn the_whole_scenario_runs_on_a_real_repository() {
    let Some(mut server) = Server::start_own_database() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let _worker = Worker::start(&server.database_url);
    let f = fresh(&server, "e2e-acceptance");

    // -----------------------------------------------------------------------
    // Phase 1 — Agent A does real work, and invokes nothing.
    //
    // A defect investigation: read the code, form a conclusion, change the
    // implementation, run the suite until it passes.
    // -----------------------------------------------------------------------
    let a_key = format!("agent-a-{}", Uuid::now_v7());
    for (event, payload) in [
        (
            "SessionStart",
            json!({ "session_id": a_key, "source": "startup" }),
        ),
        (
            "PostToolUse",
            json!({ "session_id": a_key, "tool_name": "Read",
                    "tool_input": { "file_path": "src/widget/parser.rs" },
                    "tool_response": { "exit_code": 0 } }),
        ),
        (
            "PostToolUse",
            json!({ "session_id": a_key, "tool_name": "Bash",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 1 } }),
        ),
        (
            "PostToolUse",
            json!({ "session_id": a_key, "tool_name": "Edit",
                    "tool_input": { "file_path": "src/widget/parser.rs" },
                    "tool_response": { "exit_code": 0 } }),
        ),
        (
            "PostToolUse",
            json!({ "session_id": a_key, "tool_name": "Bash",
                    "tool_input": { "command": "cargo test -p widget" },
                    "tool_response": { "exit_code": 0 } }),
        ),
        (
            "Stop",
            json!({ "session_id": a_key,
                    "last_assistant_message":
                        "we should use parser for widget from now on" }),
        ),
        (
            "SessionEnd",
            json!({ "session_id": a_key, "reason": "clear" }),
        ),
    ] {
        hook(&f, event, payload);
    }

    let project = f.project;
    settle("agent A's session reaches the server", || {
        server.count(&format!(
            "SELECT count(*) FROM sessions WHERE project_id = '{project}'"
        )) > 0
    });
    let session_a: String = server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{project}'
              ORDER BY started_at DESC LIMIT 1"
        ))
        .first()
        .cloned()
        .expect("agent A's session");

    settle("safe events are accepted canonically", || {
        server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session_a}'"
        )) > 0
    });
    settle("consolidation runs", || {
        server.count(&format!(
            "SELECT count(*) FROM consolidation_runs WHERE project_id = '{project}'"
        )) > 0
    });
    settle("durable knowledge results", || {
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{project}'"
        )) > 0
    });

    let records: Vec<String> = server.query_column(&format!(
        "SELECT id::text || '|' || type || '|' || content
           FROM memories WHERE project_id = '{project}' ORDER BY created_at"
    ));
    assert!(
        !records.is_empty(),
        "a session that invoked no Cairn tool produced no durable knowledge, \
         which is the whole claim of the feature"
    );
    // Provenance resolves back to the session and its events (SC-702).
    //
    // Through the candidate's cited events, not through `origin_session_id`.
    // A consolidated record is derived from a *set* of events, and the chain
    // that says which ones is `candidate_source_events → safe_events`; the
    // column names the session a record was filed under, which is a weaker
    // fact and not the one SC-702 is about.
    let consolidated = server.count(&format!(
        "SELECT count(*) FROM memories WHERE project_id = '{project}'
           AND origin_kind <> 'explicit'"
    ));
    assert!(
        consolidated > 0,
        "no consolidated record exists, so there is no provenance to resolve"
    );
    let unresolvable = server.count(&format!(
        "SELECT count(*) FROM memories m
          WHERE m.project_id = '{project}' AND m.origin_kind <> 'explicit'
            AND NOT EXISTS (
                  SELECT 1 FROM knowledge_candidates kc
                    JOIN candidate_source_events cse ON cse.candidate_id = kc.candidate_id
                    JOIN safe_events e ON e.event_id = cse.event_id
                   WHERE kc.result_knowledge_id = m.id AND e.session_id = '{session_a}')"
    ));
    assert_eq!(
        unresolvable, 0,
        "a durable record cannot be traced back to the events it came from. \
         Unresolvable provenance is knowledge nobody can check (SC-702)"
    );

    // -----------------------------------------------------------------------
    // Phase 2 — Agent B benefits, without searching.
    // -----------------------------------------------------------------------
    let b_key = format!("agent-b-{}", Uuid::now_v7());
    let out = f.sandbox.hook_as(
        "claude-code",
        "SessionStart",
        json!({ "session_id": b_key, "source": "startup" }),
    );
    assert!(out.ok(), "agent B's session start: {}", out.stderr);
    let emitted: Value = serde_json::from_str(out.stdout.trim())
        .unwrap_or_else(|e| panic!("the hook emits context JSON ({e}): {:?}", out.stdout));
    let briefing = emitted["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        !briefing.is_empty(),
        "agent B received no briefing at all, so nothing was delivered to \
         benefit from"
    );

    settle("the retrieval is recorded", || {
        server.count(&format!(
            "SELECT count(*) FROM retrieval_traces WHERE project_id = '{project}'"
        )) > 0
    });
    let delivery: Vec<String> = server.query_column(&format!(
        "SELECT delivery_state || '/' || acknowledgement_state
           FROM retrieval_traces WHERE project_id = '{project}'
          ORDER BY created_at DESC LIMIT 1"
    ));
    let recorded = delivery
        .first()
        .cloned()
        .expect("the retrieval that just happened has a trace");
    // At the strength the vendor actually supports, and no more: Claude Code
    // has no acknowledgement channel, so `unavailable` is the honest value and
    // `acknowledged` would be a claim nothing established.
    assert!(
        recorded.starts_with("generated") || recorded.starts_with("transmitted"),
        "the delivery state is `{recorded}`; a briefing that reached the \
         integration is `generated` or `transmitted`, and anything weaker means \
         it did not"
    );
    assert!(
        recorded.ends_with("unavailable"),
        "the trace claims an acknowledgement this vendor has no channel to \
         give: {recorded}"
    );

    // -----------------------------------------------------------------------
    // Phase 3 — the reviewer's trail, through the API those pages call.
    // -----------------------------------------------------------------------
    for (what, path) in [
        (
            "the session list",
            format!("/api/projects/{project}/sessions"),
        ),
        (
            "the consolidation runs",
            format!("/api/projects/{project}/consolidation-runs"),
        ),
        (
            "the retrieval traces",
            format!("/api/projects/{project}/retrieval-traces"),
        ),
        (
            "the memory list",
            format!("/api/projects/{project}/memories"),
        ),
    ] {
        let (body, status) = get_json_status_bearer(&server.base, &path, &f.token);
        assert_eq!(
            status, 200,
            "a reviewer cannot reach {what} at {path}, so the story cannot be \
             traced from the web at all: {body}"
        );
    }
    let (detail, status) = get_json_status_bearer(
        &server.base,
        &format!("/api/sessions/{session_a}"),
        &f.token,
    );
    assert_eq!(status, 200, "agent A's session detail: {detail}");

    // -----------------------------------------------------------------------
    // Phase 4 — the server goes away mid-session.
    // -----------------------------------------------------------------------
    let before_outage = server.count(&format!(
        "SELECT count(*) FROM safe_events WHERE project_id = '{project}'"
    ));
    server.go_offline();

    let c_key = format!("agent-c-{}", Uuid::now_v7());
    for (event, payload) in [
        (
            "SessionStart",
            json!({ "session_id": c_key, "source": "startup" }),
        ),
        (
            "PostToolUse",
            json!({ "session_id": c_key, "tool_name": "Bash",
                    "tool_input": { "command": "cargo build -p widget" },
                    "tool_response": { "exit_code": 0 } }),
        ),
        (
            "SessionEnd",
            json!({ "session_id": c_key, "reason": "clear" }),
        ),
    ] {
        hook(&f, event, payload);
    }
    // The agent remained usable: every hook exited zero above, which is what
    // `hook` asserts, and the spool holds the work rather than losing it.
    let spooled: i64 = f
        .sandbox
        .query_column("SELECT CAST(count(*) AS TEXT) FROM event_spool")[0]
        .parse()
        .expect("a count");
    assert!(
        spooled > 0,
        "an outage produced no spooled work, so either the events were lost or \
         the session was not usable"
    );

    server.come_back();
    // Drained, not merely started: the count below is only a baseline if
    // nothing is still in flight behind it.
    let spool_is_empty = || {
        f.sandbox
            .query_column(
                "SELECT CAST(count(*) AS TEXT) FROM event_spool
                  WHERE state IN ('pending','in_flight')",
            )
            .first()
            .and_then(|n| n.parse::<i64>().ok())
            == Some(0)
    };
    settle("the spool drains once the server is back", || {
        server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE project_id = '{project}'"
        )) > before_outage
            && spool_is_empty()
    });

    // **Replay is idempotent**, and replay means redelivering the *same*
    // spooled rows — not performing the same action twice. Firing the hook
    // again would be a second occurrence and a second event, which is correct
    // and would prove nothing about delivery. So the spool is put back to
    // `pending` and drained a second time.
    let after_replay = server.count(&format!(
        "SELECT count(*) FROM safe_events WHERE project_id = '{project}'"
    ));
    f.sandbox.exec_sql(
        "UPDATE event_spool SET state = 'pending', claimed_at = NULL,
                next_attempt_at = NULL
          WHERE state = 'delivered'",
    );
    settle("the redelivered spool drains again", spool_is_empty);
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE project_id = '{project}'"
        )),
        after_replay,
        "redelivering events the server already holds produced more rows. \
         Delivery is idempotent by construction or the server is not canonical"
    );
    // And no competing local truth: one event id, one row.
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM (
                 SELECT event_id FROM safe_events WHERE project_id = '{project}'
                  GROUP BY event_id HAVING count(*) > 1) d"
        )),
        0,
        "one event reached the server as two rows"
    );

    // -----------------------------------------------------------------------
    // Phase 5 — the machine is lost.
    // -----------------------------------------------------------------------
    let canonical_before = server.count(&format!(
        "SELECT count(*) FROM memories WHERE project_id = '{project}'"
    ));
    assert!(
        canonical_before > 0,
        "the local store is only safe to destroy once the server is confirmed \
         to hold the knowledge"
    );

    // What is about to be destroyed, and what the store itself says about that.
    let durability = f.sandbox.json(&["doctor", "--durability"]);
    let named = durability.to_string();
    for category in [
        "spooled events",
        "integration state",
        "local-only memory",
        "project memory",
        "personal knowledge",
        "team knowledge",
    ] {
        assert!(
            named.contains(category),
            "the durability report does not name `{category}`. A category the \
             report omits is a category lost silently, and the reader takes the \
             omission for an assurance (SC-714): {durability}"
        );
    }

    f.sandbox.stop_daemon();
    std::fs::remove_file(f.sandbox.db_path()).expect("the local store is destroyed");
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = f.sandbox.db_path().into_os_string();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(sidecar));
    }
    f.sandbox.restart_daemon();
    f.sandbox.must(&["init"]);
    attach_server(&f.sandbox, &server, &f.token);
    f.sandbox.must(&["link", "--project", &project.to_string()]);

    // Durable knowledge survives, because it was never only here.
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{project}'"
        )),
        canonical_before,
        "destroying a machine's local store changed the canonical record"
    );
    settle(
        "durable knowledge is reachable again on the rebuilt store",
        || {
            f.sandbox
                .query_column("SELECT CAST(count(*) AS TEXT) FROM memories")
                .first()
                .and_then(|n| n.parse::<i64>().ok())
                .unwrap_or(0)
                > 0
        },
    );

    // And the machine-local half is gone, which Cairn says rather than hides.
    let spool_after: i64 = f
        .sandbox
        .query_column("SELECT CAST(count(*) AS TEXT) FROM event_spool")[0]
        .parse()
        .expect("a count");
    assert_eq!(
        spool_after, 0,
        "a rebuilt store claims to still hold the events that were spooled at \
         the moment of loss. They are gone, and saying otherwise is the \
         unqualified success SC-723 forbids"
    );
}

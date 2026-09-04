//! User Story 4, end to end: the server goes away mid-session and the agent
//! keeps working (T105; SC-715, SC-716, SC-717, FR-781, FR-787, FR-792).
//!
//! # What makes this the *story* test
//!
//! The other US4 files each hold one mechanism still: `feature005_outage.rs`
//! proves the pieces, `feature005_spool_capacity.rs` the capacity policy,
//! `feature005_replay_idempotency.rs` the arithmetic of replay. This one is the
//! sequence a person actually lives through, in order, without resetting
//! anything between the steps — because the failures worth catching here are the
//! ones that only appear when an outage arrives *in the middle* of work that had
//! already started.
//!
//! # The two claims, and they pull against each other
//!
//! **Usability must not degrade.** Every hook exits 0, every explicit command
//! answers, and nothing waits on a server that is not there (FR-781, SC-715).
//! The easy way to satisfy that is to write locally and stop caring about the
//! server.
//!
//! **Authority must not move.** No knowledge becomes canonical because the
//! server is absent (FR-787, SC-717). The easy way to satisfy *that* is to
//! refuse the write, which breaks the first claim.
//!
//! So the interesting assertions are always the pair: the command succeeded
//! **and** it created nothing durable. A test that checked only one of those
//! would pass with the other one broken, and both broken states are shipped
//! defects.
//!
//! # Why the outage stops the process rather than re-pointing the client
//!
//! Re-pointing a client at a dead port looks like an outage and is not one: the
//! endpoint is part of the identity, so `auth token set --server <dead>` makes
//! the client correctly forget which account it was, and every subsequent
//! command is refused for want of an account. That is a sign-out, and testing it
//! here would prove nothing about unreachability.
//!
//! So the server process stops and its address and database stay. The client
//! keeps its token, its endpoint and its account, and simply cannot reach
//! anything — which is the situation FR-781 and FR-787 are written about. It is
//! also reversible without losing the rows the recovery half needs, which
//! dropping the server would not be.

use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

fn server() -> Option<Server> {
    match Server::start_own_database() {
        Some(s) => Some(s),
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            None
        }
    }
}

struct Device {
    sandbox: Sandbox,
    project: Uuid,
}

impl Device {
    fn count(&self, sql: &str) -> i64 {
        self.sandbox
            .query_column(sql)
            .first()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// `cairn status --json`, as the daemon reports it.
    fn status(&self) -> Value {
        self.sandbox.json(&["status"])
    }

    fn spool(&self, which: &str) -> Value {
        self.status()["capture"][which].clone()
    }
}

fn device(server: &Server, label: &str) -> Device {
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

    // The end state, set directly in the store.
    //
    // The cutover that moves a real installation into `server_authoritative` is
    // US7's (T138 onward) and has not shipped, so inventing a command for it
    // here would be inventing US7's interface in a US4 test. What US4 owns is
    // what happens once the mode is set, and this is the smallest way to reach
    // it. Before cutover the whole question is moot: the local store *is* the
    // authority, so an outage cannot move authority anywhere.
    sandbox.stop_daemon();
    sandbox.execute_sql("UPDATE authority_mode SET mode = 'server_authoritative' WHERE id = 1");
    sandbox.must(&["daemon", "start"]);
    sandbox.must(&["sync", "now"]);

    Device { sandbox, project }
}

/// Wait for a condition, driving synchronization each round.
///
/// `Sandbox::settle` allows five seconds and only observes. Recovery here has to
/// re-establish a connection and drain two spools, so this drives `sync now`
/// itself and allows a minute; a passive wait would be measuring the worker's
/// schedule rather than whether the backlog clears.
fn settle_syncing(d: &Device, what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        let _ = d.sandbox.cairn(&["sync", "now"]);
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("timed out waiting for: {what}");
}

/// The whole sequence: work, outage, more work, recovery, and the arithmetic.
///
/// **Falsified by** any of: a handler writing locally instead of queueing under
/// server authority; a drain marking a row delivered on a transport error; the
/// ingest route creating a second canonical event for a replayed id; or the
/// status report claiming the queue is fine while it is not.
#[test]
fn an_outage_mid_session_costs_freshness_and_never_authority() {
    let Some(mut server) = server() else { return };
    let d = device(&server, "us4");

    // ---------------------------------------------------------------------
    // 1. Work, with the server there. This is the baseline the outage
    //    interrupts, and it exists so nothing later is a first attempt.
    // ---------------------------------------------------------------------
    let session = format!("us4-{}", Uuid::now_v7());
    let out = d.sandbox.hook(
        "SessionStart",
        json!({ "session_id": session, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "session start: {}", out.stderr);

    let before = d
        .sandbox
        .json(&["memory", "add", "the parser owns validation"]);
    assert_eq!(
        before["accepted_for_delivery"],
        json!(true),
        "under server authority even a healthy write is a request, not a \
         durable record (FR-712): {before}"
    );

    settle_syncing(&d, "the first command reaches the server", || {
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            d.project
        )) > 0
    });
    let accepted_before = server.count(&format!(
        "SELECT count(*) FROM memories WHERE project_id = '{}'",
        d.project
    ));
    let local_memories_before = d.count("SELECT CAST(COUNT(*) AS TEXT) FROM memories");

    // ---------------------------------------------------------------------
    // 2. The server goes away, mid-session.
    // ---------------------------------------------------------------------
    server.go_offline();

    // Capture keeps working. Every hook exits 0 — a hook that failed would
    // surface in the agent's own transcript, which is precisely the "Cairn
    // broke my session" outcome FR-781 exists to prevent.
    for (event, payload) in [
        (
            "PostToolUse",
            json!({ "session_id": session, "tool_name": "Edit",
                    "tool_input": { "file_path": "src/parser.rs" } }),
        ),
        (
            "PostToolUse",
            json!({ "session_id": session, "tool_name": "Bash",
                    "tool_input": { "command": "cargo test" } }),
        ),
        ("Stop", json!({ "session_id": session })),
    ] {
        let out = d.sandbox.hook(event, payload);
        assert_eq!(
            out.code, 0,
            "{event} failed while the server was away, which blocks the agent: {}",
            out.stderr
        );
    }

    // Explicit commands keep working, and keep being requests. Each of these is
    // a different handler and a different command kind, and each was a local
    // durable write until T097 routed it — the reason to exercise several is
    // that they were wrong independently.
    // Personal knowledge has no `cairn personal add`: the CLI's `personal`
    // subcommand is list/forget, and creation is an MCP action
    // (`cairn_remember` with `domain: "personal"`). Driving it through MCP is
    // therefore not a detour — it is the only entry point that exists, and it is
    // the one an agent actually uses.
    let mut mcp = cairn_e2e::Mcp::start(&d.sandbox);
    let cwd = d.sandbox.repo_dir();
    let cwd = cwd.to_string_lossy().to_string();
    let personal = mcp.tool_result(
        "cairn_remember",
        json!({
            "action": "create",
            "content": "I read the failing assertion before the stack",
            "domain": "personal",
            "type": "convention",
        }),
        &cwd,
    );
    // MCP wraps a reply in `content[0].text`. Unwrapped here so the assertions
    // below read the same shape as the CLI's, rather than each of them knowing
    // which transport its value arrived over.
    assert_eq!(
        personal["isError"],
        json!(false),
        "the personal write failed during the outage: {personal}"
    );
    let personal = personal["content"][0]["text"].clone();

    let during: Vec<(&str, Value)> = vec![
        (
            "project memory",
            d.sandbox
                .json(&["memory", "add", "the ingest boundary owns rejection"]),
        ),
        ("personal knowledge", personal),
        (
            "team proposal",
            d.sandbox.json(&[
                "team",
                "propose",
                "a migration ships with the code that needs it",
            ]),
        ),
    ];
    for (what, reply) in &during {
        assert_eq!(
            reply["accepted_for_delivery"],
            json!(true),
            "{what} did not answer accepted-for-delivery during the outage: {reply}"
        );
        assert!(
            reply.get("stored").is_none() && reply.get("memory").is_none(),
            "{what} reported a durable record while the server was unreachable, \
             which is the canonical fork FR-787 forbids: {reply}"
        );
    }

    // **Nothing became canonical locally.** The sharpest assertion in the file:
    // three explicit writes happened and the local knowledge tables did not
    // grow. If any handler still wrote locally, this is where it shows.
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM memories"),
        local_memories_before,
        "a local durable memory appeared during the outage (FR-709, FR-787)"
    );
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge WHERE forgotten_at IS NULL"),
        0,
        "a local durable personal record appeared during the outage"
    );
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge"),
        0,
        "a local durable team record appeared during the outage"
    );

    // The backlog is visible and says why (FR-792). Depth alone is not enough:
    // a queue with no age and no reason is a number nobody can act on.
    let commands = d.spool("commands");
    assert!(
        commands["undelivered"].as_i64().unwrap_or(0) >= 3,
        "the three queued commands are not reported: {commands}"
    );
    assert!(
        commands["oldest_at"].is_string(),
        "the queue reports no oldest entry: {commands}"
    );
    let events = d.spool("events");
    assert!(
        events["undelivered"].as_i64().unwrap_or(0) > 0,
        "capture produced nothing to deliver, so the outage half of this test \
         would pass vacuously: {events}"
    );

    // ---------------------------------------------------------------------
    // 3. The server comes back. Nobody repairs anything.
    // ---------------------------------------------------------------------
    server.come_back();

    settle_syncing(&d, "the backlog drains", || {
        let c = d.spool("commands");
        let e = d.spool("events");
        c["undelivered"].as_i64().unwrap_or(1) == 0 && e["undelivered"].as_i64().unwrap_or(1) == 0
    });

    // Exactly the three commands issued during the outage landed, and the one
    // from before did not land twice.
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            d.project
        )),
        accepted_before + 1,
        "the project memory queued during the outage did not land exactly once"
    );
    assert_eq!(
        server.count("SELECT count(*) FROM personal_knowledge"),
        1,
        "the personal record queued during the outage did not land exactly once"
    );
    assert_eq!(
        server.count("SELECT count(*) FROM team_knowledge WHERE state = 'proposed'"),
        1,
        "the team proposal queued during the outage did not land exactly once"
    );

    // The queue reports itself healthy again, and says nothing is blocking.
    let commands = d.spool("commands");
    assert!(
        commands["blocked_reason"].is_null() || commands["blocked_reason"].as_str().is_none(),
        "the queue still reports a blocker after draining: {commands}"
    );
    assert!(
        commands["oldest_at"].as_str().is_none(),
        "an empty queue still reports an oldest entry: {commands}"
    );

    // ---------------------------------------------------------------------
    // 4. Replay is once, even now. Draining again changes nothing.
    // ---------------------------------------------------------------------
    let after_first_drain = server.count(&format!(
        "SELECT count(*) FROM memories WHERE project_id = '{}'",
        d.project
    ));
    for _ in 0..3 {
        let _ = d.sandbox.cairn(&["sync", "now"]);
    }
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            d.project
        )),
        after_first_drain,
        "a repeated drain created a second durable effect (FR-786, SC-716)"
    );

    // And the events landed once each. Identity is UUIDv5 over the session and
    // its durable ordinal, so a replay carries the same id and the server
    // answers `duplicate` — this asserts the arithmetic that follows from it.
    let distinct: i64 = server
        .query_column("SELECT count(DISTINCT event_id)::text FROM safe_events")
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let total = server.count("SELECT count(*) FROM safe_events");
    assert_eq!(
        total, distinct,
        "the same event exists more than once after replay (SC-716)"
    );
    assert!(
        total > 0,
        "no canonical event survived the outage, so the replay assertion above \
         proved nothing"
    );
}

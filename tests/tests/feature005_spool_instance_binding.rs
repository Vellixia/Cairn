//! Queued work belongs to the server it was queued for, and to no other
//! (FR-791, FR-495, FR-496).
//!
//! # Why this file exists separately from `feature005_identity_outage.rs`
//!
//! That file's replacement-deployment test changes the *account* as well as the
//! deployment, because `Server::replaced_at_same_address` gives the new process
//! a database of its own and the old account does not exist in it. So it proves
//! the account rule (FR-790) a second time and proves nothing at all about the
//! instance rule: remove every instance check from the code and it still passes,
//! because the account check alone refuses everything it asserts. A test that
//! cannot fail when the property it names is deleted is not evidence for it.
//!
//! # What makes this one load-bearing
//!
//! **Everything except the server instance is held identical.** Same database,
//! same account row, same token, same project, same session, same spooled rows,
//! same URL, same port, same process image. The only thing that changes is the
//! single value in `server_instance` — the one field whose entire purpose is to
//! say "which deployment is this". If a drain delivers under S2 work that was
//! queued under S1, there is nothing else it could have been confused by.
//!
//! That also makes it the sharpest possible statement of the rule the code has
//! to follow: **an endpoint is not an identity**. Every weaker mechanism — URL
//! equality, reachability, "the token still works" — is satisfied throughout
//! this test, and every one of them would deliver.
//!
//! # And the work must survive
//!
//! Refusing to deliver is only half. A mismatch must leave the rows intact,
//! unattempted and visible, because the operator's remedy is to point the store
//! back at its own server — and rows discarded, refused, or driven to
//! `retry_exhausted` while the wrong deployment was answering would not be there
//! when they did. So the test asserts the depth, the states and the attempt
//! counts across the mismatch, then restores S1 and requires exactly-once
//! delivery of the very same rows.

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

    fn column(&self, sql: &str) -> Vec<String> {
        self.sandbox.query_column(sql)
    }

    fn spool(&self, which: &str) -> Value {
        self.sandbox.json(&["status"])["capture"][which].clone()
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

    // Server-authoritative, so an explicit write is a command rather than a
    // local record and there is something in the command spool to bind. The
    // cutover that reaches this state on a real installation is US7's and has
    // not shipped; this is the smallest way to the mode US4 is about.
    sandbox.stop_daemon();
    sandbox.execute_sql("UPDATE authority_mode SET mode = 'server_authoritative' WHERE id = 1");
    sandbox.must(&["daemon", "start"]);
    sandbox.must(&["sync", "now"]);

    Device { sandbox, project }
}

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

/// Wait for a condition to hold for a whole window, not merely to occur.
///
/// The mismatch assertions are about something *not happening*, and a single
/// observation cannot establish that — it only says the delivery had not landed
/// yet. This drives synchronization throughout the window, so the daemon has
/// every opportunity to do the wrong thing.
fn never_during(d: &Device, window: Duration, what: &str, mut wrong: impl FnMut() -> bool) {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        assert!(!wrong(), "{what}");
        let _ = d.sandbox.cairn(&["sync", "now"]);
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The whole of the rule, in one sequence.
///
/// **Falsified by** deleting either `AND server_instance_id = ?` from
/// `claim_events`/`claim_commands`, or by making the claim treat `NULL` as a
/// wildcard. Verified: with the event claim's clause removed, the mismatch
/// assertion fails within the first second of the window.
#[test]
fn work_queued_for_one_server_instance_is_never_delivered_to_another() {
    let Some(mut server) = server() else { return };
    let d = device(&server, "instance");

    let s1: Uuid = server
        .text("SELECT id::text FROM server_instance")
        .parse()
        .expect("the server has an instance id");

    // -----------------------------------------------------------------------
    // 1. Queue one event and one command against S1, with the server away so
    //    they stay queued. The outage is a stopped process, not a re-pointed
    //    endpoint: re-pointing is a credential transition and would change the
    //    account, which is the very confound this file exists to remove.
    // -----------------------------------------------------------------------
    let key = format!("instance-{}", Uuid::now_v7());
    let out = d.sandbox.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "session start: {}", out.stderr);
    settle_syncing(&d, "the session reaches S1", || {
        server.count(&format!(
            "SELECT count(*) FROM sessions WHERE project_id = '{}'",
            d.project
        )) > 0
    });

    server.go_offline();

    let out = d.sandbox.hook(
        "PostToolUse",
        json!({
            "session_id": key, "tool_name": "Edit",
            "tool_input": { "file_path": "src/binding.rs" }
        }),
    );
    assert_eq!(out.code, 0, "a hook must not fail during an outage");
    let queued = d
        .sandbox
        .json(&["memory", "add", "the spool binds to an instance, not a URL"]);
    assert_eq!(
        queued["accepted_for_delivery"],
        json!(true),
        "the command was not queued: {queued}"
    );

    // Every queued row is bound to S1, and that is the fact the rest turns on.
    let bound_events = d.column(
        "SELECT event_id FROM event_spool
          WHERE state IN ('pending','in_flight','failed') ORDER BY event_id",
    );
    let bound_commands = d.column(
        "SELECT command_id FROM command_spool
          WHERE state IN ('pending','in_flight','failed') ORDER BY command_id",
    );
    assert!(
        !bound_events.is_empty() && !bound_commands.is_empty(),
        "nothing was queued, so every assertion below would pass vacuously"
    );
    assert_eq!(
        d.count(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE server_instance_id = '{s1}'"
        )) as usize,
        bound_events.len(),
        "an event was queued without being bound to the instance it was queued for"
    );
    assert_eq!(
        d.count(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool
              WHERE server_instance_id = '{s1}'"
        )) as usize,
        bound_commands.len(),
        "a command was queued without being bound to the instance it was queued for"
    );

    // -----------------------------------------------------------------------
    // 2. Change the server's identity and **nothing else**. Same database, same
    //    account, same token, same project, same session, same address.
    // -----------------------------------------------------------------------
    let s2 = Uuid::now_v7();
    server.execute(&format!("UPDATE server_instance SET id = '{s2}'"));
    server.come_back();
    assert_eq!(
        server.text("SELECT id::text FROM server_instance"),
        s2.to_string(),
        "the server did not take its new identity"
    );
    assert_ne!(s1, s2);

    // Worth being explicit about what did *not* change, because it is the whole
    // argument: the account row, the token, the project, the session, the
    // spooled rows and the address are byte-for-byte what they were. Every
    // weaker notion of "same server" — the URL resolves, the port answers, the
    // credential authenticates — is still satisfied here, and every one of them
    // would deliver.

    // -----------------------------------------------------------------------
    // 3. Nothing is delivered, refused, or discarded.
    // -----------------------------------------------------------------------
    let attempts_before: Vec<String> =
        d.column("SELECT event_id || '=' || attempts FROM event_spool ORDER BY event_id");
    never_during(
        &d,
        Duration::from_secs(12),
        "queued work reached a server instance it was not queued for (FR-791)",
        || {
            server.count(&format!(
                "SELECT count(*) FROM safe_events WHERE project_id = '{}'",
                d.project
            )) > 0
        },
    );

    assert_eq!(
        d.column(
            "SELECT event_id FROM event_spool
              WHERE state IN ('pending','in_flight','failed') ORDER BY event_id"
        ),
        bound_events,
        "the mismatched events did not survive intact: work queued for another \
         deployment must be held, not discarded, because pointing the store back \
         at its own server is the remedy and there would be nothing left to send"
    );
    assert_eq!(
        d.column(
            "SELECT command_id FROM command_spool
              WHERE state IN ('pending','in_flight','failed') ORDER BY command_id"
        ),
        bound_commands,
        "the mismatched commands did not survive intact"
    );
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM event_spool WHERE state = 'refused'"),
        0,
        "a mismatch refused work it was never entitled to judge"
    );
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM command_spool WHERE state = 'refused'"),
        0,
        "a mismatch refused a command it was never entitled to judge"
    );
    assert_eq!(
        d.column("SELECT event_id || '=' || attempts FROM event_spool ORDER BY event_id"),
        attempts_before,
        "a mismatch spent delivery attempts: an arbitrarily long period pointed at \
         the wrong server would drive every row to retry_exhausted, which is a \
         terminal verdict about work the right server never saw"
    );

    // And it is visible rather than silently stuck.
    let events = d.spool("events");
    assert!(
        events["other_instance"].as_i64().unwrap_or(0) > 0,
        "the backlog belongs to another deployment and the report does not say \
         so: {events}"
    );
    assert_eq!(
        events["blocked_reason"].as_str(),
        Some("server_instance_mismatch"),
        "the reason delivery is not progressing is not reported (FR-792): {events}"
    );

    // -----------------------------------------------------------------------
    // 4. Give the identity back, and the very same rows deliver exactly once.
    // -----------------------------------------------------------------------
    server.execute(&format!("UPDATE server_instance SET id = '{s1}'"));
    server.come_back();

    settle_syncing(
        &d,
        "the held work delivers once the right server returns",
        || {
            d.count(
                "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE state IN ('pending','in_flight','failed')",
            ) == 0
                && d.count(
                    "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool
                  WHERE state IN ('pending','in_flight','failed')",
                ) == 0
        },
    );

    for id in &bound_events {
        assert_eq!(
            server.count(&format!(
                "SELECT count(*) FROM safe_events WHERE event_id = '{id}'"
            )),
            1,
            "event {id} did not land exactly once after the right server returned"
        );
    }
    assert_eq!(
        server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            d.project
        )),
        1,
        "the held command did not land exactly once"
    );
    let events = d.spool("events");
    assert_eq!(
        events["other_instance"].as_i64(),
        Some(0),
        "the report still claims work belongs elsewhere: {events}"
    );
}

/// Work queued before this store ever knew an instance is adopted once, by the
/// first server it actually talks to — and never re-adopted after that.
///
/// The safe first-binding rule. A `NULL` binding is "queued before there was an
/// instance to name", which is a real state for a store that captured something
/// before its first successful sync. Treating it as a wildcard would be the
/// pre-binding behaviour restated, and would hand that work to whichever
/// deployment answered first.
///
/// **The rows have to be genuinely undelivered for this to prove anything.** An
/// earlier version nulled the bindings on a spool that had already drained and
/// then waited for the depth to reach zero, which it already had — so removing
/// adoption entirely left the test green. It is written the other way round now:
/// the work is queued while the server is away, unbound while it is still
/// queued, and only then is the server given back.
///
/// **Falsified by** deleting the `adopt_unbound_rows` call from `claim_events`:
/// the unbound rows are then claimable by nobody and the settle times out.
#[test]
fn work_queued_before_any_instance_binds_once_and_is_not_rebound() {
    let Some(mut server) = server() else { return };
    let d = device(&server, "firstbind");
    let s1 = server.text("SELECT id::text FROM server_instance");

    // Queue real work with the server away, so it is still in the spool when the
    // binding is removed.
    let key = format!("firstbind-{}", Uuid::now_v7());
    let out = d.sandbox.hook(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "session start: {}", out.stderr);
    settle_syncing(&d, "the session reaches the server", || {
        server.count(&format!(
            "SELECT count(*) FROM sessions WHERE project_id = '{}'",
            d.project
        )) > 0
    });

    server.go_offline();
    for i in 0..3 {
        let out = d.sandbox.hook(
            "PostToolUse",
            json!({
                "session_id": key, "tool_name": "Read",
                "tool_input": { "file_path": format!("src/first{i}.rs") }
            }),
        );
        assert_eq!(out.code, 0, "a hook must not fail during an outage");
    }
    // Wait for the depth to *stop moving*, not merely to become non-zero.
    //
    // Capture is fire-and-forget and one vendor hook can produce several
    // canonical events, so unbinding the instant the spool is non-empty catches
    // it mid-fill — and a row that lands a moment later is spooled with its
    // binding intact, which then reads as "adoption did not happen" when in
    // fact that row never needed adopting.
    let mut queued = 0;
    let mut steady = 0;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && steady < 6 {
        let now = d.count(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE state IN ('pending','in_flight','failed')",
        );
        steady = if now > 0 && now == queued {
            steady + 1
        } else {
            0
        };
        queued = now;
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        queued > 0,
        "nothing was queued, so unbinding it would prove nothing"
    );

    // Now make it look like work from before this store ever synchronized.
    // Reached by hand because `device` has to link and sync to have a project at
    // all, and the state under test is precisely the one before that.
    // The daemon keeps running. Stopping it to make this edit and starting it
    // again put the store in a state the drain never recovered from within the
    // window, and the restart is not what is under test — SQLite's WAL takes a
    // second writer perfectly well for two statements.
    d.sandbox
        .execute_sql("UPDATE event_spool SET server_instance_id = NULL");
    d.sandbox
        .execute_sql("UPDATE command_spool SET server_instance_id = NULL");
    // Counted again rather than compared against the earlier snapshot. Capture
    // is fire-and-forget, so one hook can still be landing rows while the next
    // statement runs — the earlier count is a lower bound on what is queued, not
    // an equality, and asserting it as one measures the scheduler.
    let unbound = d.count(
        "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
          WHERE server_instance_id IS NULL
            AND state IN ('pending','in_flight','failed')",
    );
    assert!(
        unbound >= queued,
        "the unbinding did not take: {unbound} unbound of at least {queued} queued"
    );
    assert_eq!(
        d.count(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE server_instance_id IS NOT NULL
                AND state IN ('pending','in_flight','failed')"
        ),
        0,
        "a queued row kept its binding, so adoption is not what would deliver it"
    );

    server.come_back();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if d.count(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE state IN ('pending','in_flight','failed')",
        ) == 0
        {
            break;
        }
        let _ = d.sandbox.cairn(&["sync", "now"]);
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(
        d.count(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE state IN ('pending','in_flight','failed')"
        ),
        0,
        "the unbound rows were never adopted and delivered. states={:?} bindings={:?} \
         row_accounts={:?} config_account={:?} blocked={:?}",
        d.column("SELECT state || ' x' || CAST(COUNT(*) AS TEXT) FROM event_spool GROUP BY state"),
        d.column(
            "SELECT COALESCE(server_instance_id, '<unbound>') || ' x'
                    || CAST(COUNT(*) AS TEXT)
               FROM event_spool GROUP BY server_instance_id"
        ),
        d.column("SELECT DISTINCT account_id FROM event_spool"),
        std::fs::read_to_string(d.sandbox.cairn_home().join("config.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|c| c["server_account_id"].clone()),
        d.spool("events"),
    );

    // Adoption happened, once, and it named the server that actually answered.
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM event_spool WHERE server_instance_id IS NULL"),
        0,
        "a row stayed unbound after a drain against an established instance"
    );
    assert_eq!(
        d.count(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE server_instance_id <> '{s1}'"
        )),
        0,
        "adoption bound a row to something other than the server it spoke to"
    );
    assert!(
        server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE project_id = '{}'",
            d.project
        )) >= unbound,
        "the adopted rows did not all deliver: the server holds fewer events than \
         were adopted. The comparison is `>=` rather than `>` because the \
         pre-outage `session_opened` event was itself unbound by the statement \
         above and re-adopted with the rest — it is inside the count, not \
         additional to it."
    );
}

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
    // **What the queue looked like when the deadline passed.**
    //
    // "Timed out waiting for X" cannot distinguish work that was never
    // claimable from work claimed and rejected, or from a store still keyed to
    // the wrong deployment — and those have opposite repairs. The rows carry
    // their binding and their attempt count, and `sync_cursor` carries the
    // instance the store believes it is talking to, so the two can be compared.
    panic!(
        "timed out waiting for: {what}\n  events: {:?}\n  commands: {:?}\n  \
         established namespaces: {:?}",
        d.column(
            "SELECT event_id || ' kind=' || kind || ' state=' || state
                    || ' attempts=' || CAST(attempts AS TEXT)
                    || ' instance=' || COALESCE(server_instance_id, '<none>')
               FROM event_spool ORDER BY created_at, event_id"
        ),
        d.column(
            "SELECT command_id || ' state=' || state
                    || ' attempts=' || CAST(attempts AS TEXT)
                    || ' instance=' || COALESCE(server_instance_id, '<none>')
               FROM command_spool ORDER BY command_id"
        ),
        d.column("SELECT namespace FROM sync_cursor ORDER BY namespace"),
    );
}

/// What the daemon logged about which instance it saw and compared against.
///
/// `cairn::observation` is the target the two trace points use: one where an
/// `AuthenticatedContext` learns the endpoint's instance, one where the spool
/// report chooses what to measure rows against. Read from the daemon's own log
/// because the value lives in that process, not in the store.
fn observation_trace(d: &Device) -> String {
    let path = d.sandbox.home.path().join("cairnd.log");
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = body
        .lines()
        .filter(|l| l.contains("cairn::observation") || l.contains("server instance"))
        .rev()
        .take(12)
        .collect();
    lines
        .into_iter()
        .rev()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
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
    // **Asked as "nothing is bound to anything else", not as a count.**
    //
    // These used to compare an all-states `server_instance_id = s1` count
    // against `bound_events`, which selects only `pending`/`in_flight`/
    // `failed`. Those two are equal only while nothing has been delivered yet
    // — and this test guarantees the opposite a few lines above, where
    // `settle_syncing` waits for the session to actually reach S1 before the
    // outage. A row that was bound to S1 *and* delivered is the arrangement
    // working, but it counts on the left and not on the right, so the
    // assertion failed as `left: 3, right: 2` whenever the drain won the race
    // — never in isolation, only under a loaded full-workspace run.
    //
    // The form below is strictly stronger than the count it replaces: it holds
    // every row in every state to the binding, so a row bound to the wrong
    // instance or left unbound fails it, in states the old comparison could
    // not see at all. What it no longer does is mistake delivery for a missing
    // binding.
    assert_eq!(
        d.count(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE server_instance_id IS DISTINCT FROM '{s1}'"
        )),
        0,
        "an event was queued without being bound to the instance it was queued \
         for; bindings={:?} states={:?}",
        d.column(
            "SELECT COALESCE(server_instance_id, '<unbound>') || ' x'
                    || CAST(COUNT(*) AS TEXT)
               FROM event_spool GROUP BY server_instance_id"
        ),
        d.column("SELECT state || ' x' || CAST(COUNT(*) AS TEXT) FROM event_spool GROUP BY state"),
    );
    assert_eq!(
        d.count(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool
              WHERE server_instance_id IS DISTINCT FROM '{s1}'"
        )),
        0,
        "a command was queued without being bound to the instance it was queued \
         for; bindings={:?} states={:?}",
        d.column(
            "SELECT COALESCE(server_instance_id, '<unbound>') || ' x'
                    || CAST(COUNT(*) AS TEXT)
               FROM command_spool GROUP BY server_instance_id"
        ),
        d.column(
            "SELECT state || ' x' || CAST(COUNT(*) AS TEXT) FROM command_spool GROUP BY state"
        ),
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
    // **Measured against what S1 legitimately already had, not against zero.**
    //
    // FR-791 forbids work reaching a server instance it was not queued for, so
    // the violation is an *increase* during the window. Watching for a non-zero
    // count instead made the test fail on its first sample whenever the
    // session-open event had already been delivered — which step 1 permits,
    // because it waits for the *session* to reach S1, not for that event. The
    // event reached S1 while S1 was still the right instance, which is the
    // arrangement working; reported as an FR-791 violation it looked like the
    // opposite. Reproduced locally at roughly one run in twenty-five before
    // this baseline existed.
    let delivered_before = server.count(&format!(
        "SELECT count(*) FROM safe_events WHERE project_id = '{}'",
        d.project
    ));
    never_during(
        &d,
        Duration::from_secs(12),
        "queued work reached a server instance it was not queued for (FR-791)",
        || {
            server.count(&format!(
                "SELECT count(*) FROM safe_events WHERE project_id = '{}'",
                d.project
            )) > delivered_before
        },
    );

    // **Every row queued for S1 is still queued — extra rows are not a
    // violation.**
    //
    // Asked as containment rather than as set equality, and the difference is
    // the requirement. FR-791 is about work *reaching a server it was not
    // queued for*; it says nothing about the spool being frozen. A live daemon
    // is attached to this sandbox for the whole twelve-second window, and
    // ordinary capture may legitimately arrive during it — which is exactly
    // what CI saw when a snapshot of two rows was compared against three and
    // the equality failed over an arrival that broke nothing.
    //
    // The loss direction stays strict: nothing that was queued may leave the
    // undelivered set, because being held is the whole claim. Bounded-spool
    // shedding (FR-785) cannot account for a loss here — the bound is fifty
    // thousand events and this spool holds single digits — so a missing row is
    // unexplained and must fail loudly, with the whole spool in the message so
    // the next occurrence says what happened to it rather than only that it is
    // gone.
    let held_events = d.column(
        "SELECT event_id FROM event_spool
          WHERE state IN ('pending','in_flight','failed') ORDER BY event_id",
    );
    for id in &bound_events {
        assert!(
            held_events.contains(id),
            "event {id} was queued for another deployment and did not survive: \
             work queued for another deployment must be held, not discarded, \
             because pointing the store back at its own server is the remedy \
             and there would be nothing left to send. Spool now: {:?}",
            d.column(
                "SELECT event_id || ' kind=' || kind
                        || ' class=' || CAST(boundary_class AS TEXT)
                        || ' state=' || state
                        || ' attempts=' || CAST(attempts AS TEXT)
                        || ' instance=' || COALESCE(server_instance_id, '<none>')
                   FROM event_spool ORDER BY created_at, event_id"
            )
        );
    }
    let held_commands = d.column(
        "SELECT command_id FROM command_spool
          WHERE state IN ('pending','in_flight','failed') ORDER BY command_id",
    );
    for id in &bound_commands {
        assert!(
            held_commands.contains(id),
            "command {id} was queued for another deployment and did not survive. \
             Spool now: {:?}",
            d.column(
                "SELECT command_id || ' state=' || state
                        || ' attempts=' || CAST(attempts AS TEXT)
                        || ' instance=' || COALESCE(server_instance_id, '<none>')
                   FROM command_spool ORDER BY command_id"
            )
        );
    }
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
    // Per row rather than as a whole-set comparison, for the same reason: a row
    // that arrived during the window has no "before" to be compared against,
    // and its arrival is not what this asserts.
    let attempts_now =
        d.column("SELECT event_id || '=' || attempts FROM event_spool ORDER BY event_id");
    for before in &attempts_before {
        assert!(
            attempts_now.contains(before),
            "a mismatch spent delivery attempts on {before}: an arbitrarily long \
             period pointed at the wrong server would drive every row to \
             retry_exhausted, which is a terminal verdict about work the right \
             server never saw. Attempts now: {attempts_now:?}"
        );
    }

    // And it is visible rather than silently stuck.
    let events = d.spool("events");
    assert!(
        events["other_instance"].as_i64().unwrap_or(0) > 0,
        "the backlog belongs to another deployment and the report does not say \
         so: {events}\n  s1={s1} s2={s2}\n  rows: {:?}\n  daemon observation trace:\n{}",
        d.column(
            "SELECT event_id || ' state=' || state || ' instance='
                    || COALESCE(server_instance_id, '<none>')
               FROM event_spool ORDER BY created_at, event_id"
        ),
        observation_trace(&d),
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
    //
    // **Scoped to rows that are actually queued**, and that scope is
    // load-bearing rather than tidy. `settle_syncing` above waits for the
    // session to reach the server, and under load a capture event can drain
    // with it — so by this point the spool may already hold a `delivered` row.
    // Nulling that row's binding too produced a `delivered` row bound to
    // nothing, which adoption then correctly never touched: it binds rows it
    // can *claim*, and a delivered row is not claimable. The final assertion
    // counts `IS NULL` across every state, so that row failed it — a green
    // test whenever the pre-outage drain lost the race and a red one whenever
    // it won, which is a test measuring the scheduler rather than adoption.
    d.sandbox.execute_sql(
        "UPDATE event_spool SET server_instance_id = NULL
          WHERE state IN ('pending','in_flight','failed')",
    );
    d.sandbox.execute_sql(
        "UPDATE command_spool SET server_instance_id = NULL
          WHERE state IN ('pending','in_flight','failed')",
    );
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
        "a row stayed unbound after a drain against an established instance. \
         bindings={:?} states={:?} row_accounts={:?}",
        d.column(
            "SELECT COALESCE(server_instance_id, '<unbound>') || ' x'
                    || CAST(COUNT(*) AS TEXT)
               FROM event_spool GROUP BY server_instance_id"
        ),
        d.column("SELECT state || ' x' || CAST(COUNT(*) AS TEXT) FROM event_spool GROUP BY state"),
        d.column("SELECT DISTINCT account_id FROM event_spool"),
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

// ---------------------------------------------------------------------------
// The report has to survive the daemon that saw the replacement
// (FR-792, FR-792a–FR-792d, SC-718a)
// ---------------------------------------------------------------------------

/// The whole of `sync_cursor`, as text, so "the probe changed nothing" is an
/// assertion about the table rather than about a field somebody remembered to
/// check.
///
/// Only usable while the server is unreachable — see [`sync_lanes`] for why.
fn sync_state_strict(d: &Device) -> Vec<String> {
    d.column(
        "SELECT namespace
                || ' pull=' || COALESCE(pull_cursor, '<none>')
                || ' last_success=' || COALESCE(last_success_at, '<none>')
                || ' capability=' || COALESCE(CAST(server_capability AS TEXT), '<none>')
           FROM sync_cursor ORDER BY namespace",
    )
}

/// The lanes and their cursors — the part of `sync_cursor` a status probe could
/// conceivably touch.
///
/// **Why not the whole table here.** `last_success_at` and `server_capability`
/// are the background worker's record of a sync that succeeded, and the worker
/// runs on its own half-second tick beside whatever a test is doing. A
/// reachable server therefore moves `project:`'s `last_success_at` between any
/// two reads a few hundred milliseconds apart, and an assertion over the whole
/// table would be measuring the worker rather than the probe — it would fail on
/// correct behaviour and pass or fail by timing.
///
/// What is left is exactly what FR-792b forbids the probe from doing: adding or
/// removing a lane — an adopted `team:S2` is the failure this whole file exists
/// to prevent — and moving a cursor. The stronger whole-table assertion is
/// still made, in the one state where the worker provably cannot write either
/// column: with the endpoint down, no sync can succeed, so anything that
/// changed was changed by the read itself.
fn sync_lanes(d: &Device) -> Vec<String> {
    d.column(
        "SELECT namespace || ' pull=' || COALESCE(pull_cursor, '<none>')
           FROM sync_cursor ORDER BY namespace",
    )
}

/// Every undelivered row with its state, binding and attempt count.
fn queued_rows(d: &Device) -> Vec<String> {
    let mut rows = d.column(
        "SELECT 'event ' || event_id || ' state=' || state
                || ' attempts=' || CAST(attempts AS TEXT)
                || ' instance=' || COALESCE(server_instance_id, '<none>')
           FROM event_spool WHERE state IN ('pending','in_flight','failed')
          ORDER BY event_id",
    );
    rows.extend(d.column(
        "SELECT 'command ' || command_id || ' state=' || state
                || ' attempts=' || CAST(attempts AS TEXT)
                || ' instance=' || COALESCE(server_instance_id, '<none>')
           FROM command_spool WHERE state IN ('pending','in_flight','failed')
          ORDER BY command_id",
    ));
    rows
}

/// The team lane is the durable binding, so this is what "S2 was not adopted"
/// means concretely: no lane for it, in any state.
fn team_lanes(d: &Device) -> Vec<String> {
    d.column("SELECT namespace FROM sync_cursor WHERE namespace LIKE 'team:%' ORDER BY namespace")
}

/// FR-792 across daemon lifetimes, which is where it was broken.
///
/// # The defect this exists to prevent
///
/// The instance the spool report measured rows against was
/// `Daemon::last_observed_instance` — in memory, per process — falling back to
/// the store's own binding when this process had not yet spoken to the server.
/// Both halves are wrong for the same reason and the fallback is the worse one:
/// comparing rows to the binding answers "no rows belong elsewhere" *precisely*
/// when a replacement deployment has arrived, because the rows and the binding
/// agree and it is the peer that has changed.
///
/// That would be a narrow race if daemons were long-lived, and they are not.
/// `cairnd`'s `supervise` exits a daemon within one two-second tick of another
/// owning its socket, and any command starts one automatically (FR-046). So the
/// process that watched S2 appear is routinely gone by the time an operator asks
/// what happened, and the survivor — having observed nothing — reported that
/// nothing was wrong while the entire backlog was queued for a deployment that
/// no longer answers.
///
/// # Why each step here is deterministic rather than lucky
///
/// The sync worker's loop *begins* with `sleep(WORKER_TICK)`, so a daemon born
/// to serve one request has provably not touched the network when it serves it.
/// Reading the status as the **first command after `stop_daemon`** therefore
/// puts a daemon with no inherited observation in front of the question, every
/// time, with a 500 ms margin rather than a hope.
///
/// The stale-observation step is deterministic from the other side: with the
/// endpoint down, nothing can *change* the observation this process already
/// holds, so `last_observed_instance` is pinned at S2 while the truthful answer
/// is that the server cannot be reached.
///
/// # Falsified by
///
/// - restoring the fallback, `observed.unwrap_or(bound)` — step 3 goes green on
///   `other_instance = 0` and no reason;
/// - deciding the reason from the cached observation instead of a fresh sample —
///   step 2 reports `server_instance_mismatch` for a server that is simply down;
/// - letting the probe establish or adopt the peer — step 3's binding, team-lane
///   and `sync_cursor` assertions fail;
/// - letting the probe enter the claim path — step 3's attempt and row-state
///   assertions fail.
#[test]
fn the_spool_report_names_the_answering_server_across_a_daemon_replacement() {
    let Some(mut server) = server() else { return };
    let d = device(&server, "lifetime");

    let s1: Uuid = server
        .text("SELECT id::text FROM server_instance")
        .parse()
        .expect("the server has an instance id");
    let s2 = Uuid::now_v7();

    // -----------------------------------------------------------------------
    // 0. A backlog bound to S1, with the server away so it stays one.
    // -----------------------------------------------------------------------
    let key = format!("lifetime-{}", Uuid::now_v7());
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
            "tool_input": { "file_path": "src/lifetime.rs" }
        }),
    );
    assert_eq!(out.code, 0, "a hook must not fail during an outage");
    let queued = d.sandbox.json(&[
        "memory",
        "add",
        "the report must outlive the daemon that saw it",
    ]);
    assert_eq!(
        queued["accepted_for_delivery"],
        json!(true),
        "the command was not queued: {queued}"
    );

    // **Waited for, not read.** Capture is fire-and-forget: the hook returns
    // as soon as the daemon has accepted the payload, and the canonical event
    // it produces lands a moment later. Snapshotting the spool immediately
    // caught it mid-fill — 1 run in 182 held only the command row, and the
    // vacuity guard below correctly refused to proceed. Waiting for the event
    // is not a weakening of anything: the rest of this test is about what
    // happens to queued work, and this is where the work becomes queued.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline
        && d.count(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE state IN ('pending','in_flight','failed')",
        ) == 0
    {
        std::thread::sleep(Duration::from_millis(100));
    }

    let event_ids_before = d.column(
        "SELECT event_id FROM event_spool
          WHERE state IN ('pending','in_flight','failed') ORDER BY event_id",
    );
    let rows_before = queued_rows(&d);
    assert!(
        !event_ids_before.is_empty(),
        "no capture event reached the spool within the window, so the \
         cross-instance assertions would have nothing to measure: {rows_before:?}"
    );
    assert!(
        rows_before.len() >= 2,
        "nothing was queued, so every assertion below would pass vacuously: \
         {rows_before:?}"
    );
    let lanes_before = team_lanes(&d);
    assert_eq!(
        lanes_before,
        vec![format!("team:{s1}")],
        "the store is not bound to S1 alone, so the rest measures nothing: \
         {lanes_before:?}"
    );

    // -----------------------------------------------------------------------
    // 1. A different deployment answers at the same address, and this daemon
    //    sees it. Nothing else changes: same database, account, token, project,
    //    session, rows and port.
    // -----------------------------------------------------------------------
    server.execute(&format!("UPDATE server_instance SET id = '{s2}'"));
    server.come_back();
    let _ = d.sandbox.cairn(&["sync", "now"]);

    // -----------------------------------------------------------------------
    // 2. **A stale observation is not the current peer** (FR-792c). The
    //    endpoint goes down while this daemon still remembers S2 — and with it
    //    down, nothing can revise that memory, so it is pinned. The honest
    //    report is that the server cannot be reached, not that a deployment
    //    nobody can reach is the wrong one.
    // -----------------------------------------------------------------------
    server.go_offline();
    let stale = d.spool("events");
    assert_eq!(
        stale["blocked_reason"].as_str(),
        Some("server_unreachable"),
        "an endpoint that cannot be reached was reported from a remembered \
         instance instead (FR-792c): {stale}\n  s1={s1} s2={s2}\n  \
         daemon observation trace:\n{}",
        observation_trace(&d),
    );
    assert_eq!(
        stale["other_instance"].as_i64(),
        Some(0),
        "nothing is answering, so no row can belong to a different deployment \
         than the one that is (FR-792c): {stale}"
    );
    assert_eq!(
        team_lanes(&d),
        lanes_before,
        "the binding moved while the server was unreachable"
    );

    // -----------------------------------------------------------------------
    // 3. **The daemon that saw S2 is replaced, and the report is unchanged.**
    //    The status read below is the first command after the stop, so the
    //    daemon serving it was born to serve it and has observed nothing.
    // -----------------------------------------------------------------------
    server.come_back();
    // **Prove the endpoint is answering before asserting what answering means.**
    // `come_back` returns once the address is bound, which is not the same as
    // the server having served anything — and the assertion below is precisely
    // about what a probe finds there. One round trip through the daemon settles
    // it. Harmless to the scenario: S2 is not this store's binding, so no
    // S1-bound row can drain, and the daemon that makes this call is stopped
    // two statements later, so nothing it learns survives into the read.
    settle_syncing(&d, "S2 is answering again", || {
        d.sandbox.cairn(&["sync", "now"]).code == 0
    });
    let lanes_and_cursors_before = sync_lanes(&d);
    let rows_at_mismatch = queued_rows(&d);
    d.sandbox.stop_daemon();

    let fresh = d.spool("events");
    assert!(
        fresh["other_instance"].as_i64().unwrap_or(0) > 0,
        "a daemon that never saw S2 reported no mismatched work, so the backlog \
         reads as a queue that mysteriously stopped — which is the outcome \
         FR-792 exists to prevent: {fresh}\n  s1={s1} s2={s2}\n  rows: {:?}\n  \
         daemon observation trace:\n{}",
        queued_rows(&d),
        observation_trace(&d),
    );
    assert_eq!(
        fresh["blocked_reason"].as_str(),
        Some("server_instance_mismatch"),
        "the reason delivery is not progressing is not reported by a daemon \
         that did not personally witness the replacement (FR-792a): {fresh}"
    );
    assert!(
        fresh["undelivered"].as_i64().unwrap_or(0) > 0,
        "the held work is gone, so the mismatch report is about nothing: {fresh}"
    );

    // The probe is a read. Everything durable it could have touched is checked
    // rather than assumed, because "read-only" is the property the rest of this
    // rests on: an adopted S2 would deliver S1's work to the wrong deployment.
    assert_eq!(
        team_lanes(&d),
        lanes_before,
        "the status probe established or moved the durable server binding \
         (FR-792b); S2 must never become the lane merely by answering"
    );
    assert_eq!(
        sync_lanes(&d),
        lanes_and_cursors_before,
        "the status probe added a lane or moved a cursor (FR-792b)"
    );
    assert_eq!(
        queued_rows(&d),
        rows_at_mismatch,
        "the status probe claimed a row, spent an attempt or changed a state \
         (FR-792b); a report is not entitled to touch the queue it describes"
    );
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM event_spool WHERE state = 'refused'"),
        0,
        "a status read refused work it was never entitled to judge"
    );

    // -----------------------------------------------------------------------
    // 4. **An unreachable endpoint is unreachable, on a daemon with no
    //    history.** The removed per-process reachability latch reported this
    //    case as healthy for exactly as long as the new daemon took to fail its
    //    first drain — which is a window an operator lands in, because the
    //    command they run is what starts the daemon.
    //
    //    Ordered before the restore deliberately. Nothing can drain here, so
    //    the backlog is still there to be reported on; once S1 answers again it
    //    is delivered within a tick, and an assertion about an empty spool
    //    would hold for the uninteresting reason.
    // -----------------------------------------------------------------------
    server.go_offline();
    // Taken with the endpoint already down and the daemon already stopped, so
    // between this and the read below nothing can synchronize: any change to
    // `sync_cursor` at all was made by the read itself. This is the whole-table
    // assertion `sync_lanes` cannot safely make while the server is answering.
    d.sandbox.stop_daemon();
    let sync_state_before_read = sync_state_strict(&d);

    let down = d.spool("events");
    assert!(
        down["undelivered"].as_i64().unwrap_or(0) > 0,
        "nothing is queued, so there is no reason to report either way: {down}"
    );
    assert_eq!(
        down["blocked_reason"].as_str(),
        Some("server_unreachable"),
        "a daemon that has never reached the server reported an outage as \
         healthy (FR-792a): {down}"
    );
    assert_eq!(
        down["other_instance"].as_i64(),
        Some(0),
        "nothing is answering, so nothing can be mismatched: {down}"
    );
    assert_eq!(
        sync_state_strict(&d),
        sync_state_before_read,
        "the status read wrote to synchronization state (FR-792b); with the \
         endpoint down nothing else could have"
    );
    assert_eq!(
        team_lanes(&d),
        lanes_before,
        "the binding changed during an outage"
    );

    // -----------------------------------------------------------------------
    // 5. **The mismatch clears from the current peer, and the very same rows
    //    deliver.** S1 comes back while no daemon exists, so the daemon that
    //    answers has observed nothing at all — no S2 in memory, no outage in
    //    memory — and the only thing that can tell it the mismatch is over is
    //    its own sample.
    //
    //    Delivery is the assertion, because delivery is unambiguous: these are
    //    the rows queued for S1 before any of this began, and they reach S1.
    //    A report that had kept reporting a mismatch, or an implementation that
    //    had adopted S2 along the way, could not produce this.
    // -----------------------------------------------------------------------
    d.sandbox.stop_daemon();
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
    for id in &event_ids_before {
        assert!(
            d.column("SELECT event_id FROM event_spool WHERE state = 'delivered'")
                .contains(id),
            "event {id} was queued for S1 before any of this and did not reach \
             S1 afterwards; three daemon replacements, two outages and a \
             replacement deployment must cost the queue nothing. Spool now: {:?}",
            queued_rows(&d),
        );
    }
    assert_eq!(
        d.count("SELECT CAST(COUNT(*) AS TEXT) FROM event_spool WHERE state = 'refused'"),
        0,
        "work was refused after the right server came back"
    );
    assert_eq!(
        team_lanes(&d),
        lanes_before,
        "S2 was adopted somewhere along the way; the binding must still be S1 \
         alone (FR-495, FR-496, FR-792b)"
    );

    let drained = d.spool("events");
    assert!(
        drained["blocked_reason"].is_null(),
        "delivery is progressing again and the report still says it is \
         blocked: {drained}"
    );
    assert_eq!(
        drained["other_instance"].as_i64(),
        Some(0),
        "the work reached the deployment it was queued for and is still \
         counted against another: {drained}"
    );
}

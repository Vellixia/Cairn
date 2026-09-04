//! User Story 4, end to end: the server goes away and the agent keeps working
//! (T093, SC-715, SC-716, SC-717; FR-781, FR-786, FR-787, FR-788, FR-789,
//! FR-792).
//!
//! # What "the server is away" means here
//!
//! The daemon's configured endpoint stops answering, with the credential and
//! the learned account identity untouched. That is written directly into
//! `config.json` with the daemon stopped, and the reason it is not done with
//! `cairn auth token set --server` is load-bearing: changing the endpoint
//! through that command clears `server_account_id` (FR-591, and rightly — a
//! different endpoint may be a different account), and a daemon with no
//! account **spools nothing at all**. `spool_capture` declines every event and
//! counts it, because a row spooled with no account could never be claimed by
//! anyone. Taking the server away that way would model a machine that is
//! signed out, which is a different story with a different correct answer, and
//! every assertion below about a backlog would pass vacuously against an empty
//! spool.
//!
//! Port 1 is reserved and nothing listens on it, so a request fails
//! immediately with a connection refusal. That is the same endpoint
//! `identity_partition.rs` already uses for an unreachable server.
//!
//! # Why the drain is waited on rather than triggered
//!
//! `cairn sync now` drains the Feature 004 outbox and the global lanes; it does
//! **not** drain either Feature 005 spool. Those are drained by the sync
//! worker's own tick, which is the point — FR-783 says work queued during an
//! outage is delivered when the server comes back "with no manual step", and a
//! test that pushed a button would be proving the button works.
//!
//! One consequence worth stating, because it is what makes recovery here fast
//! rather than a five-minute backoff climb: while the endpoint is refusing,
//! `AuthenticatedContext::acquire` fails on `GET /api/version` *before*
//! `claim_events` is reached, so an outage spends no attempt budget and moves
//! no row's `next_attempt_at`. A spooled row comes out of an outage exactly as
//! due as it went in.

use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Long enough for a worker tick, a session push and a spool drain to happen in
/// sequence, on a machine running the rest of this suite at the same time.
const SETTLE: Duration = Duration::from_secs(60);

/// Nothing listens here, and nothing ever will.
const NOWHERE: &str = "http://127.0.0.1:1";

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
    base: String,
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
    Device {
        sandbox,
        project,
        base: server.base.clone(),
    }
}

/// Point the daemon's endpoint somewhere, restarting it so it reads the change.
///
/// The config is read-modify-written rather than replaced: `Sandbox::set_deadlines`
/// overwrites the whole file, and doing that here would silently drop the token
/// endpoint, the account identity and every other setting the daemon needs to
/// still be the same installation after the outage.
fn point_at(s: &Sandbox, url: &str) {
    s.stop_daemon();
    let path = s.cairn_home().join("config.json");
    let text = std::fs::read_to_string(&path).expect("the sandbox has a config");
    let mut config: Value = serde_json::from_str(&text).expect("config.json parses");
    let before = config["server_account_id"].clone();
    config["server_url"] = json!(url);
    std::fs::write(&path, config.to_string()).expect("write config");
    // The next command starts the daemon again (FR-046), reading the endpoint
    // it now has.
    let _ = s.cairn(&["status"]);
    let after: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("config after the daemon started"),
    )
    .expect("config.json parses");
    assert_eq!(
        after["server_account_id"], before,
        "moving the endpoint changed the account this machine believes it is; \
         an outage is not a credential change, and a store that forgot its \
         account would decline every capture rather than spooling it"
    );
}

fn take_the_server_away(s: &Sandbox) {
    point_at(s, NOWHERE);
}

fn bring_the_server_back(d: &Device) {
    point_at(&d.sandbox, &d.base.clone());
}

/// Wait for a condition, naming what never happened.
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

/// Poll the health report until it says what is being asserted, and hand back
/// the payload that said it.
///
/// Returning the payload matters: every assertion downstream then reads one
/// consistent snapshot rather than re-running `cairn status` and getting a
/// different moment for each field.
fn settle_health(what: &str, s: &Sandbox, mut predicate: impl FnMut(&Value) -> bool) -> Value {
    let deadline = Instant::now() + SETTLE;
    let mut last = Value::Null;
    while Instant::now() < deadline {
        last = events_health(s);
        if predicate(&last) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}; last report was {last}");
}

/// `COUNT(*)` from the sandbox's own store.
///
/// Cast to TEXT because `query_column` decodes TEXT and a bare `COUNT(*)`
/// comes back as an integer it cannot read.
fn local_count(s: &Sandbox, sql: &str) -> i64 {
    s.query_column(sql)
        .first()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn undelivered_events(s: &Sandbox) -> i64 {
    local_count(
        s,
        "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
          WHERE state IN ('pending','in_flight','failed')",
    )
}

fn events_health(s: &Sandbox) -> Value {
    s.json(&["status"])["capture"]["events"].clone()
}

/// The vendor traffic one working session produces.
///
/// Deliberately a mix: two tool calls that leave `file_read`/`command_executed`
/// events, and a `Stop` that carries an assistant message. An outage that only
/// dropped one shape would still pass a single-event test.
fn work(key: &str) -> Vec<(&'static str, Value)> {
    vec![
        (
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "Bash",
                "tool_input": { "command": "cargo test -p widget" },
                "tool_response": { "exit_code": 1 },
            }),
        ),
        (
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "Edit",
                "tool_input": { "file_path": "src/widget/parser.rs" },
                "tool_response": { "exit_code": 0 },
            }),
        ),
        (
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "Bash",
                "tool_input": { "command": "cargo test -p widget" },
                "tool_response": { "exit_code": 0 },
            }),
        ),
        (
            "Stop",
            json!({
                "session_id": key,
                "last_assistant_message": "we should use parser for widget from now on",
            }),
        ),
    ]
}

/// Open a session against a reachable server and wait until the server holds
/// it.
///
/// Every test below that cares about delivery does this first, because
/// `POST /api/events/batch` refuses a batch whose session the server cannot
/// resolve — with a request-level 403, so one unknown session fails the whole
/// batch. That refusal is transient and the spool retries through it, but a
/// test that started its session inside the outage would be measuring how fast
/// the session catches up rather than whether the events survived.
fn open_session_online(d: &Device, server: &Server, key: &str) -> Uuid {
    let out = d.sandbox.hook_as(
        "claude-code",
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    );
    assert_eq!(out.code, 0, "SessionStart exited non-zero: {}", out.stderr);
    let project = d.project;
    settle("the session reaches the server", || {
        server.count(&format!(
            "SELECT COUNT(*) FROM sessions WHERE project_id = '{project}'"
        )) > 0
    });
    server
        .query_column(&format!(
            "SELECT id::text FROM sessions WHERE project_id = '{project}'
              ORDER BY started_at DESC LIMIT 1"
        ))
        .first()
        .and_then(|s| s.parse().ok())
        .expect("a synced session")
}

// ---------------------------------------------------------------------------
// 1 and 2 — the agent is never blocked, and nothing it produced is lost
// ---------------------------------------------------------------------------

/// Every hook exits zero with the server gone, and every event it produced is
/// in the spool afterwards.
///
/// SC-715 and FR-781. The two halves belong in one test because either alone
/// is satisfiable by the wrong implementation: a daemon that swallowed capture
/// entirely would pass "the hook exits zero", and a daemon that blocked the
/// agent until the write landed would pass "nothing is lost".
///
/// **On the timing bound.** The endpoint refuses connections instantly, so a
/// daemon that *did* wait on the server would still answer quickly, and no
/// wall-clock number here can tell blocking from not blocking. The bound below
/// is therefore a liveness check — it fails a hook that hangs, not one that is
/// merely slow — and it is deliberately far above the 250 ms and 1500 ms hook
/// deadlines so a loaded machine cannot make it flake. The actual latency
/// budgets are asserted in `perf_capture.rs`, against a clock this test has no
/// business duplicating.
///
/// **Falsified by** making capture's spool write conditional on the server
/// being reachable, or by letting a capture failure reach the hook's exit code.
#[test]
fn hook_traffic_during_an_outage_never_fails_the_agent_and_nothing_is_lost() {
    let Some(server) = server() else { return };
    let d = device(&server, "outage-not-blocked");
    let key = format!("outage-{}", Uuid::now_v7());
    open_session_online(&d, &server, &key);

    take_the_server_away(&d.sandbox);
    let before = undelivered_events(&d.sandbox);

    // A liveness bound, not a latency budget. See the note above.
    const RESPONSIVE: Duration = Duration::from_secs(20);
    let mut events: Vec<(&'static str, Value)> = vec![(
        "SessionStart",
        json!({ "session_id": key, "source": "startup" }),
    )];
    events.extend(work(&key));
    for (event, payload) in &events {
        let started = Instant::now();
        let out = d.sandbox.hook_as("claude-code", event, payload.clone());
        let elapsed = started.elapsed();
        assert_eq!(
            out.code, 0,
            "{event} exited {} with the server away. Cairn is never the reason \
             a session breaks (FR-193, FR-781): {}",
            out.code, out.stderr
        );
        assert!(
            elapsed < RESPONSIVE,
            "{event} took {elapsed:?} with the server away, which is a hook \
             waiting on something rather than degrading"
        );
    }

    // An explicit command answers too. It is not fire-and-forget like capture,
    // so "the agent is not blocked" has to hold for the synchronous path as
    // well as the asynchronous one.
    let started = Instant::now();
    let added = d.sandbox.json(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "project",
        "the parser owns rejection",
    ]);
    assert!(
        started.elapsed() < RESPONSIVE,
        "an explicit memory took {:?} with the server away",
        started.elapsed()
    );
    assert!(
        !added.is_null(),
        "an explicit memory produced no answer during an outage"
    );

    // Spooled, not vanished. Counted against the depth before the outage
    // traffic, so a spool that was already holding something does not make this
    // pass on its own.
    let after = undelivered_events(&d.sandbox);
    assert!(
        after > before,
        "the agent did {} hook events' worth of work during an outage and the \
         spool went from {before} rows to {after}: the events did not survive",
        events.len()
    );

    // …and the health report agrees, with an age. A depth on its own does not
    // say whether anything is wrong; the age of the oldest entry is the number
    // a person acts on (FR-792).
    let health = events_health(&d.sandbox);
    assert_eq!(
        health["undelivered"].as_i64(),
        Some(after),
        "`cairn status` and the spool table disagree about the backlog: {health}"
    );
    assert!(
        health["oldest_at"].as_str().is_some(),
        "a backlog with {after} undelivered rows reported no oldest entry: {health}"
    );
}

// ---------------------------------------------------------------------------
// 6 — the backlog is visible and truthful, and stops being visible when it goes
// ---------------------------------------------------------------------------

/// While the server is away the report says how deep the backlog is, how old
/// it is, and why it is not moving; when the backlog drains, all three go quiet.
///
/// FR-792. The recovery half is the part that is easy to get wrong: a
/// `blocked_reason` that is written when delivery fails and never cleared when
/// it succeeds turns into a permanent alarm, and an operator who has learned to
/// ignore it is worse off than one who was never told.
///
/// `oldest_at` is asserted to *move backwards into silence* rather than merely
/// to exist: it is derived from the undelivered rows, so an implementation that
/// counted terminal rows in it would keep reporting an ever-growing age for a
/// spool that is empty.
///
/// **Falsified by** deriving `blocked_reason` from a latch, by reporting it
/// from anything other than the current state of the spool and the connection,
/// or by including delivered or refused rows in `oldest_at`.
#[test]
fn the_backlog_says_how_deep_how_old_and_why_and_goes_quiet_when_it_drains() {
    let Some(server) = server() else { return };
    let d = device(&server, "outage-backlog");
    let key = format!("backlog-{}", Uuid::now_v7());
    open_session_online(&d, &server, &key);

    take_the_server_away(&d.sandbox);
    for (event, payload) in work(&key) {
        assert_eq!(d.sandbox.hook_as("claude-code", event, payload).code, 0);
    }
    settle("the outage traffic reaches the spool", || {
        undelivered_events(&d.sandbox) > 0
    });

    // Waited for, not read once. The spool row exists the moment the hook
    // writes it, and the worker ticks every 500 ms — so between those two
    // instants there is a backlog the daemon has genuinely not tried to deliver
    // yet, and it has nothing truthful to say about why. Reporting a reason
    // there would be a guess, so waiting for one is not a weakening of the
    // assertion; reading the field once, immediately, would be a race.
    let health = settle_health("the backlog says why it is not moving", &d.sandbox, |h| {
        h["blocked_reason"].as_str().is_some()
    });
    assert!(
        health["undelivered"].as_i64().unwrap_or(0) > 0,
        "nothing to report on: {health}"
    );
    assert!(
        health["oldest_at"].as_str().is_some(),
        "the backlog has no age: {health}"
    );

    // The vocabulary is closed. A free-text reason would be a sentence somebody
    // has to read; these are things a caller can branch on, and one of them has
    // to be true while the server is unreachable.
    //
    // `server_unreachable` is the one this test expects to see, and it is not
    // derivable from the rows: a drain that cannot reach the server fails while
    // acquiring its authenticated context, *before* `claim_events`, so every
    // row stays `waiting` with no attempt spent and no `last_error_kind`. The
    // reason has to come from the daemon's memory of the last reachability
    // outcome, which is why the field exists at all rather than being computed
    // from the spool.
    const REASONS: [&str; 7] = [
        "no_account",
        "server_unreachable",
        "saturated",
        "retry_exhausted",
        "refused_by_server",
        "awaiting_capability",
        "backing_off",
    ];
    let reason = health["blocked_reason"].as_str().expect("a reason");
    assert!(
        REASONS.contains(&reason),
        "`{reason}` is not one of the reasons this field is allowed to carry \
         ({REASONS:?}); a reason a caller cannot branch on is prose"
    );
    assert_ne!(
        reason, "no_account",
        "the machine is still signed in — an outage reported as a missing \
         account would send someone to fix a credential that is fine"
    );

    bring_the_server_back(&d);
    settle("the backlog drains once the server is back", || {
        undelivered_events(&d.sandbox) == 0
    });

    let recovered = events_health(&d.sandbox);
    assert_eq!(
        recovered["undelivered"].as_i64(),
        Some(0),
        "the spool is empty and the report is not: {recovered}"
    );
    assert!(
        recovered["blocked_reason"].is_null(),
        "delivery is progressing again and the report still says it is \
         blocked: {recovered}"
    );
    assert!(
        recovered["oldest_at"].is_null(),
        "there is no undelivered row left to be the oldest one: {recovered}"
    );
}

// ---------------------------------------------------------------------------
// 3 and 4 — replay is once, and a duplicate answer is a success
// ---------------------------------------------------------------------------

/// Delivered event ids on the server, for one session.
fn server_event_ids(server: &Server, session: Uuid) -> Vec<String> {
    let mut ids = server.query_column(&format!(
        "SELECT event_id::text FROM safe_events WHERE session_id = '{session}' ORDER BY event_id"
    ));
    ids.sort();
    ids
}

/// Local spool rows in a given state, for one session.
fn local_event_ids(s: &Sandbox, states: &str) -> Vec<String> {
    let mut ids = s.query_column(&format!(
        "SELECT event_id FROM event_spool WHERE state IN ({states}) ORDER BY event_id"
    ));
    ids.sort();
    ids
}

/// Everything spooled during an outage reaches the server, once, and a replay
/// of an already-delivered row is a success rather than a failure.
///
/// SC-716 and FR-786, and the two halves are the same mechanism seen from two
/// sides. `event_id` is UUIDv5 over the session and its durable ordinal and is
/// assigned once, when the row is spooled — so however many times a row is
/// sent, the server derives the same identity, stores at most one canonical
/// event, and answers `duplicate`. A client that treated that answer as a
/// failure would retry forever against a server that had already accepted the
/// event; a client that recomputed the identity on each attempt would store a
/// second copy.
///
/// The replay is forced rather than waited for. A row is put back to `pending`
/// with its backoff expired, which is exactly the state a drainer that died
/// mid-send leaves behind, and is the only way to observe the second delivery
/// without waiting out a real crash.
///
/// **Falsified by** deriving `event_id` at send time, by treating `duplicate`
/// as anything but `delivered`, or by the server storing a row per attempt.
#[test]
fn everything_spooled_during_an_outage_reaches_the_server_exactly_once() {
    let Some(server) = server() else { return };
    let d = device(&server, "outage-replay");
    let key = format!("replay-{}", Uuid::now_v7());
    let session = open_session_online(&d, &server, &key);

    take_the_server_away(&d.sandbox);
    for (event, payload) in work(&key) {
        assert_eq!(d.sandbox.hook_as("claude-code", event, payload).code, 0);
    }
    settle("the outage traffic reaches the spool", || {
        undelivered_events(&d.sandbox) > 0
    });
    // Every id the spool is holding, captured while the server cannot possibly
    // have seen any of them.
    let spooled = local_event_ids(&d.sandbox, "'pending','in_flight','failed'");
    assert!(
        !spooled.is_empty(),
        "nothing was spooled, so nothing can be proved about its delivery"
    );

    bring_the_server_back(&d);
    settle("the spool drains", || undelivered_events(&d.sandbox) == 0);

    // No row ended anywhere but `delivered`. A refusal here would be silent
    // data loss dressed as a settled queue.
    assert_eq!(
        local_event_ids(&d.sandbox, "'refused'"),
        Vec::<String>::new(),
        "recovery refused rows the server had never even been offered before"
    );

    // One canonical event per local event, and no more. `safe_events.event_id`
    // is the primary key, so a duplicate would be a second row under a second
    // id — which is exactly what a client-side re-derivation produces.
    let landed = server_event_ids(&server, session);
    for id in &spooled {
        assert!(
            landed.contains(id),
            "event {id} was spooled during the outage and never reached the \
             server: the server holds {landed:?}"
        );
    }
    let before_replay = landed.len();
    assert_eq!(
        before_replay,
        server.count(&format!(
            "SELECT COUNT(DISTINCT event_id) FROM safe_events WHERE session_id = '{session}'"
        )) as usize,
        "the same event landed twice"
    );

    // Now make the daemon send them all again, exactly as a drainer that died
    // between sending and settling would have left them.
    d.sandbox.stop_daemon();
    let ids = spooled
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(",");
    d.sandbox.execute_sql(&format!(
        "UPDATE event_spool
            SET state = 'pending', attempts = 0, claimed_at = NULL,
                last_error_kind = NULL,
                next_attempt_at = '2020-01-01T00:00:00+00:00'
          WHERE event_id IN ({ids})"
    ));
    let _ = d.sandbox.cairn(&["status"]);
    settle("the replay settles", || undelivered_events(&d.sandbox) == 0);

    // Every replayed row is `delivered`. The server answered `duplicate`, and a
    // `duplicate` is what the retry was *for*: at most one canonical event
    // exists, which is the success this whole identity arrangement buys.
    let delivered = local_event_ids(&d.sandbox, "'delivered'");
    for id in &spooled {
        assert!(
            delivered.contains(id),
            "replayed event {id} did not end `delivered`; a `duplicate` answer \
             is a success and treating it otherwise retries an event the \
             server already holds, forever"
        );
    }
    assert_eq!(
        local_event_ids(&d.sandbox, "'refused','failed'"),
        Vec::<String>::new(),
        "a replay was recorded as a failure"
    );
    assert_eq!(
        server_event_ids(&server, session).len(),
        before_replay,
        "sending every event a second time produced more canonical events"
    );
}

// ---------------------------------------------------------------------------
// 5 — no local durable knowledge is created during an outage
// ---------------------------------------------------------------------------

/// Put this store past the cutover.
///
/// Written straight into `authority_mode` because the command that performs the
/// cutover is US7's and does not exist yet. This is the state that command will
/// produce, not a shortcut around a check: `authority::mode` reads this one row
/// and `memory_create` branches on it, so a store with this row set is
/// indistinguishable from one that got here properly.
fn cut_over(s: &Sandbox) {
    s.stop_daemon();
    s.execute_sql("UPDATE authority_mode SET mode = 'server_authoritative' WHERE id = 1");
    let _ = s.cairn(&["status"]);
    assert_eq!(
        s.query_column("SELECT mode FROM authority_mode WHERE id = 1")
            .first()
            .map(String::as_str),
        Some("server_authoritative"),
        "the store did not reach the authority mode this test is about"
    );
}

/// An explicit memory written during an outage is queued, and creates no local
/// durable record.
///
/// FR-787, and it is the sharpest requirement in this story. Once the server
/// owns durable knowledge, a local row written while the server cannot see it
/// is a **canonical fork**: two stores now disagree about what the project
/// knows, and nothing in the system is responsible for noticing. The queued
/// command is the whole answer — it is intent waiting to be applied, not a
/// record that already is.
///
/// So the assertion is not "the reply is polite about it". It is that
/// `memories` did not grow. A store that queued the command *and* wrote the row
/// would pass every reply-shaped assertion and still be forked.
///
/// **Falsified by** letting `memory_create` fall through to the local write
/// after queueing, or by treating an unreachable server as a reason to write
/// locally "just this once".
#[test]
fn an_explicit_memory_during_an_outage_is_queued_and_writes_nothing_durable_locally() {
    let Some(server) = server() else { return };
    let d = device(&server, "outage-no-fork");
    let key = format!("fork-{}", Uuid::now_v7());
    open_session_online(&d, &server, &key);
    cut_over(&d.sandbox);
    take_the_server_away(&d.sandbox);

    let memories_before = local_count(
        &d.sandbox,
        "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE deleted_at IS NULL",
    );
    let commands_before = local_count(
        &d.sandbox,
        "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool",
    );

    let reply = d.sandbox.json(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "project",
        "the ingest boundary owns rejection, not the parser",
    ]);

    assert_eq!(
        reply["accepted_for_delivery"].as_bool(),
        Some(true),
        "a post-cutover write during an outage has to say it was accepted for \
         delivery, which is a promise about the queue and not about durability: \
         {reply}"
    );
    assert!(
        reply.get("stored").is_none(),
        "the reply claims the knowledge is `stored`. A queued command is not a \
         durable record, and FR-709 and FR-787 exist because saying otherwise \
         is how a user comes to rely on something that never landed: {reply}"
    );
    assert!(
        reply.get("memory").is_none(),
        "the reply carries a memory record, which is a local durable row by \
         another name: {reply}"
    );
    assert!(
        reply["command_id"].as_str().is_some(),
        "a queued command with no identity cannot be delivered idempotently: \
         {reply}"
    );

    assert_eq!(
        local_count(
            &d.sandbox,
            "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool"
        ),
        commands_before + 1,
        "the explicit write produced no queued command, so the user's intent \
         is nowhere at all"
    );
    assert_eq!(
        local_count(
            &d.sandbox,
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE deleted_at IS NULL",
        ),
        memories_before,
        "a durable memory row appeared locally while the server could not see \
         it. That is a canonical fork (FR-787): the server will never learn \
         this row exists as anything but a command it may yet refuse"
    );
}

/// The queued command is not described to the user as remembered.
///
/// Separate from the test above on purpose. That one asserts the machine-
/// readable contract, and it must not be masked by this, which asserts what a
/// person is actually told. FR-709 and FR-815a are about the claim, and the
/// claim is made in the sentence the CLI prints, not only in the JSON nobody
/// reads.
///
/// **Falsified by** rendering the queued reply through the same
/// "Remembered {id}." line the durable path uses.
#[test]
fn a_queued_command_is_not_reported_to_the_user_as_remembered() {
    let Some(server) = server() else { return };
    let d = device(&server, "outage-wording");
    let key = format!("wording-{}", Uuid::now_v7());
    open_session_online(&d, &server, &key);
    cut_over(&d.sandbox);
    take_the_server_away(&d.sandbox);

    let out = d.sandbox.cairn(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "project",
        "queued, not kept",
    ]);
    assert!(
        out.ok(),
        "memory add failed during an outage: {}",
        out.stderr
    );
    let said = out.stdout.to_lowercase();
    assert!(
        !said.contains("remembered"),
        "the user was told the knowledge was `remembered` while it is a command \
         waiting in a queue the server may still refuse (FR-709, FR-815a): {}",
        out.stdout.trim()
    );
}

// ---------------------------------------------------------------------------
// 7 — degradation is reported, not disguised
// ---------------------------------------------------------------------------

/// A briefing assembled during an outage says it is not fresh, and claims no
/// transmission.
///
/// FR-788 and FR-789. The first half is the one a user sees: a stale briefing
/// an agent believes is current is worse than no briefing, because it cannot
/// be told apart from one.
///
/// The second half is the one only the wire shows. A `trace_id` is the handle a
/// transmission outcome is reported against, and
/// `contracts/retrieval-delivery.md` §3 admits only a *generated* trace into
/// `transmitted`. A cached answer's trace was already resolved on the call it
/// was captured on, so replaying its id here would let this call's outcome land
/// against that older trace — the server would record a transmission of content
/// it never served this time. The answer must therefore carry no trace at all.
///
/// **The outage here is `drop(server)`, not the endpoint swap the other tests
/// use, and the difference matters.** The outage cache lives in the daemon's
/// memory, so the endpoint swap — which stops and restarts the daemon — empties
/// it, and the cached branch would be unreachable. Killing the server process
/// leaves the daemon running with its cache, its credential and its link
/// intact, which is what an outage looks like to a machine that was already
/// working. Nothing is restored afterwards because nothing here needs recovery.
///
/// The session is named explicitly for the same reason: `/api/retrieve` binds
/// its project from a session, and `cairn context` with no `--session` falls
/// back to the active sessions in this worktree — a fallback that finds nothing
/// after a daemon restart, and would quietly test the purely local assembly
/// instead of the delivery path.
///
/// `feature005_briefing_cache.rs` already asserts the user-visible label
/// through the hook's rendered text; what is added here is the machine-readable
/// half, read from the daemon's own reply.
///
/// **Falsified by** presenting a cached answer as fresh, by omitting the
/// unavailability marker when there is nothing cached, or by carrying a
/// `trace_id` on either.
#[test]
fn a_briefing_during_an_outage_says_it_is_not_fresh_and_claims_no_transmission() {
    let Some(server) = server() else { return };
    let d = device(&server, "outage-degraded");
    let key = format!("degraded-{}", Uuid::now_v7());
    let session = open_session_online(&d, &server, &key);

    // Something durable to retrieve, so a fresh answer and a cached one differ.
    server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state,
             origin_session_id, topic_key, value_key, origin_kind)
         VALUES ('{}', '{}', 'fact', 'project', '{}',
                 'the estimator undercounts nested generics', 'active',
                 '{session}', 'fact.estimator', 'undercounts', 'explicit')",
        Uuid::now_v7(),
        d.project,
        d.project
    ));

    // A fresh answer first: it must say it is fresh, or "not fresh" later
    // proves nothing. This is also what fills the cache.
    let id = session.to_string();
    let fresh = d.sandbox.json(&["context", "--session", &id]);
    assert_eq!(
        fresh["served_from_cache"].as_bool(),
        Some(false),
        "a briefing assembled from a reachable server claimed to be cached: {fresh}"
    );
    assert!(
        fresh["trace_id"].as_str().is_some(),
        "a fresh retrieval produced no trace, so the absence of one during the \
         outage below would prove nothing: {fresh}"
    );

    // The server process dies. The daemon keeps its credential, its link and
    // its cache, and simply cannot reach anything.
    drop(server);
    let degraded = d.sandbox.json(&["context", "--session", &id]);

    let cached = degraded["served_from_cache"].as_bool() == Some(true);
    let unavailable = degraded["fresh_knowledge_unavailable"].as_bool() == Some(true);
    assert!(
        cached || unavailable,
        "an outage produced a briefing that says nothing about its own \
         freshness. Reporting nothing is indistinguishable from a project that \
         knows nothing, and the two call for opposite reactions (FR-789): \
         {degraded}"
    );
    assert!(
        !(cached && unavailable),
        "the same answer says both that it came from cache and that no \
         knowledge was available: {degraded}"
    );
    assert!(
        cached,
        "this session's briefing was assembled and cached moments ago and the \
         outage answer is not it. §12.3 permits the cache precisely so an \
         outage degrades to a stale answer rather than to none: {degraded}"
    );
    assert!(
        degraded["trace_id"].is_null(),
        "an outage answer carries a trace id. Only a generated trace can become \
         `transmitted` (§3), and this call generated none — a report against \
         this id would claim a transmission of something the server never \
         served on this call: {degraded}"
    );
}

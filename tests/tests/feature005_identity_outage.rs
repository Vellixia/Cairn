//! Whose work is it, when the machine has held more than one credential
//! (T094, FR-790, FR-790a, FR-791).
//!
//! # The failure this file exists to catch
//!
//! A spooled row outlives the sign-in that produced it. That is the whole point
//! of a spool — an outage may last longer than a session, and longer than the
//! period one person is signed in on a shared workstation. So every row in
//! either spool is a small, durable claim about *who authored it*, and the
//! claim has to survive a credential change intact in both directions: the row
//! must not be delivered under somebody else's credential, and it must not be
//! thrown away when its author signs out.
//!
//! `crates/cairn-store/tests/feature005_spool.rs` proves the claim predicate is
//! an exact `account_id` match, at the level of one SQL statement against a
//! store. What it cannot see is the drain: `drain_event_spool` acquires an
//! authenticated context and passes `context.account` to `claim_events`, and
//! there is a whole credential pipeline between the token on disk and that
//! argument. This file drives the real daemon across a real credential change
//! and asserts the outcome the requirement is actually about.
//!
//! # How a credential change is made here, and why not through the CLI
//!
//! `cairn auth token set` is used wherever the server is reachable, because
//! that is what a person does. It cannot be used to switch *during* an outage:
//! the endpoint change clears `server_account_id` (FR-591) and the identity
//! lookup that would restore it needs the server. A machine in that state has
//! no account at all, so nothing delivers — for the wrong reason, and every
//! assertion below about the identity gate would pass vacuously.
//!
//! So a switch that has to straddle an outage is written into the three files
//! that hold a credential — the token file, `config.json`'s `server_url`, and
//! its `server_account_id` — with the daemon stopped. That is precisely the
//! state a successful `auth token set` leaves behind (see
//! `Daemon::mutate_credentials`), assembled with no window in which the old
//! account is signed in and the server is reachable. Without that atomicity the
//! drain would deliver the rows under their real author between the two steps,
//! and the test would prove nothing.

use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SETTLE: Duration = Duration::from_secs(60);

/// How long a wrongly-credentialed daemon is watched before concluding it is
/// not going to deliver somebody else's rows.
///
/// Not a guess: every test that uses it afterwards measures how long delivery
/// under the *right* account took and asserts this window was comfortably
/// longer. A window shorter than the real delivery time would be a test that
/// passes because it did not wait, which is the standard way an identity gate
/// appears to hold and does not.
const WRONG_ACCOUNT_WINDOW: Duration = Duration::from_secs(15);

/// Nothing listens here.
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
    account: Uuid,
    token: String,
}

fn device(server: &Server, label: &str) -> Device {
    let sandbox = Sandbox::new();
    let remote = format!("git@localhost:cairnfixture/{label}.git");
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);

    let (account, token) = server.new_user(label);
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
        account,
        token,
    }
}

/// A second account on the same server, a member of the same project.
fn colleague(server: &Server, project: Uuid, label: &str) -> (Uuid, String) {
    let (account, token) = server.new_user(label);
    server.execute(&format!(
        "INSERT INTO project_members (project_id, user_id) VALUES ('{project}', '{account}')"
    ));
    (account, token)
}

/// Write one whole credential — endpoint, token and account — with the daemon
/// stopped, then start it again.
///
/// The three files move together or the daemon comes up holding a pairing no
/// sign-in ever produced. Passing `url = NOWHERE` is how an outage is entered
/// or left in the same breath as a credential change.
fn sign_in_as(s: &Sandbox, url: &str, account: Option<Uuid>, token: Option<&str>) {
    s.stop_daemon();
    let home = s.cairn_home();
    match token {
        Some(t) => std::fs::write(home.join("token"), t).expect("write the token file"),
        // Signing out: the token file is removed, which is what
        // `write_token_file(None)` does.
        None => {
            let _ = std::fs::remove_file(home.join("token"));
        }
    }
    let path = home.join("config.json");
    let text = std::fs::read_to_string(&path).expect("the sandbox has a config");
    let mut config: Value = serde_json::from_str(&text).expect("config.json parses");
    config["server_url"] = json!(url);
    config["server_account_id"] = match account {
        Some(a) => json!(a),
        None => Value::Null,
    };
    std::fs::write(&path, config.to_string()).expect("write config");
    // The next command starts the daemon again (FR-046).
    let _ = s.cairn(&["status"]);
}

fn take_the_server_away(d: &Device) {
    sign_in_as(&d.sandbox, NOWHERE, Some(d.account), Some(&d.token));
}

fn settle(what: &str, mut predicate: impl FnMut() -> bool) -> Duration {
    let started = Instant::now();
    let deadline = started + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return started.elapsed();
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

/// Watch for the whole window and fail the moment `forbidden` becomes true.
///
/// Not a sleep followed by one read: a row that was delivered and then somehow
/// settled back would slip past a single check at the end, and the assertion
/// here is that it never happens at all.
fn never_within(window: Duration, what: &str, mut forbidden: impl FnMut() -> bool) {
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        assert!(!forbidden(), "{what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn local(s: &Sandbox, sql: &str) -> Vec<String> {
    s.query_column(sql)
}

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

/// How many of *these* rows have been delivered.
///
/// Scoped to a named set rather than counting the whole table, because the
/// session that carries the outage traffic was opened against a reachable
/// server and its `session_opened` event was delivered then, quite legitimately,
/// under its own author's credential. A test that watched the table's delivered
/// count would fail on that row before the interesting one had a chance to
/// misbehave.
fn delivered_among(s: &Sandbox, ids: &[String]) -> i64 {
    if ids.is_empty() {
        return 0;
    }
    let list = ids
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(",");
    local_count(
        s,
        &format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
              WHERE state = 'delivered' AND event_id IN ({list})"
        ),
    )
}

fn work(key: &str) -> Vec<(&'static str, Value)> {
    vec![
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
            "PostToolUse",
            json!({
                "session_id": key,
                "tool_name": "Edit",
                "tool_input": { "file_path": "src/widget/parser.rs" },
                "tool_response": { "exit_code": 0 },
            }),
        ),
    ]
}

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

/// Spool events for `d`'s account by working through an outage.
fn spool_during_an_outage(d: &Device, key: &str) -> Vec<String> {
    take_the_server_away(d);
    for (event, payload) in work(key) {
        assert_eq!(
            d.sandbox.hook_as("claude-code", event, payload).code,
            0,
            "a hook failed during the outage"
        );
    }
    // Wait for the depth to *stop moving*, not merely to become non-zero.
    //
    // One vendor hook can produce several canonical events, and the hooks are
    // fire-and-forget, so reading the spool the instant it is non-empty catches
    // it mid-fill. Every assertion below compares a later depth against this
    // one, so a snapshot taken too early does not fail the test — it makes it
    // report a backlog that grew as a backlog that was discarded, which is a
    // false accusation of exactly the defect being looked for.
    let mut ids: Vec<String> = Vec::new();
    let mut steady = 0;
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        let now = local(
            &d.sandbox,
            "SELECT event_id FROM event_spool
              WHERE state IN ('pending','in_flight','failed') ORDER BY event_id",
        );
        steady = if !now.is_empty() && now == ids {
            steady + 1
        } else {
            0
        };
        ids = now;
        if steady >= 8 {
            return ids;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    panic!("the outage traffic never settled into a stable spool depth")
}

// ---------------------------------------------------------------------------
// 1 — an event authored by A is never delivered as B, and survives the switch
// ---------------------------------------------------------------------------

/// Events spooled by one account are not delivered under another's credential,
/// and are still there when their author comes back.
///
/// FR-790. This has been a real regression in this repository twice — a claim
/// predicate of the shape `account_id IS NULL OR account_id = ?` is exactly how
/// one identity comes to deliver another's work under its own credential, and
/// the consequence is not a lost row but a *misattributed* one: the server
/// records the events as B's, and B's project sees what A did.
///
/// The second half is the same rows read the other way. A credential change is
/// not a reason to discard queued work — the person signs back in and their
/// machine still owes the server everything it captured. A daemon that cleared
/// the spool on sign-out would pass the first half perfectly.
///
/// **Falsified by** widening the claim predicate to admit a NULL or a missing
/// account, by passing anything but the authenticated account to
/// `claim_events`, or by clearing either spool on a credential change.
#[test]
fn events_spooled_by_one_account_are_never_delivered_under_another() {
    let Some(server) = server() else { return };
    let a = device(&server, "identity-a");
    let (b_account, b_token) = colleague(&server, a.project, "identity-b");
    let key = format!("identity-{}", Uuid::now_v7());
    let session = open_session_online(&a, &server, &key);

    let spooled = spool_during_an_outage(&a, &key);
    let depth = spooled.len() as i64;

    // The server comes back and B is the account holding the machine, in one
    // step: there is no moment in which A is signed in and the server is
    // reachable, so nothing here can deliver for the ordinary reason.
    sign_in_as(&a.sandbox, &a.base, Some(b_account), Some(&b_token));

    never_within(
        WRONG_ACCOUNT_WINDOW,
        "a row authored by one account was delivered under another's credential \
         (FR-790)",
        || delivered_among(&a.sandbox, &spooled) > 0,
    );
    assert_eq!(
        undelivered_events(&a.sandbox),
        depth,
        "the spool changed depth while the wrong account held the machine: a \
         credential change is not a reason to discard queued work, and it is \
         certainly not a reason to deliver it"
    );
    assert_eq!(
        local(
            &a.sandbox,
            "SELECT event_id FROM event_spool WHERE state = 'refused'"
        ),
        Vec::<String>::new(),
        "somebody else signing in refused this account's queued events"
    );
    // Only the outage traffic. The session's own `session_opened` event reached
    // the server before the outage, under its author, which is correct — the
    // question here is about the rows that were still queued when the machine
    // changed hands.
    let outage_ids = spooled
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(",");
    // **Which account owns the row is what makes this an accusation or not.**
    // `safe_events.account_id` is bound from the authenticated caller, so a row
    // stamped A was delivered while A held the machine — early, but by its own
    // author, which is FR-790 working. A row stamped B is the leak. Reporting
    // both, because a bare count cannot tell them apart and a failure that says
    // "delivered under the wrong account" had better be able to prove it.
    let leaked = server.query_column(&format!(
        "SELECT event_id::text || ' owned by ' ||
                CASE WHEN account_id = '{}' THEN 'A (its author)'
                     WHEN account_id = '{b_account}' THEN 'B (NOT its author)'
                     ELSE account_id::text END
           FROM safe_events
          WHERE session_id = '{session}' AND event_id IN ({outage_ids})",
        a.account
    ));
    assert!(
        leaked.is_empty(),
        "events queued during the outage reached the server after the machine \
         changed hands: {leaked:?}"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM safe_events WHERE account_id = '{b_account}'"
        )),
        0,
        "events were attributed to the account that happened to be signed in \
         rather than to the one that produced them"
    );

    // Their author comes back, and only now do they move.
    sign_in_as(&a.sandbox, &a.base, Some(a.account), Some(&a.token));
    let took = settle("the original account's rows deliver", || {
        undelivered_events(&a.sandbox) == 0
    });
    assert!(
        took * 3 < WRONG_ACCOUNT_WINDOW,
        "delivery under the right account took {took:?}, which is not \
         comfortably inside the {WRONG_ACCOUNT_WINDOW:?} this test watched the \
         wrong account for — the window above proved nothing"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM safe_events
              WHERE session_id = '{session}' AND account_id = '{}'
                AND event_id IN ({outage_ids})",
            a.account
        )),
        depth,
        "the rows that survived the credential change did not all arrive, or \
         did not arrive attributed to their author"
    );
}

// ---------------------------------------------------------------------------
// 2 — the same for commands
// ---------------------------------------------------------------------------

/// Put this store past the cutover, so an explicit write becomes a command.
///
/// Written straight into `authority_mode` because the cutover command is US7's
/// and does not exist yet. `authority::mode` reads this one row, so a store
/// with it set is indistinguishable from one that got here properly.
fn cut_over(s: &Sandbox) {
    s.stop_daemon();
    s.execute_sql("UPDATE authority_mode SET mode = 'server_authoritative' WHERE id = 1");
    let _ = s.cairn(&["status"]);
}

fn undelivered_commands(s: &Sandbox) -> i64 {
    local_count(
        s,
        "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool
          WHERE state IN ('pending','in_flight','failed')",
    )
}

fn delivered_commands(s: &Sandbox) -> i64 {
    local_count(
        s,
        "SELECT CAST(COUNT(*) AS TEXT) FROM command_spool WHERE state = 'delivered'",
    )
}

/// A queued command is bound to its author exactly as an event is.
///
/// The stake is higher here than for an event. A command is an instruction —
/// remember this, supersede that — and applying one under the wrong credential
/// does not merely misfile an observation, it writes into a second person's
/// knowledge under their name. `applied_commands` is keyed by
/// `(account_id, command_id)` precisely because two accounts on one machine
/// derive the same command ids, so the server cannot be the thing that catches
/// this: it would apply B's copy quite happily.
///
/// **Falsified by** the same predicate widening as the event case, in
/// `claim_commands`.
#[test]
fn commands_spooled_by_one_account_are_never_delivered_under_another() {
    let Some(server) = server() else { return };
    let a = device(&server, "identity-cmd-a");
    let (b_account, b_token) = colleague(&server, a.project, "identity-cmd-b");
    let key = format!("identity-cmd-{}", Uuid::now_v7());
    open_session_online(&a, &server, &key);
    cut_over(&a.sandbox);

    take_the_server_away(&a);
    let queued = a.sandbox.json(&[
        "memory",
        "add",
        "--type",
        "decision",
        "--scope",
        "project",
        "the ingest boundary owns rejection",
    ]);
    assert_eq!(
        queued["accepted_for_delivery"].as_bool(),
        Some(true),
        "the explicit write was not queued as a command: {queued}"
    );
    let command_id = queued["command_id"]
        .as_str()
        .expect("a queued command has an identity")
        .to_string();
    assert_eq!(undelivered_commands(&a.sandbox), 1);

    sign_in_as(&a.sandbox, &a.base, Some(b_account), Some(&b_token));
    never_within(
        WRONG_ACCOUNT_WINDOW,
        "a queued command was delivered under an account that did not issue it \
         (FR-790)",
        || delivered_commands(&a.sandbox) > 0,
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM applied_commands WHERE command_id = '{command_id}'"
        )),
        0,
        "the command was applied on the server while its author was not the \
         account signed in"
    );

    sign_in_as(&a.sandbox, &a.base, Some(a.account), Some(&a.token));
    let took = settle("the author's command delivers", || {
        undelivered_commands(&a.sandbox) == 0
    });
    assert!(
        took * 3 < WRONG_ACCOUNT_WINDOW,
        "delivery under the right account took {took:?}; the window above \
         proved nothing"
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM applied_commands
              WHERE command_id = '{command_id}' AND account_id = '{}'",
            a.account
        )),
        1,
        "the command did not arrive, or arrived credited to somebody else"
    );
}

// ---------------------------------------------------------------------------
// 3 — a row bound to an account nobody holds is never deliverable
// ---------------------------------------------------------------------------

/// A row whose account is not the signed-in one is not deliverable by anybody,
/// and there is no NULL to open the door with.
///
/// Two protections, asserted separately because they fail separately. The
/// schema makes `account_id` NOT NULL, so "a row with no recorded account"
/// cannot be written at all — that is the door being nailed shut. The claim
/// predicate then matches exactly, so an *unknown* account is not a wildcard
/// either — a row bound to an account this machine has never held stays where
/// it is rather than being adopted by whoever is signed in.
///
/// The second is the one worth testing at this level, because it is the shape
/// a widened predicate produces: `account_id IS NULL OR account_id = ?` fails
/// closed against the schema and wide open against a row carrying a nil UUID,
/// which is what a caller with nothing to say for itself would write.
///
/// **Falsified by** dropping the NOT NULL, or by any claim predicate that
/// treats an unrecognised account as claimable.
#[test]
fn a_row_bound_to_an_account_nobody_holds_is_never_deliverable() {
    let Some(server) = server() else { return };
    let a = device(&server, "identity-orphan");
    let key = format!("orphan-{}", Uuid::now_v7());
    let session = open_session_online(&a, &server, &key);

    // The door: there is no NULL to write.
    let notnull = a.sandbox.query_column(
        "SELECT CAST(\"notnull\" AS TEXT) FROM pragma_table_info('event_spool')
          WHERE name = 'account_id'",
    );
    assert_eq!(
        notnull.first().map(String::as_str),
        Some("1"),
        "`event_spool.account_id` is nullable. A NULL account is how one \
         identity comes to deliver work nobody claimed (FR-790, FR-864a), and \
         the schema is the first place that is ruled out"
    );

    // The lock: an account this machine has never held. Written directly,
    // because no surface will produce one — which is the point.
    let orphan = Uuid::now_v7();
    let event_id = Uuid::now_v7();
    a.sandbox.stop_daemon();
    a.sandbox.execute_sql(&format!(
        "INSERT INTO event_spool
            (event_id, session_id, project_id, account_id, session_seq, kind, payload,
             payload_bytes, boundary_class, state, attempts, claimed_at, next_attempt_at,
             last_error_kind, created_at)
         SELECT '{event_id}', session_id, project_id, '{orphan}', 9001, kind, payload,
                payload_bytes, boundary_class, 'pending', 0, NULL,
                '2020-01-01T00:00:00+00:00', NULL, '2020-01-01T00:00:00+00:00'
           FROM event_spool LIMIT 1"
    ));
    let planted = local_count(
        &a.sandbox,
        &format!("SELECT CAST(COUNT(*) AS TEXT) FROM event_spool WHERE event_id = '{event_id}'"),
    );
    assert_eq!(
        planted, 1,
        "the fixture could not plant an orphan row, so nothing below is being \
         tested. The session needs at least one spooled event to copy from"
    );
    let _ = a.sandbox.cairn(&["status"]);

    never_within(
        WRONG_ACCOUNT_WINDOW,
        "a row bound to an account this machine has never held was delivered by \
         the account that happens to be signed in",
        || {
            local_count(
                &a.sandbox,
                &format!(
                    "SELECT CAST(COUNT(*) AS TEXT) FROM event_spool
                      WHERE event_id = '{event_id}' AND state <> 'pending'"
                ),
            ) > 0
        },
    );
    assert_eq!(
        server.count(&format!(
            "SELECT COUNT(*) FROM safe_events WHERE event_id = '{event_id}'"
        )),
        0,
        "an orphan row reached the server under somebody else's credential"
    );
    // The signed-in account's own rows did move, so the daemon was draining
    // throughout and the row above was skipped rather than merely missed.
    assert_eq!(
        undelivered_events(&a.sandbox),
        1,
        "the drain was not running during the window above, so the orphan row \
         staying put says nothing. Only the orphan should be left"
    );
    let _ = session;
}

// ---------------------------------------------------------------------------
// 4 — a replacement deployment at the same address is not the same server
// ---------------------------------------------------------------------------

/// A store's queued events are not handed to a different deployment that
/// happens to answer at the same URL.
///
/// FR-791. `identity_partition.rs` already proves the *team corpus* is not
/// inherited across a replacement; what that test cannot see is the spool,
/// which is the half US4 owns. An endpoint is not an identity: a deployment
/// restored from backup, or a different one stood up on an address that used to
/// serve another, both look identical to a client that compares URLs. A store
/// that delivered its backlog to whatever answers would hand one organisation's
/// captured work to another's server, and the events carry session and project
/// ids that mean nothing there.
///
/// The user signs in again, because they have to: every token the predecessor
/// issued died with it, and without a working credential the store simply fails
/// to authenticate and the isolation being asserted is never reached.
///
/// The requirement is about the outcome rather than about which mechanism
/// produces it — instance binding, session binding on the far side, or both.
/// So this asserts what must be true of the rows and of the replacement's
/// tables, and deliberately does not assert *where* the refusal happened.
///
/// **Falsified by** treating the endpoint as the peer's identity anywhere on
/// the spool's delivery path.
#[test]
fn a_replacement_deployment_at_the_same_address_never_receives_the_predecessors_spool() {
    let Some(mut original) = server() else { return };
    let a = device(&original, "identity-replaced");
    let key = format!("replaced-{}", Uuid::now_v7());
    open_session_online(&a, &original, &key);
    let spooled = spool_during_an_outage(&a, &key);
    let depth = spooled.len() as i64;

    // A different deployment, same address, its own database and so its own
    // instance id — and no knowledge whatever of this store's project, session
    // or account.
    let replacement = original.replaced_at_same_address();
    let (new_account, new_token) = replacement.new_user("identity-replaced-again");
    sign_in_as(&a.sandbox, &a.base, Some(new_account), Some(&new_token));

    never_within(
        WRONG_ACCOUNT_WINDOW,
        "queued events were delivered to a deployment that is not the one they \
         were captured against (FR-791)",
        || delivered_among(&a.sandbox, &spooled) > 0,
    );
    assert_eq!(
        replacement.count("SELECT COUNT(*) FROM safe_events"),
        0,
        "a replacement deployment received the predecessor's captured work"
    );
    assert_eq!(
        undelivered_events(&a.sandbox),
        depth,
        "the backlog was discarded rather than held. A server that is not the \
         one this work belongs to is a reason to wait, not a reason to forget"
    );

    // Reported, not silent. The rows are still in the health report, so a
    // person looking at this machine can see it is holding work.
    let health = a.sandbox.json(&["status"])["capture"]["events"].clone();
    assert_eq!(
        health["undelivered"].as_i64(),
        Some(depth),
        "the backlog is invisible in the health report, which is the difference \
         between degrading and disguising: {health}"
    );
}

// ---------------------------------------------------------------------------
// 5 — a cached briefing is bound to the account it was assembled for
// ---------------------------------------------------------------------------

/// A second account gets a cache *miss*, not a filtered version of the first
/// account's briefing.
///
/// FR-790a. `feature005_briefing_cache.rs` already asserts the first account's
/// content does not appear in the second's briefing, read from the rendered
/// text. That is the outcome, and it is satisfiable two ways: the entry is
/// never found, or it is found and then filtered. The difference is not
/// cosmetic — a filtered hit means the entry *was* read under the wrong
/// account, so the isolation is whatever the filter happens to cover, and it
/// would still be labelled to the agent as a cached answer for this session.
///
/// The daemon's machine-readable reply can tell them apart and the rendered
/// text cannot, which is why this is asserted here and there rather than only
/// there: a miss says `fresh_knowledge_unavailable`, a hit says
/// `served_from_cache`.
///
/// **Falsified by** keying the outage cache on the session alone and filtering
/// the account out of the answer, rather than keying on both.
#[test]
fn a_cached_briefing_is_a_miss_for_a_second_account_and_not_a_filtered_hit() {
    let Some(server) = server() else { return };
    let a = device(&server, "identity-cache-a");
    let (_b_account, b_token) = colleague(&server, a.project, "identity-cache-b");
    let key = format!("cache-{}", Uuid::now_v7());
    let session = open_session_online(&a, &server, &key);

    let marker = format!("kestrel-{}", Uuid::now_v7().simple());
    server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state,
             origin_session_id, topic_key, value_key, origin_kind)
         VALUES ('{}', '{}', 'fact', 'project', '{}',
                 'the {marker} estimator undercounts', 'active',
                 '{session}', 'fact.{marker}', 'undercounts', 'explicit')",
        Uuid::now_v7(),
        a.project,
        a.project
    ));

    let id = session.to_string();
    let warm = a.sandbox.json(&["context", "--session", &id]);
    assert_eq!(
        warm["served_from_cache"].as_bool(),
        Some(false),
        "the first answer came from a reachable server and says otherwise: {warm}"
    );
    assert!(
        warm.to_string().contains(&marker),
        "the first account never retrieved the fact its cache should now hold, \
         so a later miss would prove nothing: {warm}"
    );

    // The second account signs in through the ordinary command, while the
    // server is still reachable, so its identity is genuinely learned. The
    // daemon keeps running, so the cache it filled a moment ago is still in
    // memory — which is what makes this a test of the key rather than of a
    // restart.
    let switched = a
        .sandbox
        .cairn(&["auth", "token", "set", &b_token, "--server", &a.base]);
    assert!(switched.ok(), "auth token set: {}", switched.stderr);

    drop(server);
    let after = a.sandbox.json(&["context", "--session", &id]);

    assert!(
        !after.to_string().contains(&marker),
        "the second account was served the first account's cached briefing \
         (FR-790a): {after}"
    );
    assert_ne!(
        after["served_from_cache"].as_bool(),
        Some(true),
        "the second account's answer is labelled as this session's cached \
         briefing. The entry was found under a key that does not include the \
         account, and what kept the content out was a filter rather than the \
         key: {after}"
    );
    assert_eq!(
        after["fresh_knowledge_unavailable"].as_bool(),
        Some(true),
        "an account with nothing cached and no server has to be told durable \
         knowledge is unavailable, rather than quietly handed Level 0 as though \
         that were the whole answer: {after}"
    );
}

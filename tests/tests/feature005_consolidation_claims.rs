//! Consolidation's claim machinery, as a contract (T031,
//! `contracts/consolidation.md` §4, §4.1, §6).
//!
//! What these tests are about is not "does consolidation produce knowledge" —
//! it does not yet, and US1 supplies the extractor. They are about the four
//! properties the claim machinery has to hold before an extractor is worth
//! writing:
//!
//! - **A group is not lockable; a row is.** PostgreSQL refuses a locking clause
//!   with `GROUP BY` (SQLSTATE 0A000), so the group that gets claimed is a
//!   `consolidation_session` row, elected `FOR UPDATE SKIP LOCKED ... LIMIT 1`
//!   and oldest first. One session per pass, and never the same session twice
//!   at once.
//! - **The attempt is counted at claim start, before the work runs.** A worker
//!   that dies mid-pass must have consumed its attempt; counting on failure
//!   instead is how a crash loop retries forever. The local spool had exactly
//!   that defect and it was repaired there (`crates/cairn-store/src/spool.rs`),
//!   so it is asserted here rather than trusted.
//! - **Five attempts means five that actually ran.** Failures 1–4 retry, a
//!   success on the fifth closes the event `done`, an unreported fifth becomes
//!   `failed`, and a sixth never starts.
//! - **Nothing strands a session.** Not a full batch, not a lease that outlived
//!   its worker, and not work that has exhausted its attempts.
//!
//! # Why these tests spawn their own `cairn-server`
//!
//! Consolidation's connection-pool share is `min(2, floor(max_connections / 5))`
//! and it does not run at all below `max_connections = 5` (§6, FR-793a1). The
//! shared harness deliberately starts servers with a pool of four, so the
//! fixture's own server never consolidates — which is a bound worth asserting
//! (see the pool-share test) and useless for exercising the machinery. So each
//! test that needs a worker starts a second `cairn-server` process against the
//! *same* database with a pool big enough to earn a share. That is a real
//! worker, not a mock: the same binary, the same code path, the same rows.
//!
//! # Why state is seeded with SQL rather than posted through ingest
//!
//! Two of the edge cases are defined by what a *crashed* worker left committed:
//! an attempt already counted, a lease already taken, a fifth attempt that never
//! reported. A crash is not observable as anything other than those rows, and
//! there is no wire message that produces them. Seeding the rows is therefore
//! the faithful reproduction, not a shortcut around one. The reopening test does
//! go through `POST /api/events/batch`, because reopening is exactly what the
//! ingest upsert does and a hand-written `UPDATE` would be testing this file.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{binary, post_json_status_bearer};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use uuid::Uuid;

macro_rules! pg {
    () => {
        match Pg::start() {
            Some(pg) => pg,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// How long a property is given to become true before the test calls it false.
///
/// Generous against the 100 ms yield the worker actually uses, and far below
/// the five-minute lease — which is the whole point for the partial-batch test:
/// a session that stalls until its lease expires cannot pass inside this.
const SETTLE: Duration = Duration::from_secs(30);

/// The pool a test worker asks for.
///
/// Five, not the production default of ten, because these tests run in parallel
/// against one PostgreSQL and every extra connection is one another test cannot
/// have. Five is also the exact boundary the bound describes: `floor(5/5) = 1`
/// is the smallest share that runs at all, so a worker started this way proves
/// the rule admits the smallest configuration it is supposed to admit.
const WORKER_POOL: &str = "5";

// ---------------------------------------------------------------------------
// A second server on the fixture's database — the worker under test
// ---------------------------------------------------------------------------

/// A `cairn-server` process started only for its consolidation task.
///
/// It serves HTTP too, on a port nothing calls; that is incidental. What it is
/// here for is the in-process task `main` starts, and the only way to get one
/// is to run the binary.
struct Worker {
    child: Child,
}

impl Worker {
    fn start(database_url: &str) -> Self {
        // A port probed and released can be taken by someone else before the
        // child binds it, and the child then exits instead of serving. The
        // harness retries for the same reason.
        for _ in 0..8 {
            let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
            let port = probe.local_addr().expect("addr").port();
            drop(probe);
            let addr = format!("127.0.0.1:{port}");
            let mut child = Command::new(binary("cairn-server"))
                .args([
                    "--addr",
                    &addr,
                    "--database-url",
                    database_url,
                    "--max-connections",
                    WORKER_POOL,
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("cairn-server runs");

            // Bound means started, and the consolidation task is spawned before
            // the listener binds — so a successful connect is proof the worker
            // is running, not merely that the process exists.
            for _ in 0..250 {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                if std::net::TcpStream::connect(&addr).is_ok() {
                    return Self { child };
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        panic!("a consolidation worker would not start");
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

/// The committed state one session starts a test in.
///
/// Every field is a dial one of the edge cases below turns: how many events are
/// queued, how many passes have already started on each, and whether a lease is
/// live, expired or absent. `claimed_by` and `expires` are SQL fragments rather
/// than values because "no lease" is `NULL` and "an expired one" is an interval
/// arithmetic the database has to do — the test's clock is not the one the
/// election reads.
struct Seed<'a> {
    events: i64,
    attempts: i32,
    state: &'a str,
    claimed_by: &'a str,
    expires: &'a str,
    enqueued: &'a str,
    /// Whether the underlying session has closed.
    ///
    /// Defaults to closed, because these tests are about the *claim*
    /// machinery — leases, attempts, ordering — and a session that is not
    /// eligible is never claimed at all, so an open one would make every test
    /// here a test of eligibility instead. Eligibility has its own tests
    /// below, and they set this deliberately.
    closed: bool,
}

impl Default for Seed<'_> {
    /// The ordinary case: pending, nobody holding it, enqueued now.
    fn default() -> Self {
        Self {
            events: 1,
            attempts: 0,
            state: "pending",
            claimed_by: "NULL",
            expires: "NULL",
            enqueued: "now()",
            closed: true,
        }
    }
}

/// Close a session on the server's own row, which is where the election reads
/// it from.
///
/// A closed session is eligible immediately (`contracts/consolidation.md` §3),
/// and it is the cheapest of the three triggers to establish — the alternatives
/// are two hundred events or a ten-minute wait. Tests that are about claim
/// machinery rather than about eligibility use this so that eligibility is not
/// silently the thing they are measuring.
fn close_session(pg: &Pg, session: Uuid) {
    pg.server.execute(&format!(
        "UPDATE sessions SET status = 'completed', ended_at = now() WHERE id = '{session}'"
    ));
}

fn seed_session(pg: &Pg, who: &Account, session: Uuid, seed: Seed) {
    let Seed {
        events,
        attempts,
        state,
        claimed_by,
        expires,
        enqueued,
        closed,
    } = seed;
    if closed {
        close_session(pg, session);
    }
    pg.server.execute(&format!(
        "INSERT INTO consolidation_session
             (project_id, session_id, state, claimed_by, claim_expires_at, oldest_enqueued_at)
         VALUES ('{}', '{session}', '{state}', {claimed_by}, {expires}, {enqueued})",
        pg.project
    ));
    // `safe_events` first: `consolidation_work.event_id` references it, and the
    // work row is what the claim actually reads.
    pg.server.execute(&format!(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind,
              session_seq, contract_version, content, occurred_at)
         SELECT gen_random_uuid(), '{}', '{session}', '{}', 'claude_code',
                'file_changed', g, 1, '{{}}'::jsonb, now()
           FROM generate_series(1, {events}) g",
        pg.project, who.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO consolidation_work
             (event_id, project_id, session_id, session_seq, state, attempts)
         SELECT event_id, project_id, session_id, session_seq, 'pending', {attempts}
           FROM safe_events WHERE session_id = '{session}'"
    ));
}

/// The ordinary case: a pending session, nobody holding it, enqueued now.
fn seed_pending(pg: &Pg, who: &Account, session: Uuid, events: i64, attempts: i32) {
    seed_session(
        pg,
        who,
        session,
        Seed {
            events,
            attempts,
            ..Seed::default()
        },
    );
}

// ---------------------------------------------------------------------------
// Observation
// ---------------------------------------------------------------------------

fn settle<F: Fn() -> bool>(what: &str, predicate: F) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!("{what} did not become true within {SETTLE:?}");
}

fn work_states(pg: &Pg, session: Uuid) -> Vec<String> {
    pg.server.query_column(&format!(
        "SELECT state FROM consolidation_work
          WHERE session_id = '{session}' ORDER BY session_seq"
    ))
}

fn attempts(pg: &Pg, session: Uuid) -> Vec<i64> {
    pg.server
        .query_column(&format!(
            "SELECT attempts::text FROM consolidation_work
              WHERE session_id = '{session}' ORDER BY session_seq"
        ))
        .iter()
        .map(|a| a.parse().expect("an integer attempt count"))
        .collect()
}

fn session_state(pg: &Pg, session: Uuid) -> String {
    pg.server.text(&format!(
        "SELECT state FROM consolidation_session WHERE session_id = '{session}'"
    ))
}

fn pending(pg: &Pg, session: Uuid) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_work
          WHERE session_id = '{session}' AND state = 'pending'"
    ))
}

fn runs(pg: &Pg, session: Uuid) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_runs WHERE session_id = '{session}'"
    ))
}

fn drained(pg: &Pg, session: Uuid) -> bool {
    pending(pg, session) == 0 && session_state(pg, session) == "done"
}

// ---------------------------------------------------------------------------
// One session, one pass
// ---------------------------------------------------------------------------

#[test]
fn a_pass_counts_one_attempt_per_event_and_releases_the_lease_it_took() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_pending(&pg, &pg.owner, session, 3, 0);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the session drains", || drained(&pg, session));

    // Exactly one, per event. Two would mean the session was claimed twice for
    // work that only ran once, which is the counter losing its meaning.
    assert_eq!(attempts(&pg, session), vec![1, 1, 1]);
    assert_eq!(work_states(&pg, session), vec!["done", "done", "done"]);
    // The lease is committed state, not a lock, so releasing it is an explicit
    // write and worth asserting: a session left `claimed` with a live lease is
    // a session nothing elects for five minutes.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{session}'
                AND (claimed_by IS NOT NULL OR claim_expires_at IS NOT NULL)"
        )),
        0,
        "the lease outlived the pass that took it"
    );
    assert_eq!(runs(&pg, session), 1, "one session, one batch, one pass");
}

#[test]
fn sessions_are_elected_oldest_first() {
    let pg = pg!();
    let older = pg.session_for(&pg.owner);
    let newer = pg.session_for(&pg.owner);
    // Both seeded before any worker exists, so the order under test is the
    // election's and not the order the rows happened to be written in.
    seed_session(
        &pg,
        &pg.owner,
        older,
        Seed {
            events: 2,
            enqueued: "now() - interval '1 hour'",
            ..Seed::default()
        },
    );
    seed_pending(&pg, &pg.owner, newer, 2, 0);

    let _worker = Worker::start(&pg.server.database_url);
    settle("both sessions drain", || {
        drained(&pg, older) && drained(&pg, newer)
    });

    // `ORDER BY oldest_enqueued_at` is what makes the queue a queue. Ordering
    // by anything else would let a session enqueued an hour ago wait behind one
    // enqueued a moment ago, indefinitely, while events kept arriving.
    let first = pg.server.text(
        "SELECT session_id::text FROM consolidation_runs ORDER BY started_at, run_id LIMIT 1",
    );
    assert_eq!(
        first,
        older.to_string(),
        "the newer session was consolidated first"
    );
}

// ---------------------------------------------------------------------------
// Leases
// ---------------------------------------------------------------------------

#[test]
fn a_live_lease_is_never_taken_from_the_worker_holding_it() {
    let pg = pg!();
    let held = pg.session_for(&pg.owner);
    let free = pg.session_for(&pg.owner);
    seed_session(
        &pg,
        &pg.owner,
        held,
        Seed {
            events: 2,
            state: "claimed",
            claimed_by: "'another-worker'",
            expires: "now() + interval '1 hour'",
            enqueued: "now() - interval '1 hour'",
            ..Seed::default()
        },
    );
    seed_pending(&pg, &pg.owner, free, 2, 0);

    let _worker = Worker::start(&pg.server.database_url);
    // The free session draining is the control: without it, "the held session
    // was untouched" would also be true of a worker that never started.
    settle("the unclaimed session drains", || drained(&pg, free));

    // `held` sorts first by `oldest_enqueued_at`, so a worker that ignored the
    // lease would have taken it before touching `free`.
    assert_eq!(
        attempts(&pg, held),
        vec![0, 0],
        "a second worker started a pass on a session already being worked"
    );
    assert_eq!(session_state(&pg, held), "claimed");
    assert_eq!(
        pg.server.text(&format!(
            "SELECT claimed_by FROM consolidation_session WHERE session_id = '{held}'"
        )),
        "another-worker"
    );
}

#[test]
fn an_expired_lease_is_reclaimed_and_the_crashed_attempt_is_not_refunded() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Exactly what a worker that died mid-pass leaves committed: the claim
    // transaction had already taken the lease and counted the attempt, and the
    // close transaction never ran. Three attempts have started; the third is
    // the one whose worker died.
    seed_session(
        &pg,
        &pg.owner,
        session,
        Seed {
            attempts: 3,
            state: "claimed",
            claimed_by: "'dead-worker'",
            expires: "now() - interval '1 minute'",
            enqueued: "now() - interval '10 minutes'",
            ..Seed::default()
        },
    );

    let _worker = Worker::start(&pg.server.database_url);
    settle("the abandoned session is reclaimed and drains", || {
        drained(&pg, session)
    });

    // Four, not one and not three. The crashed pass consumed its attempt — that
    // is what stops a crash loop retrying forever — and the reclaim resumed
    // from that count rather than restarting it.
    assert_eq!(attempts(&pg, session), vec![4]);
    assert_eq!(work_states(&pg, session), vec!["done"]);
}

// ---------------------------------------------------------------------------
// The five-attempt rule
// ---------------------------------------------------------------------------

#[test]
fn an_event_that_has_failed_fewer_than_five_times_is_tried_again() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    close_session(&pg, session);
    // Four events, one per already-spent attempt count. Failures 1–4 retry, so
    // every one of these is claimable and every one gains exactly one attempt.
    for (i, spent) in [1, 2, 3, 4].into_iter().enumerate() {
        let seq = (i as i64) + 1;
        if i == 0 {
            pg.server.execute(&format!(
                "INSERT INTO consolidation_session (project_id, session_id, state)
                 VALUES ('{}', '{session}', 'pending')",
                pg.project
            ));
        }
        pg.server.execute(&format!(
            "INSERT INTO safe_events
                 (event_id, project_id, session_id, account_id, agent, kind,
                  session_seq, contract_version, content, occurred_at)
             VALUES (gen_random_uuid(), '{}', '{session}', '{}', 'claude_code',
                     'file_changed', {seq}, 1, '{{}}'::jsonb, now())",
            pg.project, pg.owner.id
        ));
        pg.server.execute(&format!(
            "INSERT INTO consolidation_work
                 (event_id, project_id, session_id, session_seq, state, attempts)
             SELECT event_id, project_id, session_id, session_seq, 'pending', {spent}
               FROM safe_events WHERE session_id = '{session}' AND session_seq = {seq}"
        ));
    }

    let _worker = Worker::start(&pg.server.database_url);
    settle("the session drains", || drained(&pg, session));

    assert_eq!(attempts(&pg, session), vec![2, 3, 4, 5]);
    assert_eq!(
        work_states(&pg, session),
        vec!["done", "done", "done", "done"]
    );
}

#[test]
fn a_success_on_the_fifth_attempt_is_done_and_not_failed() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Four attempts spent; the pass this worker runs is the fifth and last.
    seed_pending(&pg, &pg.owner, session, 1, 4);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the session drains", || drained(&pg, session));

    // The close transaction marks successes `done` **before** it retires
    // fifth-attempt failures. Reversing those two statements would sweep this
    // row — `pending` with `attempts >= 5` at the moment the sweep ran — into
    // `failed`, discarding work that actually succeeded.
    assert_eq!(attempts(&pg, session), vec![5]);
    assert_eq!(work_states(&pg, session), vec!["done"]);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{session}' AND last_error IS NOT NULL"
        )),
        0,
        "a successful fifth attempt recorded an error"
    );
}

#[test]
fn a_fifth_attempt_that_never_reported_becomes_failed_and_never_starts_a_sixth() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Five attempts have started and the fifth never closed. Claim selection
    // requires `attempts < 5`, so there is nothing here left to run.
    seed_pending(&pg, &pg.owner, session, 1, 5);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the exhausted session is retired", || {
        session_state(&pg, session) == "done"
    });

    assert_eq!(work_states(&pg, session), vec!["failed"]);
    assert_eq!(
        attempts(&pg, session),
        vec![5],
        "a sixth attempt was started on an event that had already had five"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{session}' AND last_error IS NULL"
        )),
        0,
        "a failed event has to say why, or system health cannot report it"
    );

    // And it stays retired. A `failed` row is outside the pending predicate, so
    // nothing re-elects the session and nothing retries the event.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(attempts(&pg, session), vec![5]);
    assert_eq!(session_state(&pg, session), "done");
}

#[test]
fn work_that_has_exhausted_its_attempts_never_strands_its_session() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    close_session(&pg, session);
    // One event still has attempts left; the other has none. The session must
    // finish `done` rather than staying electable forever on the strength of a
    // row nothing will ever run again.
    pg.server.execute(&format!(
        "INSERT INTO consolidation_session (project_id, session_id, state)
         VALUES ('{}', '{session}', 'pending')",
        pg.project
    ));
    for (seq, spent) in [(1i64, 0i32), (2, 5)] {
        pg.server.execute(&format!(
            "INSERT INTO safe_events
                 (event_id, project_id, session_id, account_id, agent, kind,
                  session_seq, contract_version, content, occurred_at)
             VALUES (gen_random_uuid(), '{}', '{session}', '{}', 'claude_code',
                     'file_changed', {seq}, 1, '{{}}'::jsonb, now())",
            pg.project, pg.owner.id
        ));
        pg.server.execute(&format!(
            "INSERT INTO consolidation_work
                 (event_id, project_id, session_id, session_seq, state, attempts)
             SELECT event_id, project_id, session_id, session_seq, 'pending', {spent}
               FROM safe_events WHERE session_id = '{session}' AND session_seq = {seq}"
        ));
    }

    let _worker = Worker::start(&pg.server.database_url);
    settle("the session finishes despite the exhausted event", || {
        session_state(&pg, session) == "done"
    });

    assert_eq!(work_states(&pg, session), vec!["done", "failed"]);
    assert_eq!(pending(&pg, session), 0);
    // Still done a moment later: the `EXISTS` that chooses `pending` versus
    // `done` looks only for `pending`, so a `failed` remainder cannot re-open
    // the session and spin it.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(session_state(&pg, session), "done");
}

// ---------------------------------------------------------------------------
// Batching
// ---------------------------------------------------------------------------

#[test]
fn a_session_with_more_than_one_batch_is_re_elected_immediately() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_pending(&pg, &pg.owner, session, 205, 0);

    let _worker = Worker::start(&pg.server.database_url);
    // Thirty seconds, against a five-minute lease. An earlier form of the close
    // statement guarded on `NOT EXISTS (pending)` instead of choosing with a
    // `CASE`: when work remained it matched no row, the session stayed
    // `claimed` with its lease intact, and the remainder waited out the full
    // five minutes. That defect cannot pass this deadline.
    settle("both batches drain", || drained(&pg, session));

    assert_eq!(pending(&pg, session), 0);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{session}' AND state = 'done'"
        )),
        205
    );
    // Two passes, and the first one claimed exactly a batch. The number is the
    // bound in §6, so it is asserted rather than assumed.
    assert_eq!(runs(&pg, session), 2);
    assert_eq!(
        pg.server
            .query_column(&format!(
                "SELECT events_claimed::text FROM consolidation_runs
                  WHERE session_id = '{session}' ORDER BY started_at, run_id"
            ))
            .iter()
            .map(|n| n.parse::<i64>().expect("a claim count"))
            .collect::<Vec<_>>(),
        vec![200, 5]
    );
}

// ---------------------------------------------------------------------------
// Two workers
// ---------------------------------------------------------------------------

#[test]
fn two_workers_never_start_a_pass_on_the_same_session() {
    let pg = pg!();
    let sessions: Vec<Uuid> = (0..12).map(|_| pg.session_for(&pg.owner)).collect();
    for session in &sessions {
        seed_pending(&pg, &pg.owner, *session, 4, 0);
    }

    let _a = Worker::start(&pg.server.database_url);
    let _b = Worker::start(&pg.server.database_url);
    settle("every session drains", || {
        sessions.iter().all(|s| drained(&pg, *s))
    });

    // Every event has been through exactly one pass. A session claimed
    // concurrently by both workers would have counted the attempt twice — the
    // increment is per event, in the claim transaction — so a maximum above one
    // is the signature of `FOR UPDATE SKIP LOCKED` failing to exclude.
    assert_eq!(
        pg.server
            .count("SELECT COALESCE(max(attempts), 0)::bigint FROM consolidation_work"),
        1
    );
    assert_eq!(
        pg.server
            .count("SELECT COALESCE(min(attempts), 0)::bigint FROM consolidation_work"),
        1
    );
    // Twelve sessions, four events each: one batch apiece, so one pass apiece.
    assert_eq!(
        pg.server.count("SELECT count(*) FROM consolidation_runs"),
        12
    );
}

// ---------------------------------------------------------------------------
// Re-opening
// ---------------------------------------------------------------------------

/// The UUIDv5 the server re-derives, computed the way the daemon does.
fn file_event(session: Uuid, seq: u64, path: &str) -> Value {
    json!({
        "event_id": cairn_core::eventid::event_id(session, seq),
        "contract_version": 1,
        "kind": "file_changed",
        "agent": "claude_code",
        "vendor_event": "PostToolUse",
        "session_id": session,
        "session_seq": seq,
        "occurred_at": "2026-09-02T10:00:00Z",
        "content": { "File": {
            "repo_file": path,
            "repo_file_from": null,
            "change_kind": "modified",
            "file_identity": "present"
        }},
    })
}

fn ingest(pg: &Pg, who: &Account, event: Value) {
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &json!({ "contract_version": 1, "events": [event] }),
        &who.token,
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["results"][0]["status"], "accepted", "{body}");
}

#[test]
fn an_event_arriving_for_a_finished_session_re_opens_it_and_it_is_consolidated_again() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Closed, so its work is eligible at once. A late event arriving for a
    // session that has already ended is exactly the case re-opening exists
    // for.
    close_session(&pg, session);
    // Through the real boundary: re-opening is what the ingest upsert's `CASE`
    // does, and asserting it against a hand-written `UPDATE` here would prove
    // only that this file can write SQL.
    ingest(
        &pg,
        &pg.owner,
        file_event(session, 1, "crates/cairnd/src/sync.rs"),
    );

    let _worker = Worker::start(&pg.server.database_url);
    settle("the first event is consolidated", || drained(&pg, session));

    // The session is `done` now, and the partial election index excludes
    // `done`. An arriving event has to put it back in the queue or it is
    // stranded forever.
    ingest(
        &pg,
        &pg.owner,
        file_event(session, 2, "crates/cairnd/src/main.rs"),
    );
    settle("the re-opened session is consolidated", || {
        pending(&pg, session) == 0 && runs(&pg, session) == 2
    });

    assert_eq!(work_states(&pg, session), vec!["done", "done"]);
    assert_eq!(session_state(&pg, session), "done");
}

// ---------------------------------------------------------------------------
// The bound that keeps consolidation out of request serving's way
// ---------------------------------------------------------------------------

#[test]
fn a_server_whose_pool_share_would_be_zero_does_not_consolidate_at_all() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_pending(&pg, &pg.owner, session, 2, 0);

    // The harness starts its servers with a pool of four, and
    // `min(2, floor(4/5))` is zero: below five connections consolidation does
    // not run, because a fixed share of two would take half a small pool and
    // starve the request serving it shares a process with (FR-793a1, FR-814).
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(attempts(&pg, session), vec![0, 0]);
    assert_eq!(session_state(&pg, session), "pending");
    assert_eq!(runs(&pg, session), 0);

    // And the work really was drainable, so the assertion above is about the
    // bound rather than about a session nothing could have consolidated.
    let _worker = Worker::start(&pg.server.database_url);
    settle("a server with a share does consolidate it", || {
        drained(&pg, session)
    });
}

// ---------------------------------------------------------------------------
// Eligibility (contracts/consolidation.md §3)
// ---------------------------------------------------------------------------

/// A session is elected only once one of the three triggers has fired.
///
/// The worker used to elect every pending session the instant an event arrived,
/// which is not what §3 says and is worse than it sounds: a session that fires
/// a tool call every few seconds would be consolidated a few seconds at a time,
/// and every one of those passes reads a fragment of a sequence the extraction
/// rules only mean anything over.
fn seed_open(pg: &Pg, who: &Account, session: Uuid, events: i64, enqueued: &str) {
    seed_session(
        pg,
        who,
        session,
        Seed {
            events,
            enqueued,
            closed: false,
            ..Seed::default()
        },
    );
}

/// Wait long enough to be confident a worker had its chance and declined.
///
/// A negative assertion needs a positive control, or it passes on a worker that
/// was simply never running. Every test below that asserts "not claimed" also
/// makes a second session eligible and waits for *that* one to drain, so the
/// worker is demonstrably alive and choosing.
fn eligible_control(pg: &Pg, who: &Account) -> Uuid {
    let control = pg.session_for(who);
    close_session(pg, control);
    seed_session(
        pg,
        who,
        control,
        Seed {
            closed: true,
            ..Seed::default()
        },
    );
    control
}

#[test]
fn one_recent_event_in_an_open_session_is_not_eligible() {
    let pg = pg!();
    let quiet = pg.session_for(&pg.owner);
    seed_open(&pg, &pg.owner, quiet, 1, "now()");
    let control = eligible_control(&pg, &pg.owner);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the control session drains", || drained(&pg, control));

    assert_eq!(
        pending(&pg, quiet),
        1,
        "a single recent event in an open session was consolidated on its own"
    );
    assert_eq!(session_state(&pg, quiet), "pending");
    assert_eq!(runs(&pg, quiet), 0);
}

#[test]
fn one_event_short_of_a_full_batch_is_not_eligible() {
    let pg = pg!();
    let almost = pg.session_for(&pg.owner);
    seed_open(&pg, &pg.owner, almost, 199, "now()");
    let control = eligible_control(&pg, &pg.owner);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the control session drains", || drained(&pg, control));

    assert_eq!(
        pending(&pg, almost),
        199,
        "199 recent events in an open session were consolidated"
    );
}

#[test]
fn a_full_batch_of_recent_events_is_eligible() {
    let pg = pg!();
    let full = pg.session_for(&pg.owner);
    seed_open(&pg, &pg.owner, full, 200, "now()");

    let _worker = Worker::start(&pg.server.database_url);
    settle("a full batch is consolidated", || drained(&pg, full));
    assert_eq!(runs(&pg, full), 1);
}

#[test]
fn one_event_older_than_ten_minutes_is_eligible() {
    let pg = pg!();
    let aged = pg.session_for(&pg.owner);
    // The condition that makes a long-running session produce knowledge before
    // it ends. Without it an afternoon-long session that never reaches 200
    // events consolidates nothing until it closes.
    seed_open(&pg, &pg.owner, aged, 1, "now() - interval '11 minutes'");

    let _worker = Worker::start(&pg.server.database_url);
    settle("the aged event is consolidated", || drained(&pg, aged));
    assert_eq!(runs(&pg, aged), 1);
}

#[test]
fn one_event_just_inside_ten_minutes_is_not_yet_eligible() {
    let pg = pg!();
    let nearly = pg.session_for(&pg.owner);
    seed_open(&pg, &pg.owner, nearly, 1, "now() - interval '9 minutes'");
    let control = eligible_control(&pg, &pg.owner);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the control session drains", || drained(&pg, control));
    assert_eq!(
        pending(&pg, nearly),
        1,
        "an event nine minutes old was treated as ten"
    );
}

#[test]
fn a_closed_session_is_eligible_immediately_with_one_event() {
    let pg = pg!();
    let ended = pg.session_for(&pg.owner);
    close_session(&pg, ended);
    seed_session(
        &pg,
        &pg.owner,
        ended,
        Seed {
            closed: true,
            ..Seed::default()
        },
    );

    let _worker = Worker::start(&pg.server.database_url);
    settle("a closed session drains at once", || drained(&pg, ended));
    assert_eq!(runs(&pg, ended), 1);
}

#[test]
fn the_tail_of_a_partial_batch_is_re_elected_without_waiting_for_a_new_trigger() {
    let pg = pg!();
    let busy = pg.session_for(&pg.owner);
    // 205 events in an **open** session. The volume trigger fires at 200, the
    // first pass takes 200, and the remaining five satisfy no trigger of their
    // own — not 200, not ten minutes old, not a closed session. Without the
    // latch they would wait for a ten-minute clock that had just been reset,
    // which is a stall the five-minute lease does not even explain.
    seed_open(&pg, &pg.owner, busy, 205, "now()");

    let _worker = Worker::start(&pg.server.database_url);
    settle("both passes complete", || drained(&pg, busy));

    // Two passes, not one, and the second was elected immediately rather than
    // after any wait — `settle`'s own deadline is what proves that.
    assert_eq!(
        runs(&pg, busy),
        2,
        "the tail was not processed in a second pass"
    );
    assert_eq!(session_state(&pg, busy), "done");
    // And the latch is released once the generation is finished, so the next
    // generation has to earn its own eligibility.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{busy}' AND eligible_since IS NULL"
        )),
        1,
        "the latch outlived the generation that set it"
    );
}

#[test]
fn a_latched_generation_is_elected_by_a_worker_that_never_saw_it_latched() {
    let pg = pg!();
    let busy = pg.session_for(&pg.owner);
    // An open session, five recent events: it satisfies **no** live trigger —
    // not 200, not ten minutes, not closed. The only thing that can make it
    // electable is the latch.
    seed_open(&pg, &pg.owner, busy, 5, "now()");
    // Latched by a previous generation's pass, which then died. This is the
    // state a restart leaves behind, seeded directly rather than raced for: an
    // in-memory latch would simply not exist here, and the tail would strand.
    pg.server.execute(&format!(
        "UPDATE consolidation_session SET eligible_since = now() - interval '1 minute'
          WHERE session_id = '{busy}'"
    ));

    let _worker = Worker::start(&pg.server.database_url);
    settle("the latched tail drains", || drained(&pg, busy));
    assert_eq!(runs(&pg, busy), 1);
    // And the latch is released now the generation is finished, so the next one
    // must earn its own eligibility.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{busy}' AND eligible_since IS NULL"
        )),
        1,
        "the latch outlived the generation that set it"
    );
}

#[test]
fn an_unlatched_generation_with_the_same_shape_is_left_alone() {
    let pg = pg!();
    // The control for the test above, and the reason it is not vacuous: the
    // identical session without the latch must not be elected.
    let quiet = pg.session_for(&pg.owner);
    seed_open(&pg, &pg.owner, quiet, 5, "now()");
    let control = eligible_control(&pg, &pg.owner);

    let _worker = Worker::start(&pg.server.database_url);
    settle("the control session drains", || drained(&pg, control));
    assert_eq!(
        pending(&pg, quiet),
        5,
        "an unlatched session satisfying no trigger was consolidated"
    );
}

// ---------------------------------------------------------------------------
// Re-opened generations (defect 5)
// ---------------------------------------------------------------------------

#[test]
fn a_re_opened_session_does_not_inherit_the_old_generations_age() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // A finished generation whose work was enqueued yesterday.
    seed_session(
        &pg,
        &pg.owner,
        session,
        Seed {
            state: "done",
            enqueued: "now() - interval '1 day'",
            closed: false,
            ..Seed::default()
        },
    );
    pg.server.execute(&format!(
        "UPDATE consolidation_work SET state = 'done' WHERE session_id = '{session}'"
    ));

    // One fresh event arrives through the real boundary.
    ingest(
        &pg,
        &pg.owner,
        file_event(session, 9, "crates/cairnd/src/main.rs"),
    );

    assert_eq!(
        session_state(&pg, session),
        "pending",
        "the session did not re-open"
    );
    // The new generation's clock starts now. Carrying yesterday forward would
    // make this single fresh event instantly age-eligible on the strength of
    // work that was consolidated a day ago.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{session}'
                AND oldest_enqueued_at > now() - interval '1 minute'"
        )),
        1,
        "the re-opened generation inherited the old generation's oldest enqueue time"
    );
    // And the latch was cleared, so the new generation has met no threshold.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{session}' AND eligible_since IS NULL"
        )),
        1
    );

    let control = eligible_control(&pg, &pg.owner);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the control session drains", || drained(&pg, control));
    assert_eq!(
        pending(&pg, session),
        1,
        "the re-opened session was consolidated on yesterday's clock"
    );
}

#[test]
fn a_re_opened_generation_becomes_eligible_once_it_is_itself_old_enough() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    seed_session(
        &pg,
        &pg.owner,
        session,
        Seed {
            state: "done",
            enqueued: "now() - interval '1 day'",
            closed: false,
            ..Seed::default()
        },
    );
    pg.server.execute(&format!(
        "UPDATE consolidation_work SET state = 'done' WHERE session_id = '{session}'"
    ));
    ingest(
        &pg,
        &pg.owner,
        file_event(session, 9, "crates/cairnd/src/main.rs"),
    );

    // Age the new generation past ten minutes on its own account.
    pg.server.execute(&format!(
        "UPDATE consolidation_session
            SET oldest_enqueued_at = now() - interval '11 minutes'
          WHERE session_id = '{session}'"
    ));

    let _worker = Worker::start(&pg.server.database_url);
    settle("the aged re-opened generation drains", || {
        pending(&pg, session) == 0
    });
}

#[test]
fn a_pending_generation_keeps_the_true_oldest_enqueue_time() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Still `pending`, so the old value really is the age of work that is
    // still waiting and must be preserved rather than reset.
    ingest(&pg, &pg.owner, file_event(session, 1, "a.rs"));
    pg.server.execute(&format!(
        "UPDATE consolidation_session
            SET oldest_enqueued_at = now() - interval '11 minutes'
          WHERE session_id = '{session}'"
    ));
    ingest(&pg, &pg.owner, file_event(session, 2, "b.rs"));

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{session}'
                AND oldest_enqueued_at < now() - interval '10 minutes'"
        )),
        1,
        "a second event reset the clock of work that was already waiting"
    );
}

//! Ingest under an unconsolidatable backlog, as a contract (T045, SC-740,
//! SC-748).
//!
//! The fixture's own server is started with `--max-connections 4`
//! (`tests/src/lib.rs`'s `Server::try_start_at`), and `pool_share(4) == 0`
//! (`floor(4/5)`, `contracts/consolidation.md` §6, FR-793a1) — it never earns
//! a consolidation share and never claims a session, no matter how large the
//! backlog. That is exactly the condition SC-740 asks for: "consolidation
//! stopped", produced with the real binary rather than simulated. Every
//! latency measurement below still goes over real HTTP, through `curl`, at
//! the real `/api/events/batch` route.

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

const SETTLE: Duration = Duration::from_secs(60);

/// The pool a consolidation worker asks for, in the one test here that starts
/// one deliberately — the smallest pool that earns any share at all
/// (`pool_share(5) == 1`).
const WORKER_POOL: &str = "5";

/// How many pending events make up "at least ten thousand" (SC-740).
const BACKLOG_EVENTS: i64 = 10_000;

/// How many latency samples make "at least ten trials" (SC-740), with a
/// couple to spare against a stray outlier skewing a ten-sample median.
const TRIALS: usize = 12;

// ---------------------------------------------------------------------------
// A second server on the fixture's database — used only by the health test
// ---------------------------------------------------------------------------

struct Worker {
    child: Child,
}

impl Worker {
    fn start(database_url: &str) -> Self {
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

fn close_session(pg: &Pg, session: Uuid) {
    pg.server.execute(&format!(
        "UPDATE sessions SET status = 'completed', ended_at = now() WHERE id = '{session}'"
    ));
}

// ---------------------------------------------------------------------------
// Ingest, over real HTTP
// ---------------------------------------------------------------------------

fn event_id(session: Uuid, seq: u64) -> Uuid {
    cairn_core::eventid::event_id(session, seq)
}

fn file_event(session: Uuid, seq: u64, path: &str) -> Value {
    json!({
        "event_id": event_id(session, seq),
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

fn batch(events: Vec<Value>) -> Value {
    json!({ "contract_version": 1, "events": events })
}

fn post(pg: &Pg, who: &Account, events: Vec<Value>) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &batch(events),
        &who.token,
    )
}

fn statuses(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|o| o["status"].as_str().unwrap_or("?").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn reasons(response: &Value) -> Vec<String> {
    response["results"]
        .as_array()
        .map(|r| {
            r.iter()
                .map(|o| o["reason"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// One ingest trial: a fresh session, one accepted event, the wall-clock time
/// the HTTP round trip took. A fresh session every time so no trial's
/// `(session_id, session_seq)` collides with another's or with the seeded
/// backlog's.
fn ingest_trial(pg: &Pg, who: &Account, label: &str) -> Duration {
    let session = pg.session_for(who);
    let started = Instant::now();
    let (body, status) = post(
        pg,
        who,
        vec![file_event(session, 1, &format!("{label}/f.rs"))],
    );
    let elapsed = started.elapsed();
    assert_eq!(status, 200, "{label}: {body}");
    assert_eq!(statuses(&body), vec!["accepted"], "{label}: {body}");
    assert!(
        reasons(&body).iter().all(|r| r.is_empty()),
        "{label}: an accepted event carried a rejection reason: {body}"
    );
    elapsed
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

// ---------------------------------------------------------------------------
// Seeding a backlog nothing will ever drain
// ---------------------------------------------------------------------------

/// Bulk-seed `count` pending events into one session's backlog: `safe_events`
/// by `generate_series`, the matching `consolidation_work` rows, and one
/// `consolidation_session` row — the shape `contracts/consolidation.md` §2's
/// tables expect, built the way a real ingest burst would have left them,
/// without paying for `count` individual HTTP round trips.
fn seed_backlog(pg: &Pg, who: &Account, session: Uuid, count: i64) {
    pg.server.execute(&format!(
        "INSERT INTO consolidation_session (project_id, session_id, state, oldest_enqueued_at)
         VALUES ('{}', '{session}', 'pending', now())",
        pg.project
    ));
    pg.server.execute(&format!(
        "INSERT INTO safe_events
             (event_id, project_id, session_id, account_id, agent, kind,
              session_seq, contract_version, content, occurred_at)
         SELECT gen_random_uuid(), '{}', '{session}', '{}', 'claude_code',
                'file_changed', g, 1, '{{}}'::jsonb, now()
           FROM generate_series(1, {count}) g",
        pg.project, who.id
    ));
    pg.server.execute(&format!(
        "INSERT INTO consolidation_work
             (event_id, project_id, session_id, session_seq, state, attempts)
         SELECT event_id, project_id, session_id, session_seq, 'pending', 0
           FROM safe_events WHERE session_id = '{session}'"
    ));
}

// ---------------------------------------------------------------------------
// SC-740 — ingest latency is unaffected by a backlog consolidation can't touch
// ---------------------------------------------------------------------------

#[test]
fn ingest_latency_with_a_ten_thousand_event_backlog_stays_within_20_percent_of_the_empty_backlog_median_and_refuses_nothing_for_it(
) {
    let pg = pg!();

    // The empty-backlog baseline, measured first, before anything is seeded.
    let empty: Vec<Duration> = (0..TRIALS)
        .map(|i| ingest_trial(&pg, &pg.owner, &format!("baseline-{i}")))
        .collect();
    let empty_median = median(empty);

    // Consolidation cannot touch this: `pool_share(4) == 0`
    // (`contracts/consolidation.md` §6), so the fixture's own server, which
    // every trial in this test runs against, never claims a session no
    // matter how large `consolidation_work` grows.
    let backlog_session = pg.session_for(&pg.owner);
    seed_backlog(&pg, &pg.owner, backlog_session, BACKLOG_EVENTS);
    assert!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{backlog_session}' AND state = 'pending'"
        )) >= BACKLOG_EVENTS,
        "the backlog was not actually seeded"
    );

    let loaded: Vec<Duration> = (0..TRIALS)
        .map(|i| ingest_trial(&pg, &pg.owner, &format!("loaded-{i}")))
        .collect();
    let loaded_median = median(loaded);

    // 20% tolerance, computed in whole nanoseconds so the comparison is exact
    // rather than float-lossy.
    let bound = empty_median.as_nanos() * 6 / 5;
    assert!(
        loaded_median.as_nanos() <= bound,
        "ingest slowed down under backlog: empty-backlog median {empty_median:?}, \
         loaded median {loaded_median:?}, allowed up to {:?}",
        Duration::from_nanos(bound as u64)
    );

    // The backlog is still there — nothing here accidentally drained it and
    // made the comparison meaningless.
    assert!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{backlog_session}' AND state = 'pending'"
        )) >= BACKLOG_EVENTS
    );
}

// ---------------------------------------------------------------------------
// SC-748 — backlog depth, oldest event and failure count, readable at any time
// ---------------------------------------------------------------------------

#[test]
fn consolidation_health_reports_backlog_depth_oldest_event_and_failures_at_any_time_including_mid_pass_and_after_a_restart(
) {
    let mut pg = pg!();

    // Before anything: an empty, but well-shaped, health report.
    let health = pg
        .server
        .get_json("/api/consolidation/health", &pg.owner.token);
    for field in [
        "backlog_depth",
        "oldest_enqueued_at",
        "failed_events",
        "runs_finished",
        "runs_failed",
        "candidates_proposed",
        "candidates_accepted",
        "candidates_refused",
    ] {
        assert!(
            health.get(field).is_some(),
            "the health report is missing `{field}`: {health}"
        );
    }
    assert_eq!(health["backlog_depth"].as_i64(), Some(0));
    assert!(health["oldest_enqueued_at"].is_null());
    assert_eq!(health["failed_events"].as_i64(), Some(0));

    // A backlog this fixture's own server (pool 4) cannot drain, but a real
    // worker (pool 5) can — one closed session, large enough that a worker
    // claiming it 200 events at a time (`contracts/consolidation.md` §3) is
    // still working on it well after the first pass finishes.
    let session = pg.session_for(&pg.owner);
    let backlog_size = 2_000;
    seed_backlog(&pg, &pg.owner, session, backlog_size);
    close_session(&pg, session);

    let health = pg
        .server
        .get_json("/api/consolidation/health", &pg.owner.token);
    assert_eq!(health["backlog_depth"].as_i64(), Some(backlog_size));
    assert!(
        !health["oldest_enqueued_at"].is_null(),
        "a nonzero backlog reported no oldest outstanding event"
    );

    let worker = Worker::start(&pg.server.database_url);

    // Mid-pass: at least one pass has run (depth has dropped) and the
    // backlog is not yet exhausted (there is still a next pass to make).
    // Health is asked of the fixture's own server throughout — it is a read
    // of committed rows, not of whichever process happens to be consolidating
    // (`consolidation.md` §8: "reportable ... whether or not a pass is
    // running").
    settle("the backlog visibly drains without finishing", || {
        let depth = pg
            .server
            .get_json("/api/consolidation/health", &pg.owner.token)["backlog_depth"]
            .as_i64()
            .unwrap_or(-1);
        depth >= 0 && depth < backlog_size
    });
    let mid_pass = pg
        .server
        .get_json("/api/consolidation/health", &pg.owner.token);
    let mid_depth = mid_pass["backlog_depth"]
        .as_i64()
        .expect("an integer backlog depth");
    assert!(
        (0..backlog_size).contains(&mid_depth),
        "expected to observe the backlog partially drained, got {mid_depth}"
    );
    assert!(mid_pass["failed_events"].as_i64().is_some());
    assert!(mid_pass["runs_finished"].as_i64().unwrap_or(0) >= 1);

    // Immediately after a restart of the *reporting* server — a crash mid
    // deployment must not take health down with it, and the worker (a
    // separate process, on the same database) keeps running through it.
    pg.crash_and_restart();
    let after_restart = pg
        .server
        .get_json("/api/consolidation/health", &pg.owner.token);
    for field in [
        "backlog_depth",
        "failed_events",
        "runs_finished",
        "runs_failed",
    ] {
        assert!(
            after_restart.get(field).and_then(Value::as_i64).is_some(),
            "`{field}` was not readable immediately after a restart: {after_restart}"
        );
    }
    assert!(
        after_restart["backlog_depth"].as_i64().unwrap() >= 0,
        "backlog depth was not sane immediately after a restart: {after_restart}"
    );

    drop(worker);
}

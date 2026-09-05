//! T160 — capture, retrieval and backlog deadlines, measured rather than
//! assumed, with the numbers stored for a later run to compare against
//! (`tests/feature005/performance-measurements.md`).
//!
//! # The four budgets, and where each one actually comes from
//!
//! This file invents nothing. Every threshold below is the number the specs
//! themselves state, at the cited line, and the requirement id is the one the
//! assertion failure message names:
//!
//! 1. **Capture deadline** — `capture_deadline_ms`'s shipped default is **250
//!    ms** (`crates/cairn-core/src/config.rs:106`, asserted again at line 237).
//!    FR-749a requires the richer capture path to fit inside the existing
//!    deadline or have the deadline restated explicitly; FR-749b/c say a
//!    capture-class event that misses it is dropped **fail-soft** — the hook
//!    still exits successfully and the agent is never blocked — with the drop
//!    recorded as `capture_deadline_exceeded` rather than silently lost
//!    (SC-752, `spec.md:1575`). SC-715 (`spec.md:1436`) is the same claim from
//!    the outage side: "agent-facing operations complete within their
//!    existing deadlines" even "with the server unreachable" — capture-class
//!    events are fire-and-forget to the *local* daemon only
//!    (`crates/cairn/src/hook.rs:321-327`), so the remote server's reachability
//!    is exactly the variable this measurement holds constant to prove it
//!    doesn't matter.
//!
//! 2. **Session-open retrieval deadline** — `contracts/retrieval-delivery.md:209`:
//!    "retrieval targets **250 ms at session open** ... as internal soft
//!    targets" (FR-836). This is *not* `context_deadline_ms` (1500 ms,
//!    line 204) — that is the hard bound the hook itself enforces regardless
//!    of what retrieval does; the 250 ms figure is retrieval's own internal
//!    target, missing which degrades a level rather than failing the hook.
//!
//! 3. **Prompt-time retrieval deadline** — same line: "**100 ms at prompt
//!    time** ... prompt-time is tighter because it sits inside the model's
//!    turn."
//!
//! 4. **10,000-event backlog** — SC-740 (`spec.md:1528`): "with consolidation
//!    stopped and a backlog of at least ten thousand outstanding events, event
//!    ingestion accepts and persists safe events at a median latency within
//!    20% of its latency at an empty backlog, over at least ten trials, and
//!    zero ingestion requests are refused with a backlog-derived reason."
//!    `tests/tests/feature005_consolidation_backlog.rs` (T045) already holds
//!    this as a contract test; what this file adds is the same measurement
//!    re-run for T160's own record, so a later run has a number to compare
//!    against rather than only a pass/fail.
//!
//! # Why this file sets deadlines explicitly rather than trusting `Sandbox::new`
//!
//! `Sandbox::new` deliberately relaxes `capture_deadline_ms` to 5000 and
//! `context_deadline_ms` to 15000 (`tests/src/lib.rs:60-74`), because the
//! suite runs many sandboxes at once and the production 250 ms budget turns
//! semantic tests into load tests. A file whose entire point is measuring the
//! production deadline cannot inherit that relaxation, so every capture
//! measurement below calls `Sandbox::set_deadlines(250, 1500)` first.
//!
//! # Style note
//!
//! The retrieval-latency shape here follows
//! `tests/tests/feature005_retrieval_performance.rs`, whose own header
//! explains why a wall-clock threshold on shared CI hardware is normally the
//! wrong thing to assert. That file therefore asserts content invariants
//! instead and deliberately does not assert a latency number. This file is
//! the deliberate exception: T160 exists specifically to put a number on
//! record, on *this* machine, against the budget the contract states — a
//! median across many trials to blunt a single stray outlier, reported
//! alongside the worst trial rather than hiding it.

use cairn_e2e::feature005::{Account, Pg};
use cairn_e2e::{post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
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

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn worst(samples: &[Duration]) -> Duration {
    *samples.iter().max().expect("at least one sample")
}

fn report(label: &str, budget: Duration, samples: &[Duration]) {
    let sorted = {
        let mut s = samples.to_vec();
        s.sort();
        s
    };
    println!(
        "{label}: median {:?}  worst {:?}  budget {:?}  (n={})",
        median(sorted.clone()),
        worst(&sorted),
        budget,
        sorted.len()
    );
}

// ---------------------------------------------------------------------------
// 1. Capture deadline — FR-749a-d, SC-715, SC-752
// ---------------------------------------------------------------------------

/// `crates/cairn-core/src/config.rs:106` — the shipped production default.
const CAPTURE_DEADLINE_MS: u64 = 250;
/// The context-class hard bound the same config carries
/// (`crates/cairn-core/src/config.rs:107`); boundary hooks in this file run
/// under it, unchanged from production, so the outage condition is realistic
/// rather than loosened alongside the capture measurement.
const CONTEXT_DEADLINE_MS: u64 = 1500;

const CAPTURE_TRIALS: usize = 60;

/// Point this sandbox's daemon at an address nothing listens on, keeping
/// every other setting (account identity, deadlines) the read-modify-write
/// this mirrors from `feature005_outage.rs`'s `point_at`, kept local here
/// because test binaries do not share code across files.
fn point_at_unreachable(s: &Sandbox) {
    s.stop_daemon();
    let path = s.cairn_home().join("config.json");
    let text = std::fs::read_to_string(&path).expect("sandbox has a config");
    let mut config: Value = serde_json::from_str(&text).expect("config.json parses");
    config["server_url"] = json!("http://127.0.0.1:1");
    std::fs::write(&path, config.to_string()).expect("write config");
    // The next command starts the daemon again, reading the new endpoint.
    let _ = s.cairn(&["status"]);
}

/// SC-715 + SC-752, measured together: a capture-class hook is fire-and-forget
/// to the local daemon only (`crates/cairn/src/hook.rs:321-327`), so an
/// unreachable *remote* server is exactly the condition that must not move
/// this number. If it did, the architecture's central claim — that a
/// dead-server outage cannot slow or block the agent — would be false.
#[test]
fn capture_class_hooks_return_within_the_production_capture_deadline_with_the_server_unreachable() {
    let s = Sandbox::new();
    // A real account first, so the fixture models a machine that *was*
    // talking to a server and lost it — not one that was never configured.
    let server = match Server::start() {
        Some(server) => server,
        None => {
            eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
            return;
        }
    };
    let token = server.new_user_token("capture-perf");
    cairn_e2e::attach_server(&s, &server, &token);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": "capture-perf", "repository_remote": "git@example.test:capture-perf.git" }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project = created["id"].as_str().expect("id");
    s.must(&["link", "--project", project]);

    // The production deadlines, not the suite's relaxed defaults.
    s.set_deadlines(CAPTURE_DEADLINE_MS, CONTEXT_DEADLINE_MS);
    point_at_unreachable(&s);

    s.hook(
        "SessionStart",
        json!({ "session_id": "perf-capture", "source": "startup" }),
    );
    s.settle_session_count(1);

    let mut samples = Vec::with_capacity(CAPTURE_TRIALS);
    let mut failures = 0usize;
    for i in 0..CAPTURE_TRIALS {
        let started = Instant::now();
        let out = s.hook(
            "PostToolUse",
            json!({
                "session_id": "perf-capture",
                "tool_name": "Edit",
                "tool_input": { "file_path": format!("work/f{i}.rs") }
            }),
        );
        samples.push(started.elapsed());
        if out.code != 0 {
            failures += 1;
        }
    }

    // FR-749b: a capture-class event never becomes an agent-facing failure,
    // deadline or no deadline, server reachable or not.
    assert_eq!(
        failures, 0,
        "{failures} of {CAPTURE_TRIALS} capture hooks exited non-zero with the server \
         unreachable — SC-715 requires zero agent operations blocked"
    );

    report(
        "capture deadline (server unreachable)",
        Duration::from_millis(CAPTURE_DEADLINE_MS),
        &samples,
    );

    let m = median(samples.clone());
    let w = worst(&samples);
    let budget = Duration::from_millis(CAPTURE_DEADLINE_MS);
    assert!(
        m <= budget,
        "median capture latency {m:?} exceeded the {budget:?} production capture deadline \
         (capture_deadline_ms, crates/cairn-core/src/config.rs:106) with the server unreachable \
         (SC-715)"
    );
    assert!(
        w <= budget,
        "the worst of {CAPTURE_TRIALS} capture hooks took {w:?}, over the {budget:?} production \
         capture deadline (SC-752: a hook must return within it, fail-soft or not)"
    );
}

// ---------------------------------------------------------------------------
// 2 & 3. Retrieval deadlines — FR-836, contracts/retrieval-delivery.md §5
// ---------------------------------------------------------------------------

/// `contracts/retrieval-delivery.md:209` — the internal soft target at
/// session open.
const SESSION_OPEN_DEADLINE_MS: u64 = 250;
/// Same line — the tighter target at prompt time, "because it sits inside the
/// model's turn."
const PROMPT_DEADLINE_MS: u64 = 100;

const RETRIEVAL_TRIALS: usize = 15;

fn seed_project_memory(pg: &Pg, session: Uuid, how_many: usize) {
    for i in 0..how_many {
        pg.server.execute(&format!(
            "INSERT INTO memories
                (id, project_id, type, scope, scope_key, content, state, origin_session_id,
                 topic_key, value_key, origin_kind)
             VALUES ('{}', '{}', 'fact', 'project', '{}', 'project fact number {i}', 'active',
                     '{session}', 'topic.n{i}', 'v{i}', 'explicit')",
            Uuid::now_v7(),
            pg.project,
            pg.project
        ));
    }
}

fn retrieve(pg: &Pg, who: &Account, session: Uuid, trigger: &str) -> (Value, u16) {
    post_json_status_bearer(
        &pg.server.base,
        "/api/retrieve",
        &json!({ "session_id": session, "trigger": trigger }),
        &who.token,
    )
}

/// Runs `RETRIEVAL_TRIALS` retrievals at `trigger`, over a fresh session each
/// time (so `explicit`-style dedup on repeat asks never shrinks what is
/// measured), and returns the wall-clock elapsed for each successful call.
fn retrieval_latencies(pg: &Pg, trigger: &str) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(RETRIEVAL_TRIALS);
    for _ in 0..RETRIEVAL_TRIALS {
        let session = pg.session_for(&pg.owner);
        seed_project_memory(pg, session, 8);
        let started = Instant::now();
        let (answer, status) = retrieve(pg, &pg.owner, session, trigger);
        let elapsed = started.elapsed();
        assert_eq!(status, 200, "{trigger}: {answer}");
        samples.push(elapsed);
    }
    samples
}

#[test]
fn session_open_retrieval_meets_its_250ms_soft_target() {
    let pg = pg!();
    let samples = retrieval_latencies(&pg, "session_open");
    report(
        "session-open retrieval",
        Duration::from_millis(SESSION_OPEN_DEADLINE_MS),
        &samples,
    );

    let m = median(samples.clone());
    let budget = Duration::from_millis(SESSION_OPEN_DEADLINE_MS);
    assert!(
        m <= budget,
        "median session-open retrieval latency {m:?} exceeded the {budget:?} soft target \
         (FR-836, contracts/retrieval-delivery.md:209) over {RETRIEVAL_TRIALS} trials"
    );
}

#[test]
fn prompt_time_retrieval_meets_its_tighter_100ms_soft_target() {
    let pg = pg!();
    let samples = retrieval_latencies(&pg, "prompt_submit");
    report(
        "prompt-time retrieval",
        Duration::from_millis(PROMPT_DEADLINE_MS),
        &samples,
    );

    let m = median(samples.clone());
    let budget = Duration::from_millis(PROMPT_DEADLINE_MS);
    assert!(
        m <= budget,
        "median prompt-time retrieval latency {m:?} exceeded the {budget:?} soft target \
         (FR-836, contracts/retrieval-delivery.md:209) over {RETRIEVAL_TRIALS} trials — \
         prompt time is deliberately tighter than session open because it sits inside the \
         model's turn"
    );
}

// ---------------------------------------------------------------------------
// 4. 10,000-event backlog — SC-740
// ---------------------------------------------------------------------------

/// SC-740's own count: "a backlog of at least ten thousand outstanding events".
const BACKLOG_EVENTS: i64 = 10_000;
/// SC-740's own trial count: "over at least ten trials". A couple extra
/// against a stray outlier skewing a ten-sample median, matching
/// `feature005_consolidation_backlog.rs`'s own choice.
const BACKLOG_TRIALS: usize = 12;

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

fn ingest_trial(pg: &Pg, who: &Account, label: &str) -> (Duration, bool) {
    let session = pg.session_for(who);
    let started = Instant::now();
    let (body, status) = post_json_status_bearer(
        &pg.server.base,
        "/api/events/batch",
        &json!({ "contract_version": 1, "events": [file_event(session, 1, &format!("{label}/f.rs"))] }),
        &who.token,
    );
    let elapsed = started.elapsed();
    let accepted = status == 200
        && body["results"][0]["status"].as_str() == Some("accepted")
        && body["results"][0]["reason"]
            .as_str()
            .unwrap_or("")
            .is_empty();
    (elapsed, accepted)
}

/// Bulk-seed `count` pending events into one session's backlog, in the shape
/// `contracts/consolidation.md` §2's tables expect — matching
/// `feature005_consolidation_backlog.rs`'s `seed_backlog`, without paying for
/// `count` individual HTTP round trips.
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

/// SC-740. `Pg::start`'s server runs with `--max-connections 4`
/// (`tests/src/lib.rs`), and `pool_share(4) == 0` (`floor(4/5)`,
/// `contracts/consolidation.md` §6, FR-793a1) — it never earns a
/// consolidation share and never claims a session, so "consolidation
/// stopped" holds for the whole test without a second process.
#[test]
fn ingest_latency_holds_within_20_percent_under_a_ten_thousand_event_backlog() {
    let pg = pg!();

    let mut empty = Vec::with_capacity(BACKLOG_TRIALS);
    for i in 0..BACKLOG_TRIALS {
        let (elapsed, accepted) = ingest_trial(&pg, &pg.owner, &format!("baseline-{i}"));
        assert!(accepted, "baseline ingest {i} was not accepted");
        empty.push(elapsed);
    }
    let empty_median = median(empty.clone());

    let backlog_session = pg.session_for(&pg.owner);
    seed_backlog(&pg, &pg.owner, backlog_session, BACKLOG_EVENTS);
    assert!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{backlog_session}' AND state = 'pending'"
        )) >= BACKLOG_EVENTS,
        "the backlog was not actually seeded"
    );

    let mut loaded = Vec::with_capacity(BACKLOG_TRIALS);
    for i in 0..BACKLOG_TRIALS {
        let (elapsed, accepted) = ingest_trial(&pg, &pg.owner, &format!("loaded-{i}"));
        assert!(accepted, "loaded-backlog ingest {i} was not accepted");
        loaded.push(elapsed);
    }
    let loaded_median = median(loaded.clone());

    println!(
        "10,000-event backlog ingest: empty-backlog median {empty_median:?}  \
         loaded median {loaded_median:?}  (n={BACKLOG_TRIALS} each side, budget: loaded <= 120% \
         of empty)"
    );

    let bound = empty_median.as_nanos() * 6 / 5;
    assert!(
        loaded_median.as_nanos() <= bound,
        "ingest slowed down under a {BACKLOG_EVENTS}-event backlog: empty-backlog median \
         {empty_median:?}, loaded median {loaded_median:?}, SC-740 allows up to {:?}",
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

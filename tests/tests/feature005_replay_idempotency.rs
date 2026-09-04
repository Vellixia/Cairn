//! Replay after a lost response, as a contract (T104, SC-716, FR-786).
//!
//! # The failure this file hunts
//!
//! The server accepts a batch and the response never reaches the daemon. From
//! the daemon's side that is indistinguishable from "the server never saw it",
//! so it must retry — and the retry has to be a no-op that reports success. A
//! naive implementation makes a second copy of everything the first delivery
//! made, and the copy is silent: no error, no refusal, just two canonical
//! events, two consolidation inputs, two memories.
//!
//! So the scenario is reproduced at the protocol level rather than by staging a
//! real outage: posting the identical batch N times **is** response loss,
//! exactly and deterministically, because the two are the same bytes on the
//! wire. An end-to-end outage — spool, drain, reconnect — is
//! `feature005_outage.rs`'s subject. What this file owns is the arithmetic:
//! however many times a delivery is repeated, one canonical event per distinct
//! event, one consolidation input, one durable effect.
//!
//! # Why "ten trials" and not one
//!
//! SC-716 says "any number of times" and "in 100% of trials". A single replay
//! proves the second delivery is handled; it says nothing about the tenth, and
//! nothing about a fresh session whose first delivery raced a second. So the
//! event tests replay ten times across ten independent sessions, and one of
//! them replays concurrently — the reservation and the `ON CONFLICT` are taken
//! inside a transaction precisely so two simultaneous deliveries collapse to
//! one effect, and a sequential-only test cannot see them being taken outside
//! it.
//!
//! # What is deliberately not here
//!
//! The transmission endpoint's own idempotency — repeating an outcome, and
//! refusing a conflicting one — is already held by `feature005_delivery.rs`
//! (`repeating_the_same_outcome_is_a_duplicate_with_no_second_effect`, which
//! asserts both the `delivered_context` row count and that the delivery
//! timestamp did not move, and
//! `an_opposite_terminal_outcome_is_refused_and_the_first_one_stands`).
//! Restating them here would be two tests failing for one defect. What that
//! file does not cover, and this one adds, is the *other* replay on that
//! surface: retrying the **retrieval** rather than the report must never
//! fabricate a delivery.
//!
//! Likewise `a_rolled_back_effect_leaves_the_command_replayable`
//! (`feature005_command_delivery.rs`) already proves a failed command's id is
//! not spent. The neighbouring property — a *succeeded* command's id is spent
//! forever, and stays spent across a server crash — is what this file asserts
//! instead.

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

/// Independent trials of the same property (SC-716's "100% of trials").
///
/// Each trial is its own session, so a trial cannot pass on state a previous
/// trial established — which is the only way "100%" means anything.
const TRIALS: usize = 10;

/// Deliveries of one identical batch within a trial (SC-716's "any number of
/// times"). One is the first; the other nine are the lost-response retries.
const DELIVERIES: usize = 10;

/// Concurrent deliveries of one identical batch.
const RACERS: usize = 8;

/// Events in the batch the concurrent deliveries race over.
///
/// Larger than the batches elsewhere in this file, and sized deliberately. The
/// window a concurrent duplicate can fall into is one event wide, so a batch of
/// four gives eight racers only a handful of chances to find it and a run that
/// missed would report "idempotent under concurrency" on no evidence. The
/// racers stay in step with one another — same work, same rate — so every event
/// in the batch is another collision, and the test becomes a reliable observer
/// of the property rather than a lottery.
const RACE_EVENTS: u64 = 32;

/// How long a property is given to become true before the test calls it false.
const SETTLE: Duration = Duration::from_secs(30);

/// How long a property that must **not** become true is watched for.
///
/// Twenty times `BATCH_YIELD` (100 ms), so a worker that was going to elect the
/// session again has had many chances to. A shorter window would let "no second
/// pass" mean "no second pass yet".
const QUIET: Duration = Duration::from_secs(2);

/// The pool a worker asks for — the smallest that earns any consolidation share
/// at all (`pool_share(5) == 1`, FR-793a1). The fixture's own server runs with
/// four and never consolidates, which is what lets every other test here
/// observe the consolidation *queue* without a worker draining it underneath.
const WORKER_POOL: &str = "5";

// ---------------------------------------------------------------------------
// A second server on the fixture's database — the consolidation worker
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
// Events, built the way the daemon builds them
// ---------------------------------------------------------------------------

/// The UUIDv5 the server re-derives from the session and the ordinal
/// (`contracts/safe-events.md` §4). Two sends of the same event carry the same
/// id because the id is a function of what the event *is*, not of when it was
/// sent — which is the whole mechanism under test here.
fn event_id(session: Uuid, seq: u64) -> Uuid {
    cairn_core::eventid::event_id(session, seq)
}

fn event(session: Uuid, seq: u64, kind: &str, content: Value) -> Value {
    json!({
        "event_id": event_id(session, seq),
        "contract_version": 1,
        "kind": kind,
        "agent": "claude_code",
        "vendor_event": "PostToolUse",
        "session_id": session,
        "session_seq": seq,
        "occurred_at": "2026-09-02T10:00:00Z",
        "content": content,
    })
}

/// A `file_changed` event — deliberately the plainest kind there is.
///
/// It carries no `subject_token` or `object_token`, so it never reaches the
/// vocabulary gate. That matters for the concurrent test: a `decision_signal`
/// is justified against events the server *already holds* with a lower
/// `session_seq`, so racing batches could legitimately refuse one another and
/// the test would be measuring the vocabulary rule rather than idempotency.
fn file_changed(session: Uuid, seq: u64, path: &str) -> Value {
    event(
        session,
        seq,
        "file_changed",
        json!({ "File": {
            "repo_file": path,
            "repo_file_from": null,
            "change_kind": "modified",
            "file_identity": "present"
        }}),
    )
}

fn decision_signal(session: Uuid, seq: u64, subject: &str, object: &str, justified: u64) -> Value {
    event(
        session,
        seq,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": "adopt",
            "subject_token": subject,
            "object_token": object,
            "justified_by_seq": justified,
            "lexicon_version": 1
        }}),
    )
}

fn batch(events: &[Value]) -> Value {
    json!({ "contract_version": 1, "events": events })
}

fn post(pg: &Pg, who: &Account, events: &[Value]) -> (Value, u16) {
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

/// `count` events for one session, each naming a distinct file.
fn many_events(session: Uuid, label: &str, count: u64) -> Vec<Value> {
    (1..=count)
        .map(|seq| file_changed(session, seq, &format!("crates/{label}/file{seq}.rs")))
        .collect()
}

/// Four events for one session — the size of an ordinary spooled batch.
fn four_events(session: Uuid, label: &str) -> Vec<Value> {
    many_events(session, label, 4)
}

fn canonical_events(pg: &Pg, session: Uuid) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
    ))
}

fn consolidation_inputs(pg: &Pg, session: Uuid) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_work WHERE session_id = '{session}'"
    ))
}

// ---------------------------------------------------------------------------
// SC-716 — one canonical event per distinct event, in 100% of trials
// ---------------------------------------------------------------------------

#[test]
fn ten_replays_of_one_batch_yield_exactly_one_canonical_event_per_distinct_event() {
    let pg = pg!();

    // Ten trials, each its own session, each delivered ten times. A single
    // trial would show the second delivery being handled and say nothing about
    // the tenth; a single session across all ten would let one trial's rows
    // satisfy the next one's assertion.
    for trial in 0..TRIALS {
        let session = pg.session_for(&pg.owner);
        let events = four_events(session, &format!("trial{trial}"));

        for attempt in 0..DELIVERIES {
            let (body, status) = post(&pg, &pg.owner, &events);
            assert_eq!(status, 200, "trial {trial}, delivery {attempt}: {body}");

            // The first delivery accepts every item; every later one answers
            // `duplicate` for every item. `duplicate` is a **success** — a
            // retry that gets it has achieved exactly what it was for — and a
            // drain treats it as delivered. If a replay were answered
            // `accepted` here, a second canonical event had just been made.
            let want = if attempt == 0 {
                "accepted"
            } else {
                "duplicate"
            };
            let got = statuses(&body);
            assert_eq!(
                got,
                vec![want; events.len()],
                "trial {trial}, delivery {attempt}: expected every item {want}"
            );
        }

        assert_eq!(
            canonical_events(&pg, session),
            events.len() as i64,
            "trial {trial}: ten deliveries of four distinct events produced more than four \
             canonical events"
        );

        // The consolidation queue is the input side of the same property. Its
        // row is written in the *same transaction* as the event insert and only
        // when that insert actually inserted, so a duplicate event can never
        // reach the extractor at all. A second row here would be a second
        // consolidation input for one event even though only one event exists.
        assert_eq!(
            consolidation_inputs(&pg, session),
            events.len() as i64,
            "trial {trial}: a replay enqueued the same event for consolidation twice"
        );
    }

    // And nothing leaked across trials: the project holds exactly what the ten
    // trials distinctly sent.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE project_id = '{}'",
            pg.project
        )),
        (TRIALS * 4) as i64
    );
}

#[test]
fn concurrent_replays_of_one_batch_are_accepted_exactly_once_each() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let events = many_events(session, "concurrent", RACE_EVENTS);
    let body = batch(&events);

    // Eight simultaneous deliveries of one batch — the shape a daemon produces
    // when a retry timer fires while the original request is still in flight.
    // The insert is `ON CONFLICT (event_id) DO NOTHING` inside the transaction
    // that also enqueues the consolidation work, so PostgreSQL's primary key is
    // the arbiter: exactly one writer inserts and the rest see zero rows
    // affected. Taken *outside* a transaction — read, then insert — every racer
    // would read "absent", every racer would insert, and the losers would be
    // the ones that happened to lose a unique-violation retry rather than the
    // ones that were told `duplicate`.
    //
    // Every answer must be a per-item outcome, because `duplicate` is the whole
    // point of the retry: a request-level error tells the daemon nothing about
    // which of its events landed, and FR-771 exists so it never has to guess.
    // `safe_events` carries **two** unique constraints that say the same thing
    // — `event_id` and `UNIQUE (session_id, session_seq)`, identical because
    // the id is derived from the pair — and a conflict target names only one of
    // them, so a racer whose arbiter check misses the window still meets the
    // other index. That is what a failure here looks like, and it is the reason
    // this test races rather than merely repeating.
    let base = pg.server.base.clone();
    let token = pg.owner.token.clone();
    let mut handles = Vec::new();
    for _ in 0..RACERS {
        let (base, token, body) = (base.clone(), token.clone(), body.clone());
        handles.push(std::thread::spawn(move || {
            post_json_status_bearer(&base, "/api/events/batch", &body, &token)
        }));
    }
    let answers: Vec<(Value, u16)> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    for (body, status) in &answers {
        assert_eq!(*status, 200, "a concurrent delivery failed: {body}");
    }
    let all: Vec<String> = answers.iter().flat_map(|(b, _)| statuses(b)).collect();
    let accepted = all.iter().filter(|s| *s == "accepted").count();
    let duplicate = all.iter().filter(|s| *s == "duplicate").count();

    // Exactly one caller is told `accepted` per distinct event. This is the
    // assertion the row count alone cannot make: a system that inserted twice
    // and then deduplicated on read would still count four rows, but it would
    // have answered `accepted` eight times.
    assert_eq!(
        accepted,
        events.len(),
        "more than one concurrent delivery was told it had created each event: {all:?}"
    );
    assert_eq!(
        duplicate,
        events.len() * (RACERS - 1),
        "a concurrent delivery got neither `accepted` nor `duplicate`: {all:?}"
    );
    assert_eq!(canonical_events(&pg, session), events.len() as i64);
    assert_eq!(consolidation_inputs(&pg, session), events.len() as i64);
}

#[test]
fn an_overlapping_replay_neither_drops_the_new_work_nor_doubles_the_old() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // The shape a spool produces when a response is lost mid-window: the daemon
    // re-sends what it could not confirm, and by then it has more to send. So
    // the second batch is partly a replay and partly new.
    let a: Vec<Value> = (1..=5u64)
        .map(|seq| file_changed(session, seq, &format!("crates/overlap/file{seq}.rs")))
        .collect();
    let b: Vec<Value> = (3..=8u64)
        .map(|seq| file_changed(session, seq, &format!("crates/overlap/file{seq}.rs")))
        .collect();

    let (body, status) = post(&pg, &pg.owner, &a);
    assert_eq!(status, 200, "{body}");
    assert_eq!(statuses(&body), vec!["accepted"; 5]);

    let (body, status) = post(&pg, &pg.owner, &b);
    assert_eq!(status, 200, "{body}");
    // Per-item outcomes, in the order sent, so the client can retry precisely
    // what needs retrying (FR-771). Three duplicates then three acceptances.
    // The two ways to get this wrong are equal and opposite: a batch refused
    // wholesale because it contained a duplicate loses events 6, 7 and 8
    // forever, and a batch that re-accepted 3, 4 and 5 doubles them.
    assert_eq!(
        statuses(&body),
        vec![
            "duplicate",
            "duplicate",
            "duplicate",
            "accepted",
            "accepted",
            "accepted"
        ],
        "an overlapping batch was not answered item by item: {body}"
    );

    assert_eq!(
        canonical_events(&pg, session),
        8,
        "five events plus six events overlapping in three is eight canonical events — \
         thirteen means the duplicates were doubled, ten means the new work was dropped"
    );
    assert_eq!(consolidation_inputs(&pg, session), 8);

    // And replaying both, repeatedly, still leaves eight.
    for _ in 0..DELIVERIES {
        for events in [&a, &b] {
            let (body, status) = post(&pg, &pg.owner, events);
            assert_eq!(status, 200, "{body}");
            assert_eq!(statuses(&body), vec!["duplicate"; events.len()]);
        }
    }
    assert_eq!(canonical_events(&pg, session), 8);
    assert_eq!(consolidation_inputs(&pg, session), 8);
}

// ---------------------------------------------------------------------------
// FR-786 — one consolidation input, and one durable piece of knowledge
// ---------------------------------------------------------------------------

#[test]
fn a_batch_replayed_after_it_was_consolidated_produces_no_second_input_and_no_second_record() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // The smallest sequence the baseline extractor turns into knowledge: a file
    // change that establishes the tokens, and a decision citing them (R7,
    // `contracts/extraction.md` §13.5). Two events, one candidate, one memory —
    // small enough that "one" is unambiguous.
    let events = vec![
        file_changed(session, 1, "core/parser.rs"),
        decision_signal(session, 2, "core", "parser", 1),
    ];
    let (body, status) = post(&pg, &pg.owner, &events);
    assert_eq!(status, 200, "{body}");
    assert_eq!(statuses(&body), vec!["accepted"; 2]);

    close_session(&pg, session);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the session's consolidation pass finishes", || {
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_session
              WHERE session_id = '{session}' AND state = 'done'"
        )) == 1
    });

    let candidate = cairn_core::eventid::candidate_id(
        pg.project,
        session,
        Some("decision.core"),
        Some("parser"),
    );
    let memory_count = || {
        pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND topic_key = 'decision.core'
                AND value_key = 'parser' AND origin_kind = 'consolidated'",
            pg.project
        ))
    };
    let candidate_count = || {
        pg.server.count(&format!(
            "SELECT count(*) FROM knowledge_candidates WHERE candidate_id = '{candidate}'"
        ))
    };
    let source_links = || {
        pg.server.count(&format!(
            "SELECT count(*) FROM candidate_source_events WHERE candidate_id = '{candidate}'"
        ))
    };
    // Established before the replay, so the "unchanged" assertions below are
    // comparing against something rather than against zero. A fixture that
    // produced no candidate could not tell a duplicated one from an absent one.
    assert_eq!(candidate_count(), 1, "the fixture produced no candidate");
    assert_eq!(memory_count(), 1, "the fixture produced no durable record");
    // How many events the candidate cites is the extractor's business, not
    // this test's — R7 cites the decision alone, R1 cites three. What the
    // replay must not do is change the number, so it is read rather than
    // predicted.
    let sources_before = source_links();
    assert!(sources_before > 0, "the candidate cites no events at all");
    let runs_before = pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_runs WHERE session_id = '{session}'"
    ));

    // Now the daemon replays the batch it never got an answer for. This is the
    // sharpest case in the file: the events have already become knowledge, so a
    // re-accepted event would be re-enqueued, the finished session would be
    // re-opened as `pending`, a second pass would run, and the extractor would
    // be handed the same evidence a second time. `persist` answers `duplicate`
    // and returns *before* touching either consolidation table, which is why
    // none of that happens.
    for attempt in 0..DELIVERIES {
        let (body, status) = post(&pg, &pg.owner, &events);
        assert_eq!(status, 200, "delivery {attempt}: {body}");
        assert_eq!(statuses(&body), vec!["duplicate"; 2]);
    }

    // Give the worker every chance to elect the session again before claiming
    // it did not.
    std::thread::sleep(QUIET);

    assert_eq!(canonical_events(&pg, session), 2);
    assert_eq!(
        consolidation_inputs(&pg, session),
        2,
        "a replay re-enqueued events that had already been consolidated"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work
              WHERE session_id = '{session}' AND state = 'done'"
        )),
        2,
        "a replay reset a finished consolidation input back to pending"
    );
    assert_eq!(
        pg.server.text(&format!(
            "SELECT state FROM consolidation_session WHERE session_id = '{session}'"
        )),
        "done",
        "a replay re-opened a session whose consolidation had finished — the re-open exists \
         for genuinely new events, and a duplicate is not one"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_runs WHERE session_id = '{session}'"
        )),
        runs_before,
        "a replay provoked a second consolidation pass over the same events"
    );
    assert_eq!(candidate_count(), 1, "a replay produced a second candidate");
    assert_eq!(
        source_links(),
        sources_before,
        "a replay changed how many events the candidate cites"
    );
    assert_eq!(
        memory_count(),
        1,
        "a replay produced a second durable record of one piece of knowledge"
    );
}

// ---------------------------------------------------------------------------
// One durable effect for a replayed command
// ---------------------------------------------------------------------------

/// The envelope the daemon posts for a queued command.
fn envelope(kind: &str, command_id: Uuid, project_id: Option<Uuid>, payload: Value) -> Value {
    json!({
        "command_id": command_id,
        "kind": kind,
        "project_id": project_id,
        "target_id": Value::Null,
        "payload": payload,
    })
}

fn deliver(pg: &Pg, who: &Account, body: &Value) -> (Value, u16) {
    post_json_status_bearer(&pg.server.base, "/api/commands", body, &who.token)
}

fn remember(pg: &Pg, command_id: Uuid, content: &str) -> Value {
    envelope(
        "remember",
        command_id,
        Some(pg.project),
        json!({ "type": "decision", "scope": "project", "content": content }),
    )
}

#[test]
fn ten_deliveries_of_one_remember_leave_one_memory_one_reservation_and_one_id() {
    let pg = pg!();
    let command_id = Uuid::now_v7();
    let body = remember(&pg, command_id, "the claim the daemon queued once");

    let (first, status) = deliver(&pg, &pg.owner, &body);
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["applied"], "accepted");
    let id = first["id"].clone();
    assert!(id.is_string(), "the first delivery returned no id: {first}");

    for attempt in 1..DELIVERIES {
        let (again, status) = deliver(&pg, &pg.owner, &body);
        assert_eq!(status, 200, "delivery {attempt}: {again}");
        // `duplicate` and not `404`. A replayed command answered as a failure
        // would tell the daemon its instruction never landed, and the daemon's
        // only correct response to that is to retry forever.
        assert_eq!(
            again["applied"], "duplicate",
            "delivery {attempt} was not recognised as a replay: {again}"
        );
        // The same id every time. A duplicate that answered with a *different*
        // id would be reporting a record the caller never created — and the
        // caller would go on to reference it.
        assert_eq!(
            again["id"], id,
            "delivery {attempt} named a different record than the one it created: {again}"
        );
    }

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            pg.project
        )),
        1,
        "ten deliveries of one command produced more than one memory"
    );
    // The reservation is keyed `(account_id, command_id)` and taken inside the
    // effect's transaction. Exactly one row is what makes every later delivery
    // a lookup rather than a write.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM applied_commands
              WHERE account_id = '{}' AND command_id = '{command_id}'",
            pg.owner.id
        )),
        1
    );
}

#[test]
fn concurrent_deliveries_of_one_remember_create_exactly_one_memory() {
    let pg = pg!();
    let command_id = Uuid::now_v7();
    let body = remember(&pg, command_id, "the claim two racing deliveries share");

    // `feature005_command_delivery.rs` races a `reinforce`, whose effect is a
    // counter. This races a **create**, whose effect is a row: a check-then-act
    // gate leaves the counter reading two, but it leaves the create with two
    // records under two different ids, only one of which any caller was told
    // about. The second is unreachable knowledge that still shows up in every
    // retrieval.
    let base = pg.server.base.clone();
    let token = pg.owner.token.clone();
    let mut handles = Vec::new();
    for _ in 0..RACERS {
        let (base, token, body) = (base.clone(), token.clone(), body.clone());
        handles.push(std::thread::spawn(move || {
            post_json_status_bearer(&base, "/api/commands", &body, &token)
        }));
    }
    let answers: Vec<(Value, u16)> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    for (body, status) in &answers {
        assert_eq!(*status, 200, "a concurrent delivery failed: {body}");
    }

    let applied: Vec<String> = answers
        .iter()
        .map(|(b, _)| b["applied"].as_str().unwrap_or("?").to_string())
        .collect();
    assert_eq!(
        applied.iter().filter(|a| *a == "accepted").count(),
        1,
        "more than one concurrent delivery believed it had created the memory: {applied:?}"
    );
    // Every answer names the same record, including the seven that lost.
    let ids: std::collections::BTreeSet<String> = answers
        .iter()
        .filter_map(|(b, _)| b["id"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        ids.len(),
        1,
        "concurrent deliveries of one command named different records: {ids:?}"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            pg.project
        )),
        1
    );
}

#[test]
fn a_command_whose_effect_succeeded_is_never_replayable_even_across_a_server_crash() {
    let mut pg = pg!();
    let command_id = Uuid::now_v7();
    let body = remember(&pg, command_id, "a claim that outlives the process");

    let (first, status) = deliver(&pg, &pg.owner, &body);
    assert_eq!(status, 200, "{first}");
    let id = first["id"].clone();

    // The complement of `a_rolled_back_effect_leaves_the_command_replayable`:
    // there, a failed effect must *not* burn its command id. Here, a succeeded
    // one must burn it permanently — and permanence has to survive the process,
    // because the outage FR-786 is about is exactly the case where the daemon
    // is still holding an unanswered command when the server goes away. The
    // reservation is a committed row, not worker state, so a SIGKILL and a
    // respawn on the same database change nothing.
    pg.crash_and_restart();

    for attempt in 0..DELIVERIES {
        let (again, status) = deliver(&pg, &pg.owner, &body);
        assert_eq!(status, 200, "post-restart delivery {attempt}: {again}");
        assert_eq!(
            again["applied"], "duplicate",
            "a restarted server re-applied a command it had already carried out: {again}"
        );
        assert_eq!(again["id"], id);
    }

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories WHERE project_id = '{}'",
            pg.project
        )),
        1,
        "a command replayed after a crash produced a second memory"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM applied_commands WHERE command_id = '{command_id}'"
        )),
        1
    );
}

// ---------------------------------------------------------------------------
// The US2 transmission outcome under retry
// ---------------------------------------------------------------------------

#[test]
fn retrying_a_retrieval_never_fabricates_a_transmission_it_was_never_told_about() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    pg.server.execute(&format!(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, origin_session_id,
             topic_key, value_key, origin_kind)
         VALUES ('{}', '{}', 'fact', 'project', '{}', 'a fact worth delivering', 'active',
                 '{session}', 'topic.retry', 'settled', 'explicit')",
        Uuid::now_v7(),
        pg.project,
        pg.project
    ));

    // A retrieval that is retried — the same lost-response shape, on the read
    // side. Unlike ingest this one is *not* idempotent by design: a retrieval
    // is an event in its own right and each attempt gets its own trace, which
    // is what makes SC-729's "every retrieval is recorded" true. So the
    // property under test is not "one trace" but the one that actually
    // protects the user: no attempt, and no number of attempts, may move a
    // trace past `generated` on its own.
    let mut traces = Vec::new();
    for attempt in 0..DELIVERIES {
        let (body, status) = post_json_status_bearer(
            &pg.server.base,
            "/api/retrieve",
            &json!({ "session_id": session, "trigger": "session_open" }),
            &pg.owner.token,
        );
        assert_eq!(status, 200, "retrieval {attempt}: {body}");
        traces.push(body["trace_id"].as_str().expect("a trace_id").to_string());
    }

    for trace in &traces {
        assert_eq!(
            pg.server.text(&format!(
                "SELECT delivery_state FROM retrieval_traces WHERE trace_id = '{trace}'"
            )),
            "generated",
            "a retrieval nobody reported an outcome for claimed one anyway"
        );
    }
    // Generating a briefing is not evidence that an agent received one
    // (Principle X, FR-843). A `delivered_context` row written here would make
    // dedup withhold, for the life of the session, an item the agent never
    // saw — the dedup enforcing a delivery that never happened. Ten retries is
    // ten chances for a well-meaning "we already sent this" to appear.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM delivered_context WHERE session_id = '{session}'"
        )),
        0,
        "retrying a retrieval wrote delivery rows without any outcome report"
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM retrieval_traces
              WHERE session_id = '{session}' AND delivery_state <> 'generated'"
        )),
        0
    );
}

//! Semantic extraction, as a contract (T042, `contracts/extraction.md` §3,
//! §4, §6, §7, §13.5).
//!
//! Every test here drives the **real** pipeline end to end: events go in
//! through `POST /api/events/batch`, a real `cairn-server` process (started
//! with a pool big enough to earn a consolidation share) claims and
//! consolidates a session, and the assertions read `knowledge_candidates`,
//! `memories`, `memory_relations` and `candidate_source_events` — never the
//! extractor's Rust types directly. `extract.rs` already has unit tests for
//! the rules in isolation; what is worth a slower, real-process test is
//! whether the *whole pipeline* — ingest's vocabulary gate, the claim
//! machinery, `consolidate.rs`'s ten governance gates and the baseline
//! extractor together — produces the rows the contract promises, from
//! nothing but HTTP requests a client could actually send.
//!
//! # Why events for three sessions are posted before any session is closed
//!
//! R3, R5, R6 and R8 are project rules: `aggregate()` reads every session's
//! `safe_events` for the project, not only the session a pass claims
//! (`consolidate.rs`'s `read_project_events`). So a project rule can fire
//! from one pass over one session, as long as the *other* sessions' evidence
//! already sits in `safe_events` when that pass runs — which is exactly what
//! posting every session's events first, and closing only one, sets up.

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

/// How long a property is given to become true before the test calls it
/// false. Generous against the worker's own ~100ms poll.
const SETTLE: Duration = Duration::from_secs(30);

/// The pool a worker asks for — the smallest that earns any consolidation
/// share at all (`pool_share(5) == 1`; `contracts/consolidation.md` §6,
/// FR-793a1). The fixture's own server is deliberately started with a
/// smaller pool and never consolidates, so every test here stands up its own
/// worker against the same database.
const WORKER_POOL: &str = "5";

// ---------------------------------------------------------------------------
// A second server on the fixture's database — the worker under test
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

/// Close a session on the server's own row — the "session has closed"
/// eligibility trigger (`contracts/consolidation.md` §3), and the cheapest
/// one to establish in a test.
fn close_session(pg: &Pg, session: Uuid) {
    pg.server.execute(&format!(
        "UPDATE sessions SET status = 'completed', ended_at = now() WHERE id = '{session}'"
    ));
}

fn session_done(pg: &Pg, session: Uuid) -> bool {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_session
          WHERE session_id = '{session}' AND state = 'done'"
    )) == 1
}

fn run_count(pg: &Pg, session: Uuid) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_runs WHERE session_id = '{session}'"
    ))
}

// ---------------------------------------------------------------------------
// Event construction
// ---------------------------------------------------------------------------

fn event_id(session: Uuid, seq: u64) -> Uuid {
    cairn_core::eventid::event_id(session, seq)
}

fn event(session: Uuid, seq: u64, agent: &str, kind: &str, content: Value) -> Value {
    json!({
        "event_id": event_id(session, seq),
        "contract_version": 1,
        "kind": kind,
        "agent": agent,
        "vendor_event": "PostToolUse",
        "session_id": session,
        "session_seq": seq,
        "occurred_at": "2026-09-02T10:00:00Z",
        "content": content,
    })
}

fn file_changed(session: Uuid, seq: u64, agent: &str, path: &str) -> Value {
    event(
        session,
        seq,
        agent,
        "file_changed",
        json!({ "File": {
            "repo_file": path,
            "repo_file_from": null,
            "change_kind": "modified",
            "file_identity": "present"
        }}),
    )
}

fn command_executed(session: Uuid, seq: u64, agent: &str, line: &str, exit: i32) -> Value {
    event(
        session,
        seq,
        agent,
        "command_executed",
        json!({ "Command": { "command_line": line, "exit_status": exit }}),
    )
}

fn test_executed(session: Uuid, seq: u64, agent: &str, command: &str) -> Value {
    event(
        session,
        seq,
        agent,
        "test_executed",
        json!({ "TestInvocation": { "test_command": command }}),
    )
}

fn test_result(session: Uuid, seq: u64, agent: &str, outcome: &str, exit: i32) -> Value {
    event(
        session,
        seq,
        agent,
        "test_result",
        json!({ "TestVerdict": {
            "test_outcome": outcome,
            "exit_status": exit,
            "tests_total": null,
            "tests_failed": null
        }}),
    )
}

fn tool_failed(session: Uuid, seq: u64, agent: &str, tool: &str) -> Value {
    event(
        session,
        seq,
        agent,
        "tool_failed",
        json!({ "ToolFailure": {
            "vendor_tool": tool,
            "tool_class": "execute",
            "failure_kind": "non_zero_exit",
            "failure_note": null,
            "exit_status": 1
        }}),
    )
}

fn decision_signal(
    session: Uuid,
    seq: u64,
    agent: &str,
    kind: &str,
    subject: &str,
    object: &str,
    justified_by_seq: Option<u64>,
) -> Value {
    event(
        session,
        seq,
        agent,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": kind,
            "subject_token": subject,
            "object_token": object,
            "justified_by_seq": justified_by_seq,
            "lexicon_version": 1
        }}),
    )
}

fn instruction_signal(
    session: Uuid,
    seq: u64,
    agent: &str,
    kind: &str,
    subject: &str,
    object: &str,
    justified_by_seq: Option<u64>,
) -> Value {
    event(
        session,
        seq,
        agent,
        "user_instruction_signal",
        json!({ "Instruction": {
            "instruction_kind": kind,
            "subject_token": subject,
            "object_token": object,
            "justified_by_seq": justified_by_seq,
            "lexicon_version": 1
        }}),
    )
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

fn assert_all_accepted(label: &str, body: &Value, status: u16) {
    assert_eq!(status, 200, "{label}: {body}");
    let got = statuses(body);
    assert!(
        got.iter().all(|s| s == "accepted"),
        "{label}: not every event was accepted: {got:?} ({body})"
    );
}

// ---------------------------------------------------------------------------
// Reading what consolidation decided
// ---------------------------------------------------------------------------

fn memory_count(pg: &Pg, topic: &str, value: &str, origin_kind: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM memories
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND origin_kind = '{origin_kind}'",
        pg.project
    ))
}

fn memory_type(pg: &Pg, topic: &str, value: &str) -> String {
    pg.server.text(&format!(
        "SELECT type FROM memories
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND origin_kind = 'consolidated'",
        pg.project
    ))
}

// ---------------------------------------------------------------------------
// R1-R8, one project, one closed session, three sessions' worth of evidence
// ---------------------------------------------------------------------------

#[test]
fn each_of_the_eight_extraction_rules_fires_on_the_event_sequence_its_contract_section_states() {
    // `contracts/extraction.md` §4 (R1, R2, R3, R4, R5, R6) and §13.5 (R7, R8).
    let pg = pg!();

    let a = pg.session_for(&pg.owner); // the session that gets closed and claimed
    let b = pg.session_for(&pg.owner); // supplies a second session's worth of R3/R5/R6/R8 evidence
    let c = pg.session_for(&pg.owner); // supplies a third session for R3

    // --- Session A -----------------------------------------------------
    // 1: establishes the "core"/"parser" tokens R7's decision_signal cites.
    // 2: R7 — a recorded decision, topic `decision.<subject>`, value `<object>`.
    // 3,4: two file_changed in `api/`, after the decision — R4's evidence.
    // 5: names the suite for R1, and is R6's first (of two) consistent invocation.
    // 6,7,8: test failed -> a file changed -> test passed — R1.
    // 9,10,11: three `tool_failed`, same kind, no later success for that tool — R2.
    // 12,13: the two successful commands whose *sequence* R5 repeats in session B,
    //        and whose *second* member (`cargo build`) R3 also sees in B and C.
    // 14: a standing instruction, R8's first (of two) session.
    let a_events = vec![
        file_changed(a, 1, "claude_code", "core/parser.rs"),
        decision_signal(a, 2, "claude_code", "adopt", "core", "parser", Some(1)),
        file_changed(a, 3, "claude_code", "api/routes.rs"),
        file_changed(a, 4, "claude_code", "api/handlers.rs"),
        test_executed(a, 5, "claude_code", "cargo test -p cairn-core"),
        test_result(a, 6, "claude_code", "failed", 101),
        file_changed(a, 7, "claude_code", "core/lexer.rs"),
        test_result(a, 8, "claude_code", "passed", 0),
        tool_failed(a, 9, "claude_code", "Bash"),
        tool_failed(a, 10, "claude_code", "Bash"),
        tool_failed(a, 11, "claude_code", "Bash"),
        command_executed(a, 12, "claude_code", "cargo fmt", 0),
        command_executed(a, 13, "claude_code", "cargo build", 0),
        instruction_signal(a, 14, "claude_code", "require", "core", "parser", Some(1)),
    ];
    let (body, status) = post(&pg, &pg.owner, a_events);
    assert_all_accepted("session A", &body, status);

    // --- Session B -------------------------------------------------------
    // Its own vocabulary evidence, then the same standing instruction (R8's
    // second session), the same two-command sequence in the same order
    // (R5's second session) and a matching test invocation (R6's second).
    let b_events = vec![
        file_changed(b, 1, "codex", "core/parser.rs"),
        instruction_signal(b, 2, "codex", "require", "core", "parser", Some(1)),
        command_executed(b, 3, "codex", "cargo fmt", 0),
        command_executed(b, 4, "codex", "cargo build", 0),
        test_executed(b, 5, "codex", "cargo test -p cairn-core"),
    ];
    let (body, status) = post(&pg, &pg.owner, b_events);
    assert_all_accepted("session B", &body, status);

    // --- Session C ---------------------------------------------------
    // Just enough to be R3's third session for `cargo build`.
    let c_events = vec![command_executed(c, 1, "claude_code", "cargo build", 0)];
    let (body, status) = post(&pg, &pg.owner, c_events);
    assert_all_accepted("session C", &body, status);

    // Only A becomes eligible. B and C's events are read by the project
    // aggregator when A's pass runs; neither session is itself consolidated.
    close_session(&pg, a);

    let _worker = Worker::start(&pg.server.database_url);
    settle("session A's pass finishes", || session_done(&pg, a));
    assert_eq!(run_count(&pg, a), 1, "one closed session, one pass");

    // R1 — fix confirmed by tests.
    assert_eq!(
        memory_type(&pg, "test.cairn_core", "fixed_by.lexer"),
        "failure"
    );
    assert_eq!(
        memory_count(&pg, "test.cairn_core", "fixed_by.lexer", "consolidated"),
        1
    );

    // R2 — persistent failure.
    assert_eq!(
        memory_type(&pg, "failure.non_zero_exit", "unresolved"),
        "failure"
    );
    assert_eq!(
        memory_count(&pg, "failure.non_zero_exit", "unresolved", "consolidated"),
        1
    );

    // R4 — decision near change. `api` dominates the files changed after the
    // decision_signal (two, against `core`'s one from step 7).
    let r4_value = format!("changed.{}", a.simple());
    assert_eq!(memory_type(&pg, "area.api", &r4_value), "decision");
    assert_eq!(memory_count(&pg, "area.api", &r4_value, "consolidated"), 1);

    // R7 — recorded decision, namespaced under `decision.`.
    assert_eq!(memory_type(&pg, "decision.core", "parser"), "decision");
    assert_eq!(
        memory_count(&pg, "decision.core", "parser", "consolidated"),
        1
    );

    // R8 — standing instruction, observed in sessions A and B, namespaced
    // under `instruction.` so it cannot collide with R7's `decision.core`.
    assert_eq!(memory_type(&pg, "instruction.core", "parser"), "convention");
    assert_eq!(
        memory_count(&pg, "instruction.core", "parser", "consolidated"),
        1
    );

    // R3 — established command, seen with exit 0 in three sessions (A, B, C).
    // Value key is a bounded digest, never the command text (§4).
    let r3_value: String = pg.server.text(&format!(
        "SELECT value_key FROM memories
          WHERE project_id = '{}' AND topic_key = 'command.cargo' AND origin_kind = 'consolidated'",
        pg.project
    ));
    assert_eq!(memory_type(&pg, "command.cargo", &r3_value), "convention");
    assert!(
        r3_value.chars().count() <= 64,
        "R3 value key exceeds VALUE_KEY_MAX_CHARS: {r3_value}"
    );
    assert!(
        r3_value.chars().all(|c| c.is_ascii_hexdigit()),
        "R3 value key is not a digest: {r3_value}"
    );
    assert!(
        !r3_value.contains("cargo") && !r3_value.contains("build"),
        "R3 value key carries the command text instead of a digest: {r3_value}"
    );

    // R5 — repeated procedure, the ordered sequence `cargo fmt -> cargo build`
    // in sessions A and B. Same digest-bound treatment as R3.
    let r5_value: String = pg.server.text(&format!(
        "SELECT value_key FROM memories
          WHERE project_id = '{}' AND topic_key = 'procedure.cargo' AND origin_kind = 'consolidated'",
        pg.project
    ));
    assert_eq!(memory_type(&pg, "procedure.cargo", &r5_value), "procedure");
    assert!(r5_value.chars().count() <= 64);
    assert!(r5_value.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!r5_value.contains("cargo") && !r5_value.contains("fmt"));
    assert_ne!(
        r5_value, r3_value,
        "R3's and R5's digests collided (different evidence, same key)"
    );

    // R6 — test suite identity, a consistent `test_command` in sessions A and B.
    let r6_value: String = pg.server.text(&format!(
        "SELECT value_key FROM memories
          WHERE project_id = '{}' AND topic_key = 'test.command' AND origin_kind = 'consolidated'",
        pg.project
    ));
    assert_eq!(memory_type(&pg, "test.command", &r6_value), "fact");
    assert!(r6_value.chars().count() <= 64);
    assert!(r6_value.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!r6_value.contains("cargo"));

    // All five knowledge kinds reachable in one pass: fact (R6), decision
    // (R4, R7), convention (R3, R8), failure (R1, R2), procedure (R5).
    let kinds: Vec<String> = pg.server.query_column(&format!(
        "SELECT DISTINCT type FROM memories
          WHERE project_id = '{}' AND origin_kind = 'consolidated' ORDER BY type",
        pg.project
    ));
    let mut expected_kinds: Vec<String> =
        ["convention", "decision", "fact", "failure", "procedure"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    expected_kinds.sort();
    assert_eq!(
        kinds, expected_kinds,
        "not all five knowledge kinds were produced: {kinds:?}"
    );

    // Source verification (gate 1, FR-805c): every event a candidate from
    // this run cites really exists, in this project, and was accepted.
    let uncited: i64 = pg.server.count(&format!(
        "SELECT count(*) FROM candidate_source_events cse
           JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
          WHERE kc.project_id = '{}'
            AND NOT EXISTS (
                  SELECT 1 FROM safe_events se
                   WHERE se.event_id = cse.event_id AND se.project_id = '{}')",
        pg.project, pg.project
    ));
    assert_eq!(
        uncited, 0,
        "a candidate cited an event that does not exist in this project"
    );
}

// ---------------------------------------------------------------------------
// The honest limit: no pattern, no candidate
// ---------------------------------------------------------------------------

#[test]
fn a_session_with_no_rule_matching_pattern_produces_zero_candidates() {
    // `contracts/extraction.md` §4.1 — R1-R8 do not infer intent; a sequence
    // that matches none of them proposes nothing, and a run with zero
    // candidates is still a completed, recorded run (`consolidation.md` §8).
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    let events = vec![
        file_changed(session, 1, "claude_code", "misc/note.rs"),
        event(
            session,
            2,
            "claude_code",
            "tool_succeeded",
            json!({ "Tool": { "vendor_tool": "Bash", "tool_class": "execute" }}),
        ),
    ];
    let (body, status) = post(&pg, &pg.owner, events);
    assert_all_accepted("no-pattern session", &body, status);

    close_session(&pg, session);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the pass finishes", || session_done(&pg, session));

    let run_int = |column: &str| -> i64 {
        pg.server
            .text(&format!(
                "SELECT {column}::text FROM consolidation_runs WHERE session_id = '{session}'"
            ))
            .parse()
            .unwrap_or_else(|_| panic!("{column} was not an integer"))
    };
    let (proposed, accepted, refused) = (
        run_int("candidates_proposed"),
        run_int("candidates_accepted"),
        run_int("candidates_refused"),
    );
    assert_eq!(
        (proposed, accepted, refused),
        (0, 0, 0),
        "a run over an unremarkable session proposed something"
    );

    let candidates = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}'"
    ));
    assert_eq!(candidates, 0);
}

// ---------------------------------------------------------------------------
// Vendor invariance
// ---------------------------------------------------------------------------

#[test]
fn two_vendors_capturing_an_identical_fix_sequence_produce_the_same_topic_and_value_keys() {
    // `contracts/extraction.md` §4 R1, and the constitutional point behind
    // §3's rule-tier split: the extractor is a pure function of event
    // *structure*, so which vendor's hook produced the structure is not part
    // of the pattern R1 matches on.
    let pg = pg!();
    let x = pg.session_for(&pg.owner); // claude_code
    let y = pg.session_for(&pg.owner); // codex

    let fix = |session: Uuid, agent: &str| {
        vec![
            test_executed(session, 1, agent, "cargo test -p cairn-core"),
            test_result(session, 2, agent, "failed", 101),
            file_changed(session, 3, agent, "billing/invoice.rs"),
            test_result(session, 4, agent, "passed", 0),
        ]
    };
    let (body, status) = post(&pg, &pg.owner, fix(x, "claude_code"));
    assert_all_accepted("vendor x", &body, status);
    let (body, status) = post(&pg, &pg.owner, fix(y, "codex"));
    assert_all_accepted("vendor y", &body, status);

    close_session(&pg, x);
    close_session(&pg, y);
    let _worker = Worker::start(&pg.server.database_url);
    settle("both sessions' passes finish", || {
        session_done(&pg, x) && session_done(&pg, y)
    });

    // One durable record, keyed the same regardless of which session (and
    // therefore which vendor) produced it...
    assert_eq!(
        memory_count(&pg, "test.cairn_core", "fixed_by.invoice", "consolidated"),
        1
    );
    // ...and the second session's identical claim reinforces it rather than
    // creating a second record (`consolidation.md` §6, §7.1).
    assert_eq!(
        memory_count(&pg, "test.cairn_core", "fixed_by.invoice", "corroboration"),
        1
    );

    // The two knowledge_candidates rows (one per session, since candidate_id
    // includes session_id) agree on everything but which session proposed
    // them: same topic, same value, same content, same MemoryType.
    let rows: Vec<String> = pg.server.query_column(&format!(
        "SELECT topic_key || '|' || value_key || '|' || content || '|' || proposed_kind
           FROM knowledge_candidates
          WHERE project_id = '{}' AND topic_key = 'test.cairn_core' AND value_key = 'fixed_by.invoice'
          ORDER BY candidate_id",
        pg.project
    ));
    assert_eq!(
        rows.len(),
        2,
        "expected one candidate per vendor session: {rows:?}"
    );
    assert_eq!(
        rows[0], rows[1],
        "the two vendors' candidates disagreed on something other than session"
    );
}

// ---------------------------------------------------------------------------
// Deterministic identity
// ---------------------------------------------------------------------------

#[test]
fn re_consolidating_the_same_session_yields_the_same_candidate_id_and_no_second_record() {
    // `consolidation.md` §7: `candidate_id` is derived from project, session
    // and the normalized keys — not the event set — so re-executing a batch
    // over the same session upserts rather than duplicating.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    let events = vec![
        file_changed(session, 1, "claude_code", "core/parser.rs"),
        decision_signal(
            session,
            2,
            "claude_code",
            "adopt",
            "core",
            "parser",
            Some(1),
        ),
    ];
    let (body, status) = post(&pg, &pg.owner, events);
    assert_all_accepted("idempotency fixture", &body, status);

    close_session(&pg, session);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the first pass finishes", || session_done(&pg, session));
    assert_eq!(run_count(&pg, session), 1);

    let expected_id = cairn_core::eventid::candidate_id(
        pg.project,
        session,
        Some("decision.core"),
        Some("parser"),
    );
    let first_run_id = pg.server.text(&format!(
        "SELECT run_id::text FROM knowledge_candidates WHERE candidate_id = '{expected_id}'"
    ));
    assert_eq!(
        memory_count(&pg, "decision.core", "parser", "consolidated"),
        1
    );

    // Reopen the same generation over the same events — the faithful
    // reproduction of "the same events consolidated twice", since a real
    // reclaim is triggered by a lease expiring or a fresh election and not by
    // anything a client can ask for directly.
    pg.server.execute(&format!(
        "UPDATE consolidation_work SET state = 'pending', attempts = 0
          WHERE session_id = '{session}'"
    ));
    pg.server.execute(&format!(
        "UPDATE consolidation_session SET state = 'pending', claimed_by = NULL, claim_expires_at = NULL
          WHERE session_id = '{session}'"
    ));
    // `sessions.ended_at` is still set from `close_session`, so the session
    // is still eligible on the "closed" trigger alone (`consolidation.md` §3).
    settle("a second pass finishes", || run_count(&pg, session) >= 2);

    // Same candidate id, one row, still attributed to the first run.
    let candidate_rows = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates WHERE candidate_id = '{expected_id}'"
    ));
    assert_eq!(
        candidate_rows, 1,
        "a re-executed batch produced a second candidate row"
    );
    let run_id_after = pg.server.text(&format!(
        "SELECT run_id::text FROM knowledge_candidates WHERE candidate_id = '{expected_id}'"
    ));
    assert_eq!(
        run_id_after, first_run_id,
        "re-derivation changed which run owns the candidate's identity"
    );

    // No second durable record...
    assert_eq!(
        memory_count(&pg, "decision.core", "parser", "consolidated"),
        1
    );
    // ...and exactly one corroboration, not two (SC-703's shape: re-running
    // consolidation over unchanged events produces zero *additional* records
    // and zero additional reinforcement changes).
    assert_eq!(
        memory_count(&pg, "decision.core", "parser", "corroboration"),
        1
    );
    let reinforcements = pg.server.count(
        "SELECT count(*) FROM memory_relations mr
           JOIN memories m ON m.id = mr.from_memory_id
          WHERE mr.kind = 'reinforces' AND m.topic_key = 'decision.core' AND m.value_key = 'parser'",
    );
    assert_eq!(
        reinforcements, 1,
        "a second run recorded a second reinforcement relation"
    );
    let reinforcement_count: i64 = pg
        .server
        .text(&format!(
            "SELECT reinforcement_count::text FROM memories
              WHERE project_id = '{}' AND topic_key = 'decision.core' AND value_key = 'parser'
                AND origin_kind = 'consolidated'",
            pg.project
        ))
        .parse()
        .expect("an integer reinforcement count");
    assert_eq!(
        reinforcement_count, 1,
        "the reinforcement counter was bumped more than once"
    );
}

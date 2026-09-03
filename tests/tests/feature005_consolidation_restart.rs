//! Idempotency, paraphrase reinforcement and restart-safety, as a contract
//! (T044, `contracts/consolidation.md` §4, §4.1, §7, §7.1; `contracts/
//! extraction.md` §6, §7; SC-703, SC-736, SC-739).
//!
//! `feature005_consolidation_claims.rs` proves the claim machinery — leases,
//! attempts, ordering — with events that carry no real content, because it is
//! about the machinery and not about extraction. This file is the opposite
//! half: every event here is a real R1 "fix confirmed by tests" sequence
//! (`crates/cairn-server/src/extract.rs::rule_1_fix_confirmed_by_tests`), so
//! the deterministic extractor genuinely proposes, genuinely reinforces, and
//! genuinely gets interrupted mid-pipeline.
//!
//! # Why R1, specifically
//!
//! R7/R8 (`decision_signal`, `user_instruction_signal`) key off a
//! `VocabToken`, which must already be lower-case, dot/underscore-shaped and
//! justified by the session's own established vocabulary before ingest will
//! even accept it (`crates/cairn-core/src/event.rs::VocabToken::new`). That
//! makes them a poor vehicle for a *paraphrase* corpus: two différently-cased
//! or hyphenated wordings of the same token cannot both reach the wire in the
//! first place. R3/R5/R6 key their value on a digest of the exact command
//! text (`extract.rs::digest_token`), which is deliberately **not**
//! paraphrase-tolerant — two different wordings hash to two different values.
//! R1 is the one rule whose keys come from free text — a `test_command` and a
//! `repo_file` — folded through the same separator-folding
//! (`cairn_core::knowledge::normalize_topic_key`) that makes a paraphrase
//! corpus meaningful at all. So the paraphrase corpus
//! (`tests/feature005/corpora/paraphrase_pairs.json`) is written as wordings a
//! file stem or a command word could actually be (hyphen/case/slash/doubled-
//! separator variation; no bare space, since a space would split a command
//! line into two words), and the "representative subset...driven through the
//! real pipeline" below drives R1 with them.
//!
//! # Why this file starts its own `cairn-server` for every worker cycle
//!
//! `Pg::start`'s own server always runs with `--max-connections 4`
//! (`tests/src/lib.rs::Server::try_start_at`), and consolidation's pool share
//! is `min(2, floor(max_connections / 5))` — `floor(4/5) = 0`
//! (`contracts/consolidation.md` §6). A share of zero is a
//! `tokio::sync::Semaphore::new(0)` that `Consolidator::permit` can never
//! acquire, so `pg.server` **never attempts a single consolidation pass**,
//! restarted or not. `Pg::crash_and_restart` therefore cannot be the restart
//! injector for anything in this file — it restarts the wrong process. What
//! actually consolidates is a second `cairn-server`, started against the same
//! database with a pool large enough to earn a share
//! (`WORKER_POOL`), exactly as `feature005_consolidation_claims.rs` already
//! does. A "restart" here is: drop that process (killing it — see `Worker`'s
//! `Drop`) and start a fresh one at a fresh port on the *same database*. The
//! database, not the port, is the durable thing a restart must preserve, and
//! nothing in this file's assertions depends on the address staying fixed.
//!
//! # How a crash point is represented without racing a live worker
//!
//! A real process kill cannot be aimed at a millisecond-scale transaction
//! boundary, and it does not need to be: every consolidation pass commits in
//! at most three atomic steps — the claim transaction, the governance
//! transaction (extraction + persistence, one transaction — FR-808), and the
//! close transaction — and nothing is observable between them. So "crash
//! after governance committed, before close" is represented by **letting a
//! real pass run to completion once**, and then undoing only the three writes
//! the close transaction made (`consolidation_work` back to `pending`,
//! `consolidation_session` back to `claimed` with an expired lease,
//! `consolidation_runs` back to `running`) — leaving every row the governance
//! transaction wrote exactly as the real code left it. This is the same
//! technique `feature005_consolidation_claims.rs` already uses to represent
//! "a worker that died mid-pass": seeding the rows a crash would leave is the
//! faithful reproduction, because a crash is not observable as anything else.
//! No worker process runs while a scenario's rows are being arranged, so
//! nothing can race the seeding — see `Worker::start`/`drop` scoping in
//! `run_crash_point`.
//!
//! # A finding this file is expected to surface
//!
//! `consolidate.rs::govern`'s gate 6 looks up "does a project memory already
//! exist for this topic" (`project_by_topic`) with no exception for a memory
//! *this same session's own earlier attempt* durably created. A session that
//! crashes after its governance transaction commits a **new** (non-
//! reinforcing) memory, and is then reclaimed, will find its own prior
//! creation via that lookup and take the *reinforce* branch against it — a
//! spurious self-corroboration, an extra relation and an extra
//! `reinforcement_count`, none of which any second real occurrence produced.
//! `reinforce()` itself has no such gap (its endpoint is
//! `origin_kind = 'corroboration'`, which `project_by_topic` excludes by
//! name), so a *reinforcing* session's reclaim is safe. The tests below that
//! exercise a single session's own creation across a governance-committed
//! crash point are where this is expected to show up as a failure — see the
//! module-level report, not a defect in these assertions.

use cairn_core::eventid::event_id;
use cairn_core::knowledge::{normalize_topic_key, normalize_value_key};
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

/// How long a settle-loop waits before calling a property false.
///
/// Generous against the 100 ms yield the worker actually uses.
const SETTLE: Duration = Duration::from_secs(30);

/// The pool a test worker asks for — the smallest that earns a share at all
/// (`floor(5/5) = 1`), matching `feature005_consolidation_claims.rs`.
const WORKER_POOL: &str = "5";

// ---------------------------------------------------------------------------
// A second server on the fixture's database — the process that actually
// consolidates (see the module doc comment).
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

/// Run a real consolidation worker only for as long as `body` needs one, then
/// kill it. The restart injector this file uses: everywhere a scenario needs
/// "nobody is consolidating right now" it simply does not hold a `Worker`.
fn with_worker<R>(pg: &Pg, body: impl FnOnce() -> R) -> R {
    let _worker = Worker::start(&pg.server.database_url);
    body()
}

// ---------------------------------------------------------------------------
// Ingest — one real R1 "fix confirmed by tests" sequence
// ---------------------------------------------------------------------------

fn event(session: Uuid, seq: u64, kind: &str, content: Value) -> Value {
    let mut e = json!({
        "event_id": event_id(session, seq),
        "contract_version": 1,
        "kind": kind,
        "agent": "claude_code",
        "vendor_event": "PostToolUse",
        "session_id": session,
        "session_seq": seq,
        "occurred_at": "2026-09-02T10:00:00Z",
    });
    if !content.is_null() {
        e["content"] = content;
    }
    e
}

fn file_changed_event(session: Uuid, seq: u64, repo_file: &str) -> Value {
    event(
        session,
        seq,
        "file_changed",
        json!({ "File": {
            "repo_file": repo_file,
            "repo_file_from": null,
            "change_kind": "modified",
            "file_identity": "present"
        }}),
    )
}

fn test_executed_event(session: Uuid, seq: u64, test_command: &str) -> Value {
    event(
        session,
        seq,
        "test_executed",
        json!({ "TestInvocation": { "test_command": test_command } }),
    )
}

fn test_result_event(session: Uuid, seq: u64, outcome: &str) -> Value {
    event(
        session,
        seq,
        "test_result",
        json!({ "TestVerdict": {
            "test_outcome": outcome,
            "exit_status": null,
            "tests_total": null,
            "tests_failed": null
        }}),
    )
}

fn batch(events: Vec<Value>) -> Value {
    json!({ "contract_version": 1, "events": events })
}

fn post(pg: &Pg, who: &Account, body: &Value) {
    let (resp, status) =
        post_json_status_bearer(&pg.server.base, "/api/events/batch", body, &who.token);
    assert_eq!(status, 200, "ingest request failed: {resp}");
}

/// Post one complete R1 sequence — a failing suite, a change, and a pass —
/// into `session`, using gapped sequence numbers (10, 20, 30, 40) so a later
/// test can insert an "arrived meanwhile" event between the change and the
/// verdict without renumbering anything.
///
/// Returns the exact `(topic_key, value_key)` the real extractor will derive:
/// `suite_word`/`file_word` are embedded as a whole command word and a whole
/// file stem respectively, so what they fold through is exactly
/// `normalize_topic_key` — see the module doc comment on why a bare space or
/// a `/` is never used in a corpus wording driven through this path.
fn post_r1_sequence(
    pg: &Pg,
    who: &Account,
    session: Uuid,
    suite_word: &str,
    file_word: &str,
) -> (String, String) {
    post(
        pg,
        who,
        &batch(vec![
            test_executed_event(session, 10, suite_word),
            test_result_event(session, 20, "failed"),
            file_changed_event(session, 30, &format!("src/{file_word}.rs")),
            test_result_event(session, 40, "passed"),
        ]),
    );
    expected_r1_keys(suite_word, file_word)
}

fn expected_r1_keys(suite_word: &str, file_word: &str) -> (String, String) {
    let topic = format!(
        "test.{}",
        normalize_topic_key(suite_word).expect("a key-shaped suite word")
    );
    let value = format!(
        "fixed_by.{}",
        normalize_topic_key(file_word).expect("a key-shaped file word")
    );
    (topic, value)
}

fn close_session(pg: &Pg, session: Uuid) {
    pg.server.execute(&format!(
        "UPDATE sessions SET status = 'completed', ended_at = now() WHERE id = '{session}'"
    ));
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

fn session_state_of(pg: &Pg, session: Uuid) -> String {
    pg.server.text(&format!(
        "SELECT state FROM consolidation_session WHERE session_id = '{session}'"
    ))
}

fn session_is_done(pg: &Pg, session: Uuid) -> bool {
    session_state_of(pg, session) == "done"
}

fn drained_session(pg: &Pg, session: Uuid) -> bool {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_work WHERE session_id = '{session}' AND state = 'pending'"
    )) == 0
        && session_is_done(pg, session)
}

fn work_all_settled(pg: &Pg, session: Uuid) -> bool {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_work
          WHERE session_id = '{session}' AND state NOT IN ('done', 'failed')"
    )) == 0
}

fn memory_count(pg: &Pg, topic: &str, value: &str, origin_kind: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM memories
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND origin_kind = '{origin_kind}' AND deleted_at IS NULL",
        pg.project
    ))
}

fn reinforces_relation_count(pg: &Pg, topic: &str, value: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM memory_relations r
           JOIN memories m ON m.id = r.to_memory_id
          WHERE m.project_id = '{}' AND m.topic_key = '{topic}' AND m.value_key = '{value}'
            AND r.kind = 'reinforces'",
        pg.project
    ))
}

fn reinforcement_count_of(pg: &Pg, topic: &str, value: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT COALESCE(sum(reinforcement_count), 0)::bigint FROM memories
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND origin_kind = 'consolidated'",
        pg.project
    ))
}

fn candidate_count(pg: &Pg, topic: &str, value: &str) -> i64 {
    pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'",
        pg.project
    ))
}

/// The durable state one paraphrase's topic/value pair has produced, so a
/// before/after comparison is one `assert_eq!` rather than five.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    consolidated: i64,
    corroboration: i64,
    relations: i64,
    reinforcement: i64,
    candidates: i64,
}

fn snapshot(pg: &Pg, topic: &str, value: &str) -> Snapshot {
    Snapshot {
        consolidated: memory_count(pg, topic, value, "consolidated"),
        corroboration: memory_count(pg, topic, value, "corroboration"),
        relations: reinforces_relation_count(pg, topic, value),
        reinforcement: reinforcement_count_of(pg, topic, value),
        candidates: candidate_count(pg, topic, value),
    }
}

// ---------------------------------------------------------------------------
// The paraphrase corpus (SC-736)
// ---------------------------------------------------------------------------

/// One paraphrase pair, read directly off the corpus JSON's own field names —
/// see the shape documented in `tests/feature005/corpora/paraphrase_pairs.json`.
/// Parsed by hand off `serde_json::Value` rather than `#[derive(Deserialize)]`:
/// this crate depends on `serde_json` but not on `serde` itself, and adding a
/// dependency to parse fifty JSON objects would be its own small joke next to
/// SC-737's dependency-baseline assertion.
#[derive(Debug, Clone)]
struct ParaphrasePair {
    id: String,
    kind: String,
    topic_a: String,
    topic_b: String,
    value_a: String,
    value_b: String,
}

fn field(v: &Value, name: &str) -> String {
    v[name]
        .as_str()
        .unwrap_or_else(|| panic!("corpus pair {v} has no string field {name:?}"))
        .to_string()
}

fn load_corpus() -> Vec<ParaphrasePair> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("feature005")
        .join("corpora")
        .join("paraphrase_pairs.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let doc: Value = serde_json::from_str(&text).expect("the corpus is valid JSON");
    doc["pairs"]
        .as_array()
        .expect("the corpus has a `pairs` array")
        .iter()
        .map(|p| ParaphrasePair {
            id: field(p, "id"),
            kind: field(p, "kind"),
            topic_a: field(p, "topic_a"),
            topic_b: field(p, "topic_b"),
            value_a: field(p, "value_a"),
            value_b: field(p, "value_b"),
        })
        .collect()
}

fn pair<'a>(pairs: &'a [ParaphrasePair], id: &str) -> &'a ParaphrasePair {
    pairs
        .iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("the corpus has no pair named {id}"))
}

#[test]
fn the_paraphrase_corpus_is_pre_registered_at_the_size_and_spread_sc_736_requires() {
    let pairs = load_corpus();
    assert!(
        pairs.len() >= 50,
        "SC-736 requires at least fifty pairs; the corpus has {}",
        pairs.len()
    );
    for kind in ["fact", "decision", "convention", "failure", "procedure"] {
        let n = pairs.iter().filter(|p| p.kind == kind).count();
        assert!(
            n >= 8,
            "SC-736 requires the fifty spread across all five kinds; {kind} has only {n}"
        );
    }
    // Every id is unique, so a later test naming one by id addresses exactly
    // one pair.
    let mut ids: Vec<&str> = pairs.iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), pairs.len(), "the corpus has a duplicate pair id");
}

/// SC-736, over the whole corpus: both wordings of every pair's topic collapse
/// to one normalized topic key, and both wordings of its value collapse to one
/// normalized value key. This is the mechanism itself
/// (`contracts/consolidation.md` §7: candidate identity is the normalized key
/// pair) — asserted directly against `cairn_core::knowledge`, independent of
/// which extractor rule would eventually see any particular pair.
#[test]
fn every_paraphrase_pair_normalizes_its_two_wordings_to_one_topic_key_and_one_value_key() {
    let pairs = load_corpus();
    assert!(pairs.len() >= 50);
    for p in &pairs {
        let topic_a = normalize_topic_key(&p.topic_a);
        let topic_b = normalize_topic_key(&p.topic_b);
        assert!(
            topic_a.is_some(),
            "{}: topic wording {:?} did not normalize at all",
            p.id,
            p.topic_a
        );
        assert_eq!(
            topic_a, topic_b,
            "{}: topic wordings {:?} / {:?} normalized to different keys",
            p.id, p.topic_a, p.topic_b
        );

        let value_a = normalize_value_key(&p.value_a);
        let value_b = normalize_value_key(&p.value_b);
        assert!(
            value_a.is_some(),
            "{}: value wording {:?} did not normalize at all",
            p.id,
            p.value_a
        );
        assert_eq!(
            value_a, value_b,
            "{}: value wordings {:?} / {:?} normalized to different keys",
            p.id, p.value_a, p.value_b
        );
    }
}

/// SC-736, driven end to end: a representative subset of the corpus's
/// `failure` pairs is posted as two real R1 sequences apiece — one wording
/// per session — and the pipeline is asserted to reinforce rather than
/// duplicate.
///
/// The five chosen (`failure-02/04/05/07/09`) are exactly the `failure` pairs
/// whose variation technique never uses `/` on either half. `/` is a genuine
/// path separator to `file_token`/`command_words`
/// (`crates/cairn-server/src/extract.rs`) — it takes the last segment rather
/// than folding the separator the way a topic-key segment does — so a pair
/// built for the slash-folding property (asserted above, purely against
/// `normalize_topic_key`) is not a wording this specific rule would fold the
/// way the corpus intends. The other forty-five pairs, `failure-01/03/06/08/10`
/// included, are still covered by the whole-corpus property test above.
#[test]
fn a_representative_subset_of_paraphrased_failure_claims_reinforces_through_the_real_consolidation_pipeline(
) {
    let pg = pg!();
    let pairs = load_corpus();

    for id in [
        "failure-02",
        "failure-04",
        "failure-05",
        "failure-07",
        "failure-09",
    ] {
        let p = pair(&pairs, id);
        let session_a = pg.session_for(&pg.owner);
        let session_b = pg.session_for(&pg.owner);

        let (topic, value) = post_r1_sequence(&pg, &pg.owner, session_a, &p.topic_a, &p.value_a);
        close_session(&pg, session_a);
        with_worker(&pg, || {
            settle(&format!("{id}: the first wording creates a record"), || {
                memory_count(&pg, &topic, &value, "consolidated") == 1
            });
        });

        let (topic_b, value_b) =
            post_r1_sequence(&pg, &pg.owner, session_b, &p.topic_b, &p.value_b);
        assert_eq!(
            (topic_b, value_b),
            (topic.clone(), value.clone()),
            "{id}: the corpus promised one key pair for both wordings"
        );
        close_session(&pg, session_b);
        with_worker(&pg, || {
            settle(&format!("{id}: the second wording reinforces"), || {
                memory_count(&pg, &topic, &value, "corroboration") == 1
            });
        });

        assert_eq!(
            memory_count(&pg, &topic, &value, "consolidated"),
            1,
            "{id}: a paraphrase created a second durable record instead of reinforcing"
        );
        assert_eq!(
            reinforces_relation_count(&pg, &topic, &value),
            1,
            "{id}: expected exactly one reinforcement relation"
        );
        assert_eq!(
            reinforcement_count_of(&pg, &topic, &value),
            1,
            "{id}: the original's reinforcement_count did not reflect the corroboration"
        );
    }
}

// ---------------------------------------------------------------------------
// SC-703 — idempotency over five reclaim rounds
// ---------------------------------------------------------------------------

/// Undo only what the **close** transaction wrote for `session`, leaving
/// everything the **governance** transaction already made durable
/// (memories, candidates, relations) exactly as it is. This is "an abandoned
/// claim reclaimed and re-executed" (`contracts/consolidation.md` §4) with no
/// timing dependency at all — see the module doc comment.
fn reopen_for_reclaim(pg: &Pg, session: Uuid, lease_expired: bool) {
    pg.server.execute(&format!(
        "UPDATE consolidation_work SET state = 'pending'
          WHERE session_id = '{session}' AND state = 'done'"
    ));
    let expires = if lease_expired {
        "now() - interval '1 minute'"
    } else {
        "now() + interval '2 seconds'"
    };
    pg.server.execute(&format!(
        "UPDATE consolidation_session
            SET state = 'claimed', claimed_by = 'dead-worker', claim_expires_at = {expires}
          WHERE session_id = '{session}'"
    ));
    pg.server.execute(&format!(
        "UPDATE consolidation_runs SET state = 'running', finished_at = NULL
          WHERE session_id = '{session}' AND state IN ('finished', 'failed')"
    ));
}

/// SC-703: re-running consolidation over an unchanged set of accepted events
/// produces zero additional records, zero additional relations and zero
/// reinforcement-count changes, in 100% of trials. Five rounds of "abandoned
/// claim, reclaimed" over the very same session and events, snapshotting
/// `memories`, `memory_relations` and `reinforcement_count` before and after
/// every round.
///
/// This is a single session reclaiming its **own** prior creation — see the
/// module doc comment's finding. It is written to assert the contract's
/// guarantee, not the implementation's current behavior.
#[test]
fn rerunning_consolidation_over_an_unchanged_set_of_accepted_events_produces_nothing_new_across_five_rounds(
) {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let (topic, value) = post_r1_sequence(&pg, &pg.owner, session, "sc703suite", "sc703file");
    close_session(&pg, session);

    with_worker(&pg, || {
        settle("the first pass creates the record", || {
            memory_count(&pg, &topic, &value, "consolidated") == 1 && session_is_done(&pg, session)
        });
    });

    let baseline = snapshot(&pg, &topic, &value);
    assert_eq!(baseline.consolidated, 1);

    for round in 1..=5 {
        reopen_for_reclaim(&pg, session, true);
        with_worker(&pg, || {
            settle(&format!("round {round} re-closes the session"), || {
                session_is_done(&pg, session)
            });
        });
        let after = snapshot(&pg, &topic, &value);
        assert_eq!(
            after, baseline,
            "round {round}: a rerun over an unchanged event set changed durable state (SC-703)"
        );
    }
}

// ---------------------------------------------------------------------------
// Additive evidence (FR-798c)
// ---------------------------------------------------------------------------

/// A re-execution that saw more events adds rows to `candidate_source_events`
/// without changing `knowledge_candidates.candidate_id` and without changing
/// which run the candidate belongs to (`run_id` records the run that FIRST
/// created it — `contracts/consolidation.md` §7).
///
/// The extra event repeats the *same* changed file between the original
/// change and the verdict (gapped sequence numbers make room for it), so
/// R1's `dominant()` still names the same primary file and the candidate's
/// identity does not move out from under the assertion — this test is
/// scoped to identity and evidence only, deliberately not to the memory/
/// relation counts a self-reclaim also touches (see the module doc comment).
#[test]
fn a_reexecution_that_saw_more_events_adds_source_evidence_without_changing_candidate_identity_or_first_run(
) {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let (topic, value) = post_r1_sequence(&pg, &pg.owner, session, "evidsuite", "evidfile");
    close_session(&pg, session);

    with_worker(&pg, || {
        settle("the record is created", || {
            memory_count(&pg, &topic, &value, "consolidated") == 1 && session_is_done(&pg, session)
        });
    });

    let candidate_id_before = pg.server.text(&format!(
        "SELECT candidate_id::text FROM knowledge_candidates
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'",
        pg.project
    ));
    let run_id_before = pg.server.text(&format!(
        "SELECT run_id::text FROM knowledge_candidates WHERE candidate_id = '{candidate_id_before}'"
    ));
    let evidence_before = pg.server.count(&format!(
        "SELECT count(*) FROM candidate_source_events WHERE candidate_id = '{candidate_id_before}'"
    ));
    assert_eq!(
        evidence_before, 4,
        "the original citation is suite+failed+file+passed"
    );

    // More evidence "arrives": one more `file_changed` on the very same file,
    // between the original change (seq 30) and the verdict (seq 40) — a real
    // event this session's own pending work grows to include once reclaimed.
    post(
        &pg,
        &pg.owner,
        &batch(vec![file_changed_event(session, 35, "src/evidfile.rs")]),
    );
    reopen_for_reclaim(&pg, session, true);
    with_worker(&pg, || {
        settle(
            "the session re-closes with the extra event folded in",
            || session_is_done(&pg, session),
        );
    });

    let candidate_id_after = pg.server.text(&format!(
        "SELECT candidate_id::text FROM knowledge_candidates
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND run_id = '{run_id_before}'",
        pg.project
    ));
    let run_id_after = pg.server.text(&format!(
        "SELECT run_id::text FROM knowledge_candidates WHERE candidate_id = '{candidate_id_after}'"
    ));
    let evidence_after = pg.server.count(&format!(
        "SELECT count(*) FROM candidate_source_events WHERE candidate_id = '{candidate_id_after}'"
    ));

    assert_eq!(
        candidate_id_after, candidate_id_before,
        "candidate identity changed when the re-execution saw one more event (FR-798c)"
    );
    assert_eq!(
        run_id_after, run_id_before,
        "run_id must keep naming the run that FIRST created the candidate"
    );
    assert!(
        evidence_after > evidence_before,
        "evidence must be additive: expected more than {evidence_before}, found {evidence_after}"
    );
}

// ---------------------------------------------------------------------------
// Stable corroboration identity (FR-798a, FR-798b)
// ---------------------------------------------------------------------------

/// Re-deriving a reinforcement after an abandoned claim yields the same
/// corroboration row, not a second one. Unlike a self-reclaimed *creation*
/// (see the module doc comment), this is the safe case: the corroboration
/// endpoint `reinforce()` writes has `origin_kind = 'corroboration'`, which
/// `project_by_topic` excludes by name, so the reclaim cannot find its own
/// endpoint and reinforce it a second time.
#[test]
fn rederiving_a_reinforcement_after_an_abandoned_claim_yields_the_same_corroboration_row_not_a_second_one(
) {
    let pg = pg!();
    let session_a = pg.session_for(&pg.owner);
    let session_b = pg.session_for(&pg.owner);
    let (topic, value) = post_r1_sequence(&pg, &pg.owner, session_a, "stablesuite", "stablefile");
    close_session(&pg, session_a);
    with_worker(&pg, || {
        settle("the original record is created", || {
            memory_count(&pg, &topic, &value, "consolidated") == 1
        });
    });

    let (topic_b, value_b) =
        post_r1_sequence(&pg, &pg.owner, session_b, "stablesuite", "stablefile");
    assert_eq!((topic_b, value_b), (topic.clone(), value.clone()));
    close_session(&pg, session_b);
    with_worker(&pg, || {
        settle("session b reinforces the original", || {
            memory_count(&pg, &topic, &value, "corroboration") == 1
                && session_is_done(&pg, session_b)
        });
    });

    let corroboration_id_before = pg.server.text(&format!(
        "SELECT id::text FROM memories
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND origin_kind = 'corroboration'",
        pg.project
    ));
    let reinforcement_before = reinforcement_count_of(&pg, &topic, &value);
    assert_eq!(reinforcement_before, 1);

    // Session b's own pass crashed after its corroboration became durable but
    // before its close transaction ran, and is now reclaimed.
    reopen_for_reclaim(&pg, session_b, true);
    with_worker(&pg, || {
        settle("session b's abandoned claim re-closes", || {
            session_is_done(&pg, session_b)
        });
    });

    assert_eq!(
        memory_count(&pg, &topic, &value, "corroboration"),
        1,
        "a re-derived reinforcement produced a second corroboration row"
    );
    let corroboration_id_after = pg.server.text(&format!(
        "SELECT id::text FROM memories
          WHERE project_id = '{}' AND topic_key = '{topic}' AND value_key = '{value}'
            AND origin_kind = 'corroboration'",
        pg.project
    ));
    assert_eq!(
        corroboration_id_after, corroboration_id_before,
        "the corroboration endpoint's identity moved across a reclaim"
    );
    assert_eq!(
        reinforcement_count_of(&pg, &topic, &value),
        1,
        "a re-derived reinforcement bumped the count a second time"
    );
}

// ---------------------------------------------------------------------------
// SC-739 — at least twenty pre-registered restart points
// ---------------------------------------------------------------------------

/// The three points in a pass where a crash leaves a genuinely different
/// on-disk state (see the module doc comment on why there are only three).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// The claim transaction committed (the lease was taken, the attempt was
    /// counted); nothing from governance is durable yet.
    ClaimOnly,
    /// The governance transaction committed (memories/candidates/relations
    /// are durable); the close transaction never ran.
    GovernCommitted,
    /// The pass ran to completion, uninterrupted. The baseline: restarting an
    /// already-finished session must be a no-op.
    FullyClosed,
}

struct CrashPoint {
    name: &'static str,
    stage: Stage,
    /// Attempts already spent by the crashed session, before the pass under
    /// test even starts.
    attempts_before: i32,
    /// Whether this crash point's own claim reinforces a pre-existing record
    /// from an independent, already-completed session, or creates one fresh.
    reinforce: bool,
    /// Whether the dead worker's lease has already expired at restart time.
    lease_expired: bool,
}

impl CrashPoint {
    const fn new(
        name: &'static str,
        stage: Stage,
        attempts_before: i32,
        reinforce: bool,
        lease_expired: bool,
    ) -> Self {
        Self {
            name,
            stage,
            attempts_before,
            reinforce,
            lease_expired,
        }
    }
}

/// Eighteen of the twenty pre-registered crash points (`restart_injection` in
/// `tests/feature005/corpora/manifest.json`); the remaining two —
/// crossing a 200-event batch boundary, and two sessions crashed at once —
/// are their own dedicated tests below, because their setup does not fit this
/// table's per-point shape.
fn crash_points() -> Vec<CrashPoint> {
    use Stage::*;
    vec![
        CrashPoint::new("a cold session was never claimed before the outage", ClaimOnly, 0, false, true),
        CrashPoint::new("the first attempt died right after the claim transaction", ClaimOnly, 1, false, true),
        CrashPoint::new("the second attempt died the same way", ClaimOnly, 2, false, true),
        CrashPoint::new("the third attempt died the same way", ClaimOnly, 3, false, true),
        CrashPoint::new("the fourth attempt died with one attempt left before exhaustion", ClaimOnly, 4, false, true),
        CrashPoint::new("a reinforcing claim's first attempt died right after the claim transaction", ClaimOnly, 1, true, true),
        CrashPoint::new("a reinforcing claim's second attempt died the same way", ClaimOnly, 2, true, true),
        CrashPoint::new("a reinforcing claim's third attempt died the same way", ClaimOnly, 3, true, true),
        CrashPoint::new("a reinforcing claim's fourth attempt died with one attempt left", ClaimOnly, 4, true, true),
        CrashPoint::new("a claimed lease that has not expired yet is not reclaimed early", ClaimOnly, 1, false, false),
        CrashPoint::new("a reinforcing claim's lease that has not expired yet is not reclaimed early", ClaimOnly, 1, true, false),
        CrashPoint::new("a single session's own creation crashed after governance, before close", GovernCommitted, 0, false, true),
        CrashPoint::new("a creation on its third attempt crashed after governance, before close", GovernCommitted, 2, false, true),
        CrashPoint::new("a reinforcement crashed after governance, before close", GovernCommitted, 0, true, true),
        CrashPoint::new("a reinforcement on its third attempt crashed after governance, before close", GovernCommitted, 2, true, true),
        CrashPoint::new("a governance transaction that already committed is not reclaimed before its lease expires", GovernCommitted, 0, false, false),
        CrashPoint::new("a fully drained creation is a no-op restart", FullyClosed, 0, false, true),
        CrashPoint::new("a fully drained reinforcement is a no-op restart", FullyClosed, 0, true, true),
    ]
}

fn seed_claim(pg: &Pg, session: Uuid, lease_expired: bool) {
    let expires = if lease_expired {
        "now() - interval '1 minute'"
    } else {
        "now() + interval '2 seconds'"
    };
    pg.server.execute(&format!(
        "UPDATE consolidation_session
            SET state = 'claimed', claimed_by = 'dead-worker', claim_expires_at = {expires}
          WHERE session_id = '{session}'"
    ));
}

fn run_crash_point(pg: &Pg, idx: usize, cp: &CrashPoint) {
    let suite = format!("crashsuite{idx}");
    let file = format!("crashfile{idx}");
    let (topic, value) = expected_r1_keys(&suite, &file);
    let create_origin = "consolidated";
    let reinforce_origin = "corroboration";

    if cp.reinforce {
        // The record this scenario's own claim will reinforce, established
        // for real by an independent, already-completed session.
        let base = pg.session_for(&pg.owner);
        post_r1_sequence(pg, &pg.owner, base, &suite, &file);
        close_session(pg, base);
        with_worker(pg, || {
            settle(&format!("{}: the base record exists", cp.name), || {
                memory_count(pg, &topic, &value, create_origin) == 1
            });
        });
    }

    let session = pg.session_for(&pg.owner);
    post_r1_sequence(pg, &pg.owner, session, &suite, &file);
    pg.server.execute(&format!(
        "UPDATE consolidation_work SET attempts = {} WHERE session_id = '{session}'",
        cp.attempts_before
    ));

    let expect_origin = if cp.reinforce {
        reinforce_origin
    } else {
        create_origin
    };

    if matches!(cp.stage, Stage::GovernCommitted | Stage::FullyClosed) {
        close_session(pg, session);
        with_worker(pg, || {
            settle(
                &format!("{}: the real pass completes once", cp.name),
                || {
                    memory_count(pg, &topic, &value, expect_origin) == 1
                        && session_is_done(pg, session)
                },
            );
        });
    }

    match cp.stage {
        Stage::ClaimOnly => {
            seed_claim(pg, session, cp.lease_expired);
            close_session(pg, session);
        }
        Stage::GovernCommitted => reopen_for_reclaim(pg, session, cp.lease_expired),
        Stage::FullyClosed => {}
    }

    let before = snapshot(pg, &topic, &value);

    if !cp.lease_expired {
        with_worker(pg, || {
            std::thread::sleep(Duration::from_millis(400));
            assert_eq!(
                snapshot(pg, &topic, &value),
                before,
                "{}: reclaimed before its lease expired",
                cp.name
            );
        });
        pg.server.execute(&format!(
            "UPDATE consolidation_session SET claim_expires_at = now() - interval '1 second'
              WHERE session_id = '{session}'"
        ));
    }

    with_worker(pg, || {
        settle(
            &format!("{}: the session drains after the restart", cp.name),
            || session_is_done(pg, session),
        );
    });

    let after = snapshot(pg, &topic, &value);

    // SC-739 compares the interrupted run against **an uninterrupted run over
    // the same events**, not against the state the outage happened to leave
    // behind. Those differ for a session whose pass had not committed anything
    // yet: a cold session legitimately produces its first record after the
    // restart, and asserting nothing changed would be asserting that a
    // reclaimed session is never consolidated — the opposite of what reclaim is
    // for.
    //
    // The converged outcome is the same whatever stage the crash happened at,
    // which is the property worth pinning: one record for the key, and a
    // reinforcement exactly where a second session's evidence earned one.
    let expected_extra = i64::from(cp.reinforce);
    assert_eq!(
        after.consolidated, 1,
        "{}: expected exactly one durable record for the key, not {}",
        cp.name, after.consolidated
    );
    assert_eq!(
        (after.corroboration, after.relations, after.reinforcement),
        (expected_extra, expected_extra, expected_extra),
        "{}: the reinforcement effect did not converge on the uninterrupted one",
        cp.name
    );
    // Where the pass had already committed, the restart must add nothing at
    // all — the stronger statement, available only for the stages where there
    // was something to preserve.
    if matches!(cp.stage, Stage::GovernCommitted | Stage::FullyClosed) {
        assert_eq!(
            after, before,
            "{}: the restart changed durable state a restart must not change",
            cp.name
        );
    }

    // And running again changes nothing, whatever the crash stage was. This is
    // the half that catches a second durable effect from a re-derivation, which
    // is the failure SC-739 names.
    with_worker(pg, || {
        std::thread::sleep(Duration::from_millis(400));
    });
    assert_eq!(
        snapshot(pg, &topic, &value),
        after,
        "{}: a further pass produced a second durable effect",
        cp.name
    );
    assert!(
        work_all_settled(pg, session),
        "{}: an event was left stranded (neither done nor failed)",
        cp.name
    );
}

/// SC-739: restarting at each of at least twenty pre-registered points during
/// consolidation, including mid-pass, yields the same durable knowledge, the
/// same relations and the same reinforcement counts as an uninterrupted run,
/// and leaves zero events permanently unconsolidated.
///
/// Several of these points are a single session reclaiming its own prior
/// **creation** across a governance-committed crash — the module doc comment
/// names exactly why those are expected to fail against the implementation as
/// it stands.
#[test]
fn restarting_at_each_pre_registered_crash_point_converges_to_the_same_durable_outcome_with_zero_events_stranded(
) {
    let pg = pg!();
    let points = crash_points();
    assert!(points.len() >= 18, "the crash-point table lost entries");
    for (idx, cp) in points.iter().enumerate() {
        run_crash_point(&pg, idx, cp);
    }
}

/// The nineteenth pre-registered crash point: a session whose 205 events span
/// two batches (`BATCH_EVENTS = 200`) crashes between them.
#[test]
fn a_session_crossing_a_batch_boundary_recovers_from_a_crash_after_its_first_batch() {
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    // Plain `file_changed` events: this crash point is about the claim
    // machinery surviving a restart across the batch bound, not about
    // extraction, exactly as `feature005_consolidation_claims.rs`'s own
    // batching test is.
    let events: Vec<Value> = (1..=205u64)
        .map(|seq| file_changed_event(session, seq, &format!("src/batchf{seq}.rs")))
        .collect();
    post(&pg, &pg.owner, &batch(events));
    close_session(&pg, session);

    with_worker(&pg, || {
        settle("the first batch of 200 completes", || {
            pg.server.count(&format!(
                "SELECT count(*) FROM consolidation_work
                  WHERE session_id = '{session}' AND state = 'done'"
            )) >= 200
        });
    });
    // Crashed here: the first 200 are done, the remaining 5 are still
    // pending, and the close transaction's `CASE` has already re-opened the
    // session for its second batch (§4).
    assert_eq!(session_state_of(&pg, session), "pending");

    with_worker(&pg, || {
        settle("the whole session drains after the restart", || {
            drained_session(&pg, session)
        });
    });
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM consolidation_work WHERE session_id = '{session}' AND state = 'done'"
        )),
        205
    );
}

/// The twentieth pre-registered crash point: two independent sessions crash
/// in the same outage and neither one blocks the other's recovery.
#[test]
fn two_independently_crashed_sessions_recover_without_blocking_each_other() {
    let pg = pg!();
    let session_a = pg.session_for(&pg.owner);
    let session_b = pg.session_for(&pg.owner);
    let (topic_a, value_a) = post_r1_sequence(
        &pg,
        &pg.owner,
        session_a,
        "concurrentsuitea",
        "concurrentfilea",
    );
    let (topic_b, value_b) = post_r1_sequence(
        &pg,
        &pg.owner,
        session_b,
        "concurrentsuiteb",
        "concurrentfileb",
    );
    pg.server.execute(&format!(
        "UPDATE consolidation_work SET attempts = 2 WHERE session_id IN ('{session_a}', '{session_b}')"
    ));
    seed_claim(&pg, session_a, true);
    seed_claim(&pg, session_b, true);
    close_session(&pg, session_a);
    close_session(&pg, session_b);

    with_worker(&pg, || {
        settle("both crashed sessions drain independently", || {
            session_is_done(&pg, session_a) && session_is_done(&pg, session_b)
        });
    });

    assert_eq!(memory_count(&pg, &topic_a, &value_a, "consolidated"), 1);
    assert_eq!(memory_count(&pg, &topic_b, &value_b, "consolidated"), 1);
}

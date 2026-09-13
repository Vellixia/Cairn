//! Consolidation's governance, as a contract (T043, `contracts/consolidation.md`
//! §5, §9), and adversarial privacy — SC-704, SC-705, SC-734, SC-735, SC-741,
//! SC-742, SC-749.
//!
//! The shipped extractor (`deterministic_v1`) cannot itself emit an
//! adversarial proposal: every rule in `extract.rs` hard-codes
//! `proposed_domain: KnowledgeDomain::Project`, and `CandidateProposal` has no
//! field for durability, verification, supersession, scope or authorization at
//! all (`contracts/extraction.md` §2, FR-805b). So "an adversarial proposal"
//! is driven the only way a real client can drive one: through event content
//! chosen so that, *if* governance ever read an extractor's wish rather than
//! deciding for itself, the outcome would differ. What these tests measure is
//! that it never does.

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

const SETTLE: Duration = Duration::from_secs(30);
const WORKER_POOL: &str = "5";

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

fn session_done(pg: &Pg, session: Uuid) -> bool {
    pg.server.count(&format!(
        "SELECT count(*) FROM consolidation_session
          WHERE session_id = '{session}' AND state = 'done'"
    )) == 1
}

// ---------------------------------------------------------------------------
// Event construction
// ---------------------------------------------------------------------------

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

fn command_executed(session: Uuid, seq: u64, line: &str, exit: i32) -> Value {
    event(
        session,
        seq,
        "command_executed",
        json!({ "Command": { "command_line": line, "exit_status": exit }}),
    )
}

fn test_executed(session: Uuid, seq: u64, command: &str) -> Value {
    event(
        session,
        seq,
        "test_executed",
        json!({ "TestInvocation": { "test_command": command }}),
    )
}

fn decision_signal(
    session: Uuid,
    seq: u64,
    subject: &str,
    object: &str,
    justified_by_seq: Option<u64>,
) -> Value {
    event(
        session,
        seq,
        "decision_signal",
        json!({ "Decision": {
            "decision_kind": "adopt",
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

fn assert_all_accepted(label: &str, body: &Value, status: u16) {
    assert_eq!(status, 200, "{label}: {body}");
    let got = statuses(body);
    assert!(
        got.iter().all(|s| s == "accepted"),
        "{label}: not every event was accepted: {got:?} ({body})"
    );
}

// ---------------------------------------------------------------------------
// SC-742 — an adversarial-shaped attempt changes nothing, compared to nothing
// ---------------------------------------------------------------------------

/// Every axis a proposal has no field to assert, read back from the database
/// rather than trusted from the extractor's (nonexistent) claim.
#[derive(Debug, PartialEq, Eq)]
struct GovernanceOutcome {
    /// Durable **team** records: gate 10 permits a proposal only, and the
    /// baseline extractor never even proposes one (FR-809, SC-734).
    team_records: i64,
    /// Durable **personal** records for anyone: ownership always comes from
    /// the ingest-time account binding, never from a proposal (FR-810a).
    personal_records: i64,
    /// `memories` rows this project holds whose scope is not the plain
    /// project scope consolidation always writes (gate 4/5).
    non_project_scoped_memories: i64,
    /// `memories` rows this project holds with an asserted verification —
    /// never set by consolidation (gate 9, FR-811).
    verified_memories: i64,
    /// `verification_reports` rows attributed to this project by consolidation
    /// (there is no code path that writes one; this stays zero either way).
    verification_report_rows: i64,
    /// `memories` rows superseded by anything (gate 8, FR-800).
    superseded_memories: i64,
    /// `memory_relations` rows of kind `supersedes` for this project.
    supersedes_relations: i64,
}

fn governance_outcome(pg: &Pg) -> GovernanceOutcome {
    GovernanceOutcome {
        team_records: pg.server.count("SELECT count(*) FROM team_knowledge"),
        personal_records: pg.server.count("SELECT count(*) FROM personal_knowledge"),
        non_project_scoped_memories: pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND (scope <> 'project' OR scope_key <> '{}')",
            pg.project, pg.project
        )),
        verified_memories: pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND verification = 'verified'",
            pg.project
        )),
        verification_report_rows: pg.server.count(&format!(
            "SELECT count(*) FROM verification_reports WHERE project_id = '{}'",
            pg.project
        )),
        superseded_memories: pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND superseded_by_id IS NOT NULL",
            pg.project
        )),
        supersedes_relations: pg.server.count(&format!(
            "SELECT count(*) FROM memory_relations
              WHERE project_id = '{}' AND kind = 'supersedes'",
            pg.project
        )),
    }
}

#[test]
fn an_adversarial_attempt_to_claim_durability_domain_scope_ownership_verification_and_supersession_changes_nothing_versus_proposing_nothing(
) {
    // SC-742. Run A drives R7 (`contracts/extraction.md` §13.5) with subject
    // and object tokens chosen to *read* like a claim of team ownership, wider
    // scope, another account's personal ownership, asserted durability,
    // verified status and supersession. Run B proposes nothing at all. Every
    // axis a proposal has no field for is compared, and both runs land at the
    // same zero.
    let adversarial = pg!();
    let session = adversarial.session_for(&adversarial.owner);
    let claims = [
        // A foreign domain, in words. The *object* has to be a token the
        // session established, because the server justifies both
        // independently — an unjustified one is refused at ingest and never
        // reaches governance, which would test the wrong boundary.
        ("team", "ledger", "ledger"),
        ("personal", "vault", "vault"), // another account's ownership, in words
        ("supersedes", "baseline", "baseline"), // supersession, in words
        ("verified", "status", "status"), // verified/durable status, in words
        ("scope", "everyone", "everyone"), // a wider scope, in words
    ];
    let mut seq = 1u64;
    let mut events = Vec::new();
    for (subject, path_stem, object) in claims {
        events.push(file_changed(
            session,
            seq,
            &format!("{subject}/{path_stem}.rs"),
        ));
        let establishing_seq = seq;
        seq += 1;
        events.push(decision_signal(
            session,
            seq,
            subject,
            object,
            Some(establishing_seq),
        ));
        seq += 1;
    }
    let (body, status) = post(&adversarial, &adversarial.owner, events);
    assert_all_accepted("adversarial run", &body, status);
    close_session(&adversarial, session);
    let worker = Worker::start(&adversarial.server.database_url);
    settle("the adversarial-shaped session consolidates", || {
        session_done(&adversarial, session)
    });
    drop(worker);

    // It did produce ordinary knowledge — this is not a test that nothing
    // happened, only that nothing *forbidden* did.
    let produced = adversarial.server.count(&format!(
        "SELECT count(*) FROM memories
          WHERE project_id = '{}' AND origin_kind = 'consolidated'
            AND topic_key LIKE 'decision.%'",
        adversarial.project
    ));
    assert_eq!(
        produced, 5,
        "expected one DECISION record per adversarial-worded claim"
    );
    // R4 fires too, and should: a decision signal beside file changes makes the
    // decision *locatable*, which is a different claim from what the signal
    // said and is deliberately weak. Counting every consolidated record and
    // expecting five would have failed on a rule doing its job.
    assert_eq!(
        adversarial.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND origin_kind = 'consolidated'
                AND topic_key LIKE 'area.%'",
            adversarial.project
        )),
        1,
        "R4 records one locatability claim for the session"
    );
    for (subject, _, object) in claims {
        let state = adversarial.server.text(&format!(
            "SELECT state FROM memories
              WHERE project_id = '{}' AND topic_key = 'decision.{subject}' AND value_key = '{object}'",
            adversarial.project
        ));
        assert_eq!(state, "active", "a {subject} record was not plain `active`");
    }

    let nothing = pg!();
    let inert_session = nothing.session_for(&nothing.owner);
    let (body, status) = post(
        &nothing,
        &nothing.owner,
        vec![
            file_changed(inert_session, 1, "misc/note.rs"),
            event(
                inert_session,
                2,
                "tool_succeeded",
                json!({ "Tool": { "vendor_tool": "Bash", "tool_class": "execute" }}),
            ),
        ],
    );
    assert_all_accepted("nothing-proposed run", &body, status);
    close_session(&nothing, inert_session);
    let worker = Worker::start(&nothing.server.database_url);
    settle("the inert session consolidates", || {
        session_done(&nothing, inert_session)
    });
    drop(worker);
    let produced_nothing = nothing.server.count(&format!(
        "SELECT count(*) FROM memories WHERE project_id = '{}'",
        nothing.project
    ));
    assert_eq!(
        produced_nothing, 0,
        "the control run was supposed to propose nothing"
    );

    assert_eq!(
        governance_outcome(&adversarial),
        governance_outcome(&nothing),
        "the adversarial-worded run left a durability, authorization, domain, scope, \
         verification or supersession trace the empty run did not"
    );
}

// ---------------------------------------------------------------------------
// SC-704, SC-705 — a refusal is reachable, and it carries no material
// ---------------------------------------------------------------------------

#[test]
fn an_oversized_client_supplied_key_is_refused_with_empty_content_and_no_keys_rather_than_persisted(
) {
    // `consolidation.md` §5 gate 2 and §9. R7 keys its topic as
    // `decision.<subject_token>`: `SUBJECT_TOKEN_MAX_CHARS` is 128, the same
    // as `TOPIC_KEY_MAX_CHARS`, so a maximal subject token is legal at ingest
    // (event.rs's own bound) but its topic key overflows Cairn's own bound
    // once the `decision.` prefix is added — a real, client-reachable
    // `key_normalization_failed`. This is the adversarial corpus this
    // pipeline can actually produce: two distinct oversized attempts, so
    // §7's refusal identity (keyed on reason + a digest of the proposal, not
    // the key pair) is exercised rather than collapsed onto one row.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    // A repeating *word*, not a run of one letter. A 128-character
    // single-token path segment reads as an encoded secret to the content
    // screen, and the event would be refused at ingest for a reason that has
    // nothing to do with the oversized key this test is about.
    // Exactly 128 characters: the longest a topic key may be, and the longest
    // `VocabToken::subject` accepts. One more and the normalizer returns
    // nothing, the token never enters the vocabulary, and the signal is refused
    // at ingest — a refusal in the wrong layer for this test. At exactly 128 the
    // signal is accepted and R7's `decision.` prefix is what pushes the key over
    // the bound, which is the gate-2 refusal the test is about.
    let filler_a = format!("{}ab", "ab_".repeat(42));
    let filler_c = format!("{}cd", "cd_".repeat(42));
    let events = vec![
        // The path carries both tokens: the directory establishes the
        // oversized subject and the file stem establishes the object. A path
        // with only the subject would have the signal refused at ingest for an
        // unjustified object, which is a different refusal in a different layer
        // from the one this test is about.
        file_changed(session, 1, &format!("{filler_a}/b.rs")),
        decision_signal(session, 2, &filler_a, "b", Some(1)),
        file_changed(session, 3, &format!("{filler_c}/d.rs")),
        decision_signal(session, 4, &filler_c, "d", Some(3)),
    ];
    let (body, status) = post(&pg, &pg.owner, events);
    assert_all_accepted("oversized-key fixture", &body, status);

    close_session(&pg, session);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the pass finishes", || session_done(&pg, session));

    let refused = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}'
            AND kc.decision = 'refused' AND kc.refusal_reason = 'key_normalization_failed'"
    ));
    // Three, not two. R7 refuses each of the two signals, and R4 refuses once
    // more because its own `area.` key is built from the same oversized module
    // token — so three distinct malformed proposals record three distinct
    // refusals. That is the property this test is for: §7 keys a refusal on the
    // reason and a digest of the proposal rather than on the key pair, which a
    // refused candidate does not have. Keyed on the pair, all three would have
    // collapsed onto one row and the count FR-807 and SC-705 depend on would be
    // wrong by two.
    assert_eq!(refused, 3, "expected one refusal per oversized attempt");
    let distinct = pg.server.count(&format!(
        "SELECT count(DISTINCT kc.candidate_id) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}'
            AND kc.decision = 'refused' AND kc.refusal_reason = 'key_normalization_failed'"
    ));
    assert_eq!(
        distinct, 3,
        "two malformed proposals collapsed onto one refusal identity"
    );

    // SC-705: no portion of the content is ever carried by a refusal row.
    let with_content = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}' AND kc.decision = 'refused' AND kc.content <> ''"
    ));
    assert_eq!(with_content, 0);
    let with_keys = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates kc
           JOIN consolidation_runs cr ON cr.run_id = kc.run_id
          WHERE cr.session_id = '{session}' AND kc.decision = 'refused'
            AND (kc.topic_key IS NOT NULL OR kc.value_key IS NOT NULL)"
    ));
    assert_eq!(with_keys, 0);

    // SC-704: nothing this refused becomes durable knowledge.
    let durable = pg.server.count(&format!(
        "SELECT count(*) FROM memories WHERE project_id = '{}'",
        pg.project
    ));
    assert_eq!(durable, 0, "a refused proposal still became a memory row");
}

// ---------------------------------------------------------------------------
// SC-734 — team is never authoritative from consolidation
// ---------------------------------------------------------------------------

#[test]
fn consolidation_never_creates_an_authoritative_team_record_only_a_project_scoped_one() {
    // SC-734. `team_knowledge` gains no row at all: every rule in
    // `extract.rs` hard-codes `proposed_domain: KnowledgeDomain::Project`, and
    // gate 4/5 resolves the domain from context rather than the proposal
    // either way. A subject and object worded as if to name the team domain
    // still land as an ordinary project DECISION.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);
    let events = vec![
        file_changed(session, 1, "team/ledger.rs"),
        // `team` is the subject, which is what makes this fixture
        // team-worded. The object is a token the file established, because
        // the point is what governance does with the subject and not
        // whether ingest justifies the object.
        decision_signal(session, 2, "team", "ledger", Some(1)),
    ];
    let (body, status) = post(&pg, &pg.owner, events);
    assert_all_accepted("team-worded fixture", &body, status);

    close_session(&pg, session);
    let _worker = Worker::start(&pg.server.database_url);
    settle("the pass finishes", || session_done(&pg, session));

    assert_eq!(pg.server.count("SELECT count(*) FROM team_knowledge"), 0);
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND topic_key = 'decision.team' AND value_key = 'ledger'
                AND type = 'decision' AND scope = 'project' AND origin_kind = 'consolidated'",
            pg.project
        )),
        1,
        "the team-worded claim did not land as an ordinary project decision"
    );
}

// ---------------------------------------------------------------------------
// SC-735 — disagreement is a conflict, never a supersession
// ---------------------------------------------------------------------------

#[test]
fn a_second_disagreeing_value_under_one_subject_is_recorded_as_a_conflict_never_a_supersession() {
    // SC-735, and `consolidation.md` §5 gate 7 vs gate 8: same `topic_key`,
    // overlapping (here: identical project) scope, a different `value_key`
    // conflicts; nothing here may ever supersede anything (FR-799, FR-800).
    let pg = pg!();
    let p = pg.session_for(&pg.owner);
    let q = pg.session_for(&pg.owner);

    let (body, status) = post(
        &pg,
        &pg.owner,
        vec![
            file_changed(p, 1, "storage/server.rs"),
            decision_signal(p, 2, "storage", "server", Some(1)),
        ],
    );
    assert_all_accepted("session P", &body, status);
    let (body, status) = post(
        &pg,
        &pg.owner,
        vec![
            file_changed(q, 1, "storage/database.rs"),
            decision_signal(q, 2, "storage", "database", Some(1)),
        ],
    );
    assert_all_accepted("session Q", &body, status);

    close_session(&pg, p);
    close_session(&pg, q);
    let _worker = Worker::start(&pg.server.database_url);
    settle("both sessions consolidate", || {
        session_done(&pg, p) && session_done(&pg, q)
    });

    let records = pg.server.count(&format!(
        "SELECT count(*) FROM memories
          WHERE project_id = '{}' AND topic_key = 'decision.storage'
            AND value_key IN ('server', 'database') AND origin_kind = 'consolidated'",
        pg.project
    ));
    assert_eq!(
        records, 2,
        "both disagreeing values should be durable, independent records"
    );

    let conflicts = pg.server.count(&format!(
        "SELECT count(*) FROM memory_relations mr
           JOIN memories a ON a.id = mr.from_memory_id
           JOIN memories b ON b.id = mr.to_memory_id
          WHERE mr.project_id = '{}' AND mr.kind = 'conflicts_with'
            AND a.topic_key = 'decision.storage' AND b.topic_key = 'decision.storage'
            AND mr.basis = 'deterministic_rule'",
        pg.project
    ));
    assert_eq!(
        conflicts, 1,
        "the disagreement was not recorded as a conflict"
    );

    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memory_relations WHERE project_id = '{}' AND kind = 'supersedes'",
            pg.project
        )),
        0
    );
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM memories
              WHERE project_id = '{}' AND topic_key = 'decision.storage' AND superseded_by_id IS NOT NULL",
            pg.project
        )),
        0
    );
}

// ---------------------------------------------------------------------------
// SC-741 — an adversarial ingest corpus never becomes a stored safe event
// ---------------------------------------------------------------------------

#[test]
fn an_adversarial_ingest_corpus_of_absolute_paths_secrets_and_forbidden_fields_never_reaches_a_stored_safe_event(
) {
    // SC-741. Every attempt here is well-formed enough to be worth trying —
    // some are schema-legal but privacy-refused, which is the case the
    // criterion insists on ("well-formed for the safe-event schema but
    // carries a secret inside an approved text field") — and every one is
    // refused before it becomes a row in `safe_events`, which is the only
    // table extraction ever reads from.
    let pg = pg!();
    let session = pg.session_for(&pg.owner);

    let absolute = file_changed(session, 1, "/etc/passwd");
    let traversal = file_changed(session, 2, "../../etc/passwd");
    let secret_in_command = command_executed(
        session,
        3,
        "deploy --url https://user:hunter2@example.test/repo",
        0,
    );
    let secret_in_test_command =
        test_executed(session, 4, "cargo test --token sk-abcdefghij1234567890");
    let mut forbidden_field = file_changed(session, 5, "crates/cairnd/src/sync.rs");
    forbidden_field["summary"] = json!("a human-readable gist of what happened");
    let mut raw_vendor_json = file_changed(session, 6, "crates/cairnd/src/sync.rs");
    raw_vendor_json["raw_vendor_event"] = json!({"tool": "Bash", "input": {"command": "rm -rf /"}});

    let (body, status) = post(
        &pg,
        &pg.owner,
        vec![
            absolute,
            traversal,
            secret_in_command,
            secret_in_test_command,
            forbidden_field,
            raw_vendor_json,
        ],
    );
    assert_eq!(status, 200, "{body}");
    let got_statuses = statuses(&body);
    assert!(
        got_statuses.iter().all(|s| s == "rejected"),
        "an adversarial event was not rejected: {got_statuses:?} ({body})"
    );
    let got_reasons = reasons(&body);
    assert_eq!(
        got_reasons,
        vec![
            "repo_file_absolute",
            "repo_file_traversal",
            "content_screening_failed",
            "content_screening_failed",
            "forbidden_field_name",
            // Not `unknown_field`: the refused-name check runs before schema
            // deserialization, so a name the sync boundary refuses is answered
            // as one whether or not the schema also lacks it. Both terms are in
            // the §7.2 vocabulary, and the more specific one says more.
            "forbidden_field_name",
        ]
    );
    // The refusal must never echo the secret it refused.
    let rendered = body.to_string();
    assert!(!rendered.contains("hunter2"));
    assert!(!rendered.contains("sk-abcdefghij1234567890"));

    // What matters most: none of it is in `safe_events` — the table
    // extraction reads from and the only place a "reach the extraction
    // stage" claim could be checked against.
    assert_eq!(
        pg.server.count(&format!(
            "SELECT count(*) FROM safe_events WHERE session_id = '{session}'"
        )),
        0,
        "an adversarial event from the corpus was persisted anyway"
    );
}

// ---------------------------------------------------------------------------
// SC-749 — one pass, one project, even against a matching foreign subject
// ---------------------------------------------------------------------------

#[test]
fn a_pass_over_one_project_never_reads_writes_or_cites_another_projects_events() {
    // SC-749. Project two is seeded with the *same* subject and object
    // tokens project one will propose, and is consolidated first — the
    // strongest available temptation for a buggy cross-project read to
    // "reinforce" project one's candidate against it instead of creating an
    // independent record.
    let pg = pg!();
    let other_project = pg.extra_project("feature005-other", &[&pg.owner]);

    let foreign_session = pg.session_in(other_project, &pg.owner);
    let (body, status) = post(
        &pg,
        &pg.owner,
        vec![
            file_changed(foreign_session, 1, "storage/server.rs"),
            decision_signal(foreign_session, 2, "storage", "server", Some(1)),
        ],
    );
    assert_all_accepted("other project's session", &body, status);
    close_session(&pg, foreign_session);

    let worker = Worker::start(&pg.server.database_url);
    settle("the other project's session consolidates", || {
        session_done(&pg, foreign_session)
    });

    let foreign_memory = pg.server.text(&format!(
        "SELECT id::text FROM memories
          WHERE project_id = '{other_project}' AND topic_key = 'decision.storage' AND value_key = 'server'"
    ));

    let home_session = pg.session_for(&pg.owner);
    let (body, status) = post(
        &pg,
        &pg.owner,
        vec![
            file_changed(home_session, 1, "storage/server.rs"),
            decision_signal(home_session, 2, "storage", "server", Some(1)),
        ],
    );
    assert_all_accepted("the fixture project's session", &body, status);
    close_session(&pg, home_session);
    settle("the fixture project's session consolidates", || {
        session_done(&pg, home_session)
    });
    drop(worker);

    // An independent record in the fixture project, not a reinforcement of
    // the other project's.
    let home_record = pg.server.count(&format!(
        "SELECT count(*) FROM memories
          WHERE project_id = '{}' AND topic_key = 'decision.storage' AND value_key = 'server'
            AND origin_kind = 'consolidated'",
        pg.project
    ));
    assert_eq!(
        home_record, 1,
        "the fixture project did not get its own record"
    );
    let corroboration_of_foreign = pg.server.count(&format!(
        "SELECT count(*) FROM memory_relations WHERE to_memory_id = '{foreign_memory}'"
    ));
    assert_eq!(
        corroboration_of_foreign, 0,
        "the fixture project's pass reinforced the other project's record"
    );

    // Every event the fixture project's candidates cite belongs to the
    // fixture project.
    let foreign_citations = pg.server.count(&format!(
        "SELECT count(*) FROM candidate_source_events cse
           JOIN knowledge_candidates kc ON kc.candidate_id = cse.candidate_id
           JOIN safe_events se ON se.event_id = cse.event_id
          WHERE kc.project_id = '{}' AND se.project_id <> '{}'",
        pg.project, pg.project
    ));
    assert_eq!(
        foreign_citations, 0,
        "a candidate cited an event from another project"
    );

    // And the other project's own single record is untouched: still exactly
    // one candidate, one memory, from its own run.
    let foreign_candidates = pg.server.count(&format!(
        "SELECT count(*) FROM knowledge_candidates
          WHERE project_id = '{other_project}' AND topic_key = 'decision.storage' AND value_key = 'server'"
    ));
    assert_eq!(foreign_candidates, 1);
    let foreign_records = pg.server.count(&format!(
        "SELECT count(*) FROM memories
          WHERE project_id = '{other_project}' AND topic_key = 'decision.storage' AND value_key = 'server'"
    ));
    assert_eq!(foreign_records, 1);
}

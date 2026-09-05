//! Migration survives interruption, at any phase, and re-keys the corpus it
//! moves without ever discarding a side of a collision (T140;
//! `migration-cutover.md` §7, §8, §12.4; FR-867a, FR-868, FR-869, SC-750).
//!
//! One thesis, stated once: **"resume" and "run again" are the same
//! operation, and normalizing a key is not the same act as judging its
//! content.** Interruption at any point — a kill, a crash, a restart between
//! two CLI invocations — must leave `migration_state` exactly where it last
//! committed `done`, and re-running must re-enter at the first phase that is
//! not done, never skip one, and never create a second copy of anything it
//! already delivered. Separately, re-keying corrects a *legacy* key to the
//! same normalized form new knowledge already uses, and a collision on that
//! form is decided by `consolidation.md` §5 exactly as it would be for any
//! other candidate — surfaced as a conflict, never resolved by deleting a
//! side.
//!
//! # Two fixtures, for two different properties
//!
//! The resumability tests ([`resumable`]) use a store with exactly one
//! trivially eligible record — not [`install_legacy_v7`] — because
//! `install_legacy_v7`'s `team_proposed` row is proposed by an account
//! nobody in this suite can ever sign in as. Its `author_mismatch` can never
//! be resolved, so the Drain phase built on that fixture can never reach
//! `done` — only `blocked` — and `first_unfinished` (correctly) always names
//! the earliest phase that is not `done`. Against that fixture, asking "did
//! this resume at `verify_possession`?" would always get the answer `drain`,
//! whatever phase's state was actually rewound. The property under test here
//! is *which phase a resume names*, which needs a store that can actually
//! reach `done` at every phase — so it gets one.
//!
//! The re-keying tests use the real thing: a populated Feature 004 store at
//! the actual v7 schema, because SC-750 is a claim about the corpus users
//! already have, and a fixture written with already-normalized keys would
//! prove nothing.
//!
//! One PostgreSQL database serves the whole suite, so every server-side count
//! below is scoped to specific ids or to the account under test, never to a
//! bare, suite-wide `count(*)`.

use cairn_e2e::feature005::{install_legacy_v7, LegacyIds};
use cairn_e2e::{attach_server, Sandbox, Server};
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixture 1: one trivially eligible record, for the resumability tests
// ---------------------------------------------------------------------------

struct Resumable {
    s: Sandbox,
    server: Server,
    account: Uuid,
}

fn resumable() -> Option<Resumable> {
    let server = Server::start()?;
    let s = Sandbox::new();
    let (account, token) = server.new_user("resumer");
    attach_server(&s, &server, &token);
    s.must(&["link", "--create"]);

    // One personal-knowledge row, owned by this account. Personal-knowledge
    // eligibility is decided by ownership alone (no outbox row is consulted
    // for it), and this store has no pattern, no relation, no team row and no
    // project memory to ever block — so a single `--run` can reach `done` at
    // every one of the six phases.
    let id = Uuid::now_v7();
    // `(writer_id, writer_seq)` is unique on the SERVER's `personal_knowledge`
    // table (data-model.md §4 step 6), and one Postgres database serves the
    // whole suite — a fixed literal here collided with another test's row
    // under exactly this constraint. The account is already unique per test
    // (`Server::new_user`), so folding it into the writer id makes this one
    // unique too.
    let writer_id = format!("resumability-writer-{account}");
    s.exec_sql(&format!(
        "INSERT INTO personal_knowledge (id, owner_user_id, knowledge_type, content,
                                          topic_key, value_key, writer_id, writer_seq,
                                          created_at)
         VALUES ('{id}', '{account}', 'fact', 'a fact for the resumability test',
                 'resumability.test', 'value', '{writer_id}', 1,
                 '2026-09-05T00:00:00Z')"
    ));
    Some(Resumable { s, server, account })
}

macro_rules! resumable {
    () => {
        match resumable() {
            Some(r) => r,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

const PHASES: [&str; 6] = [
    "inspect",
    "claim_pattern_ownership",
    "drain",
    "verify_possession",
    "switch_authority",
    "demote",
];

/// Mark `phase` and every phase after it (in the order `migrate005::run`
/// walks them) as not done, directly in `migration_state` — the interruption
/// this file drives, standing in for a kill between two phases.
fn rewind_from(s: &Sandbox, phase: &str) {
    let idx = PHASES
        .iter()
        .position(|p| *p == phase)
        .unwrap_or_else(|| panic!("not a real phase name: {phase}"));
    for p in &PHASES[idx..] {
        s.exec_sql(&format!(
            "UPDATE migration_state
                SET state = 'pending', detail_count = NULL,
                    started_at = NULL, finished_at = NULL
              WHERE phase = '{p}'"
        ));
    }
}

fn phase_states(s: &Sandbox) -> Vec<Value> {
    s.json(&["migrate", "--status"])["status"]["phases"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 1. Interruption at every phase is retry-safe
// ---------------------------------------------------------------------------

/// For each of the six phases: drive one uninterrupted `--run`, rewind that
/// phase and everything after it back to not-done, and run again. Every time,
/// the run resumes exactly at the named phase, every phase ends `done`, and
/// the server's row count for the one record this store carries is exactly
/// what the first run produced — never a second row.
///
/// **Falsified by**: `resumed_at` naming any phase but the one just rewound;
/// any phase left not-`done` after the second run; the server's row count for
/// the seeded record changing at all.
#[test]
fn interrupting_any_phase_resumes_exactly_there_and_creates_no_duplicate() {
    let r = resumable!();

    let run1 = r.s.json(&["migrate", "--run"]);
    assert_eq!(
        run1["run"]["mode"],
        json!("server_authoritative"),
        "the baseline run must complete: {run1}"
    );
    let baseline = r.server.count(&format!(
        "SELECT count(*) FROM personal_knowledge WHERE owner_user_id = '{}'",
        r.account
    ));
    assert_eq!(
        baseline, 1,
        "the seeded row was never delivered by the baseline run"
    );

    for phase in PHASES {
        rewind_from(&r.s, phase);
        let run2 = r.s.json(&["migrate", "--run"]);
        assert_eq!(
            run2["run"]["resumed_at"],
            json!(phase),
            "rewinding {phase} (and everything after it) must resume exactly there: {run2}"
        );

        for p in phase_states(&r.s) {
            assert_eq!(
                p["state"],
                json!("done"),
                "phase {} did not complete after resuming at {phase}: {p}",
                p["phase"]
            );
        }

        let after = r.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge WHERE owner_user_id = '{}'",
            r.account
        ));
        assert_eq!(
            after, baseline,
            "resuming at {phase} produced duplicate canonical knowledge ({after} rows, baseline {baseline})"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. A killed daemon mid-migration resumes rather than restarts
// ---------------------------------------------------------------------------

/// After a completed migration, killing and restarting the daemon and running
/// again must not re-enter any phase that was already `done` — each keeps the
/// `started_at` its one real execution wrote, and the second run resumes
/// nowhere at all.
///
/// **Falsified by**: any phase's `started_at` changing, or `resumed_at` naming
/// a phase after a restart that changed nothing about the migration's state.
#[test]
fn a_killed_daemon_mid_migration_resumes_rather_than_restarts() {
    let r = resumable!();

    let run1 = r.s.json(&["migrate", "--run"]);
    assert_eq!(run1["run"]["mode"], json!("server_authoritative"));

    let before = phase_states(&r.s);
    assert!(
        !before.is_empty() && before.iter().all(|p| p["state"] == json!("done")),
        "the baseline run did not finish every phase: {before:?}"
    );
    let started_before: std::collections::BTreeMap<String, Value> = before
        .iter()
        .map(|p| {
            (
                p["phase"].as_str().expect("a phase name").to_string(),
                p["started_at"].clone(),
            )
        })
        .collect();

    r.s.stop_daemon();
    r.s.restart_daemon();

    let run2 = r.s.json(&["migrate", "--run"]);
    assert_eq!(
        run2["run"]["resumed_at"],
        Value::Null,
        "a fully completed migration has nothing left to resume, restart or not: {run2}"
    );

    for p in phase_states(&r.s) {
        let phase = p["phase"].as_str().expect("a phase name");
        assert_eq!(
            p["state"],
            json!("done"),
            "phase {phase} regressed after a restart"
        );
        assert_eq!(
            p["started_at"], started_before[phase],
            "phase {phase} was already done before the restart; its started_at changed, \
             meaning it was re-entered rather than skipped"
        );
    }
}

// ---------------------------------------------------------------------------
// Fixture 2: a real, populated Feature 004 store, for re-keying
// ---------------------------------------------------------------------------

struct Legacy {
    s: Sandbox,
    server: Server,
    ids: LegacyIds,
}

fn legacy() -> Option<Legacy> {
    let server = Server::start()?;
    let s = Sandbox::new();
    let (account, token) = server.new_user("migrating");
    let ids = install_legacy_v7(&s, account);
    attach_server(&s, &server, &token);
    s.must(&["link", "--create"]);
    Some(Legacy { s, server, ids })
}

macro_rules! legacy {
    () => {
        match legacy() {
            Some(l) => l,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

// ---------------------------------------------------------------------------
// 3. Legacy keys are normalized through the shared normalizer
// ---------------------------------------------------------------------------

/// After `--run`, a legacy project memory's `topic_key` reads exactly as
/// `cairn_core::knowledge::normalize_topic_key` renders its original,
/// un-normalized value — never a hardcoded string — and the same holds for a
/// legacy personal-knowledge row's `topic_key` and `value_key`.
///
/// **Falsified by**: any of the four keys disagreeing with the value the
/// shipped normalizer itself produces from the original input.
#[test]
fn legacy_keys_are_renormalized_through_the_shared_normalizer() {
    let l = legacy!();
    let _ = l.s.json(&["migrate", "--run"]);

    let topic = l.s.query_column(&format!(
        "SELECT topic_key FROM memories WHERE id = '{}'",
        l.ids.memory_queued
    ));
    let value = l.s.query_column(&format!(
        "SELECT value_key FROM memories WHERE id = '{}'",
        l.ids.memory_queued
    ));
    assert_eq!(
        topic,
        vec![
            cairn_core::knowledge::normalize_topic_key("Release.Signing ")
                .expect("the seeded topic is normalizable")
        ],
        "memory_queued's topic_key was not re-keyed through the shipped normalizer"
    );
    assert_eq!(
        value,
        vec![cairn_core::knowledge::normalize_value_key("Cosign")
            .expect("the seeded value is normalizable")],
        "memory_queued's value_key was not re-keyed through the shipped normalizer"
    );

    let p_topic = l.s.query_column(&format!(
        "SELECT topic_key FROM personal_knowledge WHERE id = '{}'",
        l.ids.personal_unnormalized
    ));
    let p_value = l.s.query_column(&format!(
        "SELECT value_key FROM personal_knowledge WHERE id = '{}'",
        l.ids.personal_unnormalized
    ));
    assert_eq!(
        p_topic,
        vec![cairn_core::knowledge::normalize_topic_key("Notes.Layout  ")
            .expect("the seeded topic is normalizable")],
        "personal_unnormalized's topic_key was not re-keyed through the shipped normalizer"
    );
    assert_eq!(
        p_value,
        vec![cairn_core::knowledge::normalize_value_key("Day File")
            .expect("the seeded value is normalizable")],
        "personal_unnormalized's value_key was not re-keyed through the shipped normalizer"
    );
}

// ---------------------------------------------------------------------------
// 4. `.` survives — it is a segment separator, not content to fold
// ---------------------------------------------------------------------------

/// A topic key containing a `.` keeps its `.` after re-keying. Folding it
/// would rewrite `test.command` to `test_command` across every record that
/// uses a dotted topic — this is the one behaviour re-keying must NOT change.
///
/// **Falsified by**: the seeded row's `topic_key` losing its `.`, or
/// disagreeing with what the shipped normalizer itself produces.
#[test]
fn a_dot_in_a_topic_key_is_a_segment_separator_and_survives_renormalization() {
    let l = legacy!();
    let extra = Uuid::now_v7();
    l.s.exec_sql(&format!(
        "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state,
                               origin_session_id, local_only, created_at, updated_at,
                               topic_key, value_key)
         VALUES ('{extra}', '{}', 'fact', 'project', '{}',
                 'run test.command before packaging', 'active', '{}', 0,
                 '2026-08-01T09:00:00Z', '2026-08-01T09:00:00Z', 'test.command', 'always')",
        l.ids.project, l.ids.project, l.ids.session
    ));

    let _ = l.s.json(&["migrate", "--run"]);

    let topic = l.s.query_column(&format!(
        "SELECT topic_key FROM memories WHERE id = '{extra}'"
    ));
    let expected =
        cairn_core::knowledge::normalize_topic_key("test.command").expect("normalizable");
    assert_eq!(topic, vec![expected.clone()]);
    assert!(
        expected.contains('.'),
        "folding the dot would rewrite `test.command` to `test_command` across every \
         record with a dotted topic: got {expected}"
    );
}

// ---------------------------------------------------------------------------
// 5. A collision becomes a conflict, not a deletion
// ---------------------------------------------------------------------------

/// `memory_queued` and `memory_collides` normalize onto one topic key with
/// different value keys. After `--run`, both rows still exist, and a
/// `memory_relations` row with kind `conflicts_with` and basis
/// `deterministic_rule` relates them.
///
/// **Falsified by**: either row disappearing, or no such relation existing.
#[test]
fn a_collision_on_the_normalized_key_becomes_a_conflict_not_a_deletion() {
    let l = legacy!();
    let _ = l.s.json(&["migrate", "--run"]);

    for id in [l.ids.memory_queued, l.ids.memory_collides] {
        let n = l.s.query_column(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM memories WHERE id = '{id}' AND deleted_at IS NULL"
        ));
        assert_eq!(
            n,
            vec!["1".to_string()],
            "migration discarded one side of a key collision instead of surfacing a conflict: {id}"
        );
    }

    let a = l.ids.memory_queued;
    let b = l.ids.memory_collides;
    let conflicts = l.s.query_column(&format!(
        "SELECT CAST(count(*) AS TEXT) FROM memory_relations
          WHERE kind = 'conflicts_with' AND basis = 'deterministic_rule'
            AND ((from_memory_id = '{a}' AND to_memory_id = '{b}')
              OR  (from_memory_id = '{b}' AND to_memory_id = '{a}'))"
    ));
    assert_eq!(
        conflicts,
        vec!["1".to_string()],
        "the two colliding memories were never related as a conflict"
    );
}

// ---------------------------------------------------------------------------
// 6. A duplicate is not a conflict
// ---------------------------------------------------------------------------

/// Two records that normalize onto the same topic key AND the same value key
/// are an ordinary duplicate, reported as one, and never turned into a
/// `conflicts_with` relation.
///
/// **Falsified by**: `keys.duplicates` disagreeing with the one duplicate
/// pair this fixture introduces, or a `conflicts_with` relation appearing
/// between them.
#[test]
fn a_duplicate_on_the_normalized_key_is_not_a_conflict() {
    let l = legacy!();
    let dup_a = Uuid::now_v7();
    let dup_b = Uuid::now_v7();
    // Different spellings of the same topic and the same value — folding
    // agrees on both, which is what makes this a duplicate rather than a
    // collision.
    for (id, topic, value) in [
        (dup_a, "Docker Registry", "Docker Hub"),
        (dup_b, "docker-registry", "docker_hub"),
    ] {
        l.s.exec_sql(&format!(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state,
                                   origin_session_id, local_only, created_at, updated_at,
                                   topic_key, value_key)
             VALUES ('{id}', '{}', 'fact', 'project', '{}',
                     'a duplicate fixture row', 'active', '{}', 0,
                     '2026-08-01T09:00:00Z', '2026-08-01T09:00:00Z', '{topic}', '{value}')",
            l.ids.project, l.ids.project, l.ids.session
        ));
    }

    let run = l.s.json(&["migrate", "--run"]);
    assert_eq!(
        run["run"]["keys"]["duplicates"],
        json!(1),
        "the run must report exactly the one duplicate this fixture introduces: {run}"
    );

    let conflicts = l.s.query_column(&format!(
        "SELECT CAST(count(*) AS TEXT) FROM memory_relations
          WHERE kind = 'conflicts_with'
            AND ((from_memory_id = '{dup_a}' AND to_memory_id = '{dup_b}')
              OR  (from_memory_id = '{dup_b}' AND to_memory_id = '{dup_a}'))"
    ));
    assert_eq!(
        conflicts,
        vec!["0".to_string()],
        "a true duplicate (same key, same value) must never become a conflict"
    );
    for id in [dup_a, dup_b] {
        let n = l.s.query_column(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM memories WHERE id = '{id}'"
        ));
        assert_eq!(
            n,
            vec!["1".to_string()],
            "a duplicate must not be discarded either"
        );
    }
}

// ---------------------------------------------------------------------------
// 7. Zero duplicate rows across repeated runs
// ---------------------------------------------------------------------------

/// The server's row counts for every record this fixture can deliver are
/// established by the first `--run` and are byte-for-byte unchanged by a
/// second and a third.
///
/// **Falsified by**: any count changing between the first run and the
/// third, or the first run itself delivering nothing (which would make
/// "unchanged" vacuous).
#[test]
fn running_the_migration_three_times_leaves_row_counts_unchanged_after_the_first() {
    let l = legacy!();
    let claim = l.s.json(&[
        "migrate",
        "--claim-patterns",
        &l.ids.pattern_claimable.to_string(),
    ]);
    let pattern_id = claim["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["local_pattern_id"] == json!(l.ids.pattern_claimable.to_string()))
        .expect("the claimed pattern's own row")["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let counts = |l: &Legacy, pattern_id: &str| -> (i64, i64, i64, i64, i64) {
        (
            l.server.count(&format!(
                "SELECT count(*) FROM memories WHERE id IN ('{}','{}','{}','{}')",
                l.ids.memory_queued, l.ids.memory_collides, l.ids.relation.0, l.ids.relation.1
            )),
            l.server.count(&format!(
                "SELECT count(*) FROM memory_relations
                  WHERE from_memory_id = '{}' AND to_memory_id = '{}' AND kind = '{}'",
                l.ids.relation.0, l.ids.relation.1, l.ids.relation.2
            )),
            l.server.count(&format!(
                "SELECT count(*) FROM personal_knowledge WHERE id IN ('{}','{}')",
                l.ids.personal_queued, l.ids.personal_unnormalized
            )),
            l.server.count(&format!(
                "SELECT count(*) FROM team_knowledge WHERE id = '{}'",
                l.ids.team_authoritative
            )),
            l.server.count(&format!(
                "SELECT count(*) FROM shared_patterns WHERE pattern_id = '{pattern_id}'"
            )),
        )
    };

    let zero = counts(&l, &pattern_id);
    assert_eq!(
        zero,
        (0, 0, 0, 0, 0),
        "the fixture's rows must not already exist on the server before any run: {zero:?}"
    );

    let _ = l.s.json(&["migrate", "--run"]);
    let after_one = counts(&l, &pattern_id);
    assert_eq!(
        after_one,
        (4, 1, 2, 1, 1),
        "the first run did not deliver everything it should have (memories, relation, \
         personal rows, the authoritative team row, the claimed pattern): {after_one:?}"
    );

    for attempt in 2..=3 {
        let _ = l.s.json(&["migrate", "--run"]);
        let again = counts(&l, &pattern_id);
        assert_eq!(
            again, after_one,
            "run #{attempt} changed the server's row counts: {again:?} vs. the first run's {after_one:?}"
        );
    }
}

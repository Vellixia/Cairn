//! US1 — canonical project knowledge, against a real store.
//!
//! The two negatives this slice exists to prevent (T025):
//!
//! - **no automatic reinforcement** — no code path writes a `reinforces`
//!   relation without an explicit request (FR-321, metric 2a);
//! - **corroboration** — a shared value key with differing content yields
//!   `Corroborated`, records nothing, and retains every statement (FR-327,
//!   metric 2b).
//!
//! Both were written before T032 added the explicit `reinforce` path, and both
//! must keep passing after it: the point is not that reinforcement is
//! impossible, it is that it never happens by itself.
//!
//! Tier 2 already checks the same properties over the corpus as pure functions.
//! This is the store: what the *write path* decides when a proposal arrives,
//! including what it persists.

use cairn_core::corpus;
use cairn_core::knowledge::ProposalOutcome;
use cairn_e2e::store_fixture::Fixture;
use uuid::Uuid;

/// Metric 2a — zero unrequested `reinforces` relations, over the whole
/// derivation corpus driven through the real write path.
#[test]
fn no_automatic_reinforcement() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let mut proposals = 0usize;

        for group in [
            "reconciliation/equivalent",
            "reconciliation/distinct",
            "reconciliation/coarse_value_key",
            "reconciliation/duplicate_content",
            "reconciliation/free_form",
            "conflict/real",
        ] {
            for case in corpus::load_group(&corpus::root(), group).expect("group loads") {
                // Each case gets its own topic namespace, so one fixture's
                // members never reconcile against another's.
                let namespace = case.name.replace(['-'], "_");
                for m in &case.input.memories {
                    let topic = m
                        .topic_key
                        .as_deref()
                        .map(|t| format!("{namespace}.{t}"));
                    f.propose(
                        Uuid::now_v7(),
                        topic.as_deref(),
                        m.value_key.as_deref(),
                        &m.content,
                    )
                    .await;
                    proposals += 1;
                }
            }
        }

        assert!(proposals > 200, "only {proposals} proposals exercised");
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'reinforces'")
                .await,
            0,
            "an automatic path wrote a reinforcement across {proposals} proposals"
        );

        // And the relations it *did* write are only the two kinds automatic
        // reconciliation may decide.
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT kind FROM memory_relations ORDER BY kind",
        )
        .fetch_all(f.store.pool())
        .await
        .expect("kinds");
        for kind in &kinds {
            assert!(
                kind == "duplicates" || kind == "conflicts_with",
                "automatic reconciliation recorded a {kind} relation"
            );
        }
    });
}

/// Metric 2b — every adversarial coarse-value-key case corroborates, records
/// nothing, and keeps both statements retrievable.
///
/// This is the false-merge path R12 closed, driven through the store. A case
/// here that yields `Reinforced` is a defect: it would report a reinforcement
/// that never happened and suppress one of two honest claims.
#[test]
fn corroboration() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let cases = corpus::load_group(&corpus::root(), "reconciliation/coarse_value_key")
            .expect("the coarse-value-key corpus loads");
        assert!(cases.len() >= 15, "{} cases", cases.len());

        for case in &cases {
            let namespace = case.name.replace(['-'], "_");
            let mut ids = Vec::new();
            let mut topics = Vec::new();

            for (i, m) in case.input.memories.iter().enumerate() {
                let topic = format!(
                    "{namespace}.{}",
                    m.topic_key.as_deref().expect("a coarse-value case is keyed")
                );
                let out = f
                    .propose(
                        Uuid::now_v7(),
                        Some(&topic),
                        m.value_key.as_deref(),
                        &m.content,
                    )
                    .await;

                if i > 0 {
                    assert!(
                        matches!(out.reconciliation, ProposalOutcome::Corroborating { .. }),
                        "{}",
                        case.context(format!(
                            "expected corroboration, got {:?}",
                            out.reconciliation
                        ))
                    );
                    assert!(
                        out.notes.contains(&"corroborating_member"),
                        "{}",
                        case.context("the writer was not told which member it matched")
                    );
                }
                ids.push(out.memory.id);
                topics.push(topic);
            }

            // Nothing was recorded for this subject at all.
            let recorded = f
                .count(&format!(
                    "SELECT COUNT(*) FROM memory_relations
                      WHERE from_memory_id IN ('{}') OR to_memory_id IN ('{}')",
                    ids.iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join("','"),
                    ids.iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join("','")
                ))
                .await;
            assert_eq!(
                recorded,
                0,
                "{}",
                case.context("corroboration recorded a relation")
            );

            // The subject reports the value as agreed and the statements as
            // several, and every one is still there.
            let read = f.subject(&topics[0]).await;
            assert_eq!(
                read.view.reconciliation.as_str(),
                "corroborated",
                "{}",
                case.context("the subject did not corroborate")
            );
            assert_eq!(
                read.view.answers.len(),
                ids.len(),
                "{}",
                case.context("a statement was dropped from the answer set")
            );
            for id in &ids {
                assert!(
                    read.members.iter().any(|m| m.id == *id),
                    "{}",
                    case.context("a member is no longer retrievable")
                );
            }
        }
    });
}

/// Scenario A — three sessions record the same thing, and the briefing gets one
/// answer with the duplication accounted for.
#[test]
fn three_sessions_yield_one_canonical_answer_with_three_origins() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let first = f
            .propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("postgresql"),
                "The production database is PostgreSQL.",
            )
            .await;
        for restatement in [
            "the production   database is postgresql!",
            "THE PRODUCTION DATABASE IS POSTGRESQL",
        ] {
            f.propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("postgresql"),
                restatement,
            )
            .await;
        }

        let read = f.subject("infrastructure.production_database").await;
        assert_eq!(read.view.reconciliation.as_str(), "reinforced");
        assert_eq!(read.view.answers, vec![first.memory.id]);
        assert_eq!(read.view.accounting[0].duplicates.len(), 2);
        assert_eq!(
            read.view.accounting[0].distinct_origins, 3,
            "three sessions is three origins"
        );

        // All three are still individually retrievable with their own
        // provenance: a duplicate leaves the answer set, never the store.
        assert_eq!(read.members.len(), 3);
        let origins = f
            .count("SELECT COUNT(DISTINCT origin_session_id) FROM memories")
            .await;
        assert_eq!(origins, 3);
    });
}

/// Scenario B — a project answer and a task-scoped exception are not a
/// conflict, and each applies where it should.
#[test]
fn a_task_scoped_exception_is_not_a_conflict() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        f.propose(
            Uuid::now_v7(),
            Some("infrastructure.production_database"),
            Some("postgresql"),
            "The production database is PostgreSQL.",
        )
        .await;

        let task = f
            .propose_scoped(
                Uuid::now_v7(),
                cairn_core::MemoryScope::Task,
                Some("T1"),
                Some("infrastructure.production_database"),
                Some("sqlite"),
                "This integration fixture uses SQLite.",
            )
            .await;

        assert_eq!(task.reconciliation, ProposalOutcome::Created);
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'conflicts_with'")
                .await,
            0,
            "a scope exception was recorded as a conflict"
        );

        let project = f.subject("infrastructure.production_database").await;
        assert_eq!(project.view.reconciliation.as_str(), "settled");
    });
}

/// FR-312 — an unrepresentable topic key never rejects the memory.
#[test]
fn an_unusable_topic_key_stores_the_memory_free_form() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let out = f
            .propose(
                Uuid::now_v7(),
                Some("데이터베이스"),
                None,
                "A claim whose proposed key has no representable characters.",
            )
            .await;

        assert!(out.notes.contains(&"invalid_topic_key"));
        assert_eq!(f.count("SELECT COUNT(*) FROM memories").await, 1);
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memories WHERE topic_key IS NULL")
                .await,
            1,
            "the memory was stored, free-form, exactly as FR-312 requires"
        );
    });
}

// ---------------------------------------------------------------------------
// T042 — the same behaviour through the whole path: CLI, daemon, store
// ---------------------------------------------------------------------------

/// The US1 quickstart, driven the way a developer drives it.
#[test]
fn the_cli_records_a_subject_and_explains_it() {
    let s = cairn_e2e::Sandbox::new();

    let add = |content: &str, value: &str| {
        s.cairn(&[
            "memory",
            "add",
            content,
            "--type",
            "decision",
            "--scope",
            "project",
            "--topic-key",
            "infrastructure.production_database",
            "--value-key",
            value,
            "--json",
        ])
    };

    let first = add("The production database is PostgreSQL.", "postgresql");
    assert!(first.ok(), "{}", first.stderr);

    // The same claim again, worded differently: a duplicate, and the response
    // says so.
    let second = add("the production   database is postgresql!", "postgresql");
    assert!(second.ok(), "{}", second.stderr);
    assert!(
        second.stdout.contains("duplicate"),
        "the writer was not told it duplicated an existing member: {}",
        second.stdout
    );

    // A different value: a conflict, visible and unresolved.
    let third = add("The production database is CockroachDB.", "cockroachdb");
    assert!(third.ok(), "{}", third.stderr);
    assert!(
        third.stdout.contains("conflict_detected"),
        "the writer was not told it conflicts: {}",
        third.stdout
    );

    let subject = s.cairn(&["memory", "subject", "infrastructure.production_database"]);
    assert!(subject.ok(), "{}", subject.stderr);
    assert!(
        subject.stdout.contains("conflicted"),
        "the subject does not report the conflict: {}",
        subject.stdout
    );
    assert!(
        subject.stdout.contains("no winner"),
        "the rendering does not say there is no winner: {}",
        subject.stdout
    );
}

/// A coarse value key, end to end: the writer is told which member it matched
/// and nothing is recorded.
#[test]
fn the_cli_reports_a_corroborating_member() {
    let s = cairn_e2e::Sandbox::new();

    let first = s.cairn(&[
        "memory", "add", "JWT uses HS256 with a shared secret.",
        "--scope", "project", "--topic-key", "auth.strategy", "--value-key", "jwt", "--json",
    ]);
    assert!(first.ok(), "{}", first.stderr);

    let second = s.cairn(&[
        "memory", "add", "JWT uses RS256 with rotating public keys.",
        "--scope", "project", "--topic-key", "auth.strategy", "--value-key", "jwt", "--json",
    ]);
    assert!(second.ok(), "{}", second.stderr);
    assert!(
        second.stdout.contains("corroborating"),
        "{}",
        second.stdout
    );
    assert!(
        second.stdout.contains("corroborating_member"),
        "the note that prompts an explicit decision is missing: {}",
        second.stdout
    );

    let subject = s.cairn(&["memory", "subject", "auth.strategy"]);
    assert!(subject.stdout.contains("corroborated"), "{}", subject.stdout);
    assert!(
        subject.stdout.contains("statements are several"),
        "{}",
        subject.stdout
    );
}

/// An unusable topic key never rejects the memory, through the CLI too.
#[test]
fn the_cli_stores_a_memory_whose_key_cannot_be_represented() {
    let s = cairn_e2e::Sandbox::new();
    let out = s.cairn(&[
        "memory", "add", "A claim with an unusable key.",
        "--topic-key", "데이터베이스", "--json",
    ]);
    assert!(out.ok(), "the memory was rejected: {}", out.stderr);
    assert!(out.stdout.contains("invalid_topic_key"), "{}", out.stdout);

    let found = s.cairn(&["memory", "search", "unusable", "--json"]);
    assert!(found.stdout.contains("A claim with an unusable key."), "{}", found.stdout);
}

/// Reinforcement is explicit, and its counts are never called verifications.
#[test]
fn the_cli_reinforces_only_when_asked() {
    let s = cairn_e2e::Sandbox::new();

    let target = s.cairn(&[
        "memory", "add", "Errors are returned, never logged and swallowed.",
        "--type", "convention", "--scope", "project", "--topic-key", "error.handling",
        "--value-key", "returned", "--json",
    ]);
    assert!(target.ok(), "{}", target.stderr);
    let target_id = extract_id(&target.stdout);

    let confirming = s.cairn(&[
        "memory", "add", "Confirmed while reviewing the retry path.",
        "--type", "fact", "--json",
    ]);
    let confirming_id = extract_id(&confirming.stdout);

    let out = s.cairn(&[
        "memory", "reinforce", &target_id, "--from", &confirming_id, "--json",
    ]);
    assert!(out.ok(), "{}", out.stderr);
    assert!(out.stdout.contains("\"reinforcements\": 1"), "{}", out.stdout);
    assert!(
        out.stdout.contains("distinct_origins"),
        "the accounting does not distinguish origins: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("verifications"),
        "a reinforcement count was presented as a verification count: {}",
        out.stdout
    );
}

/// A conflict is resolved by an explicit decision, never by Cairn.
#[test]
fn the_cli_resolves_a_conflict_by_superseding() {
    let s = cairn_e2e::Sandbox::new();

    let old = s.cairn(&[
        "memory", "add", "The API listens on 8080.",
        "--scope", "project", "--topic-key", "service.api_port", "--value-key", "8080", "--json",
    ]);
    let old_id = extract_id(&old.stdout);
    let new = s.cairn(&[
        "memory", "add", "The API listens on 9000.",
        "--scope", "project", "--topic-key", "service.api_port", "--value-key", "9000", "--json",
    ]);
    let new_id = extract_id(&new.stdout);

    let before = s.cairn(&["memory", "subject", "service.api_port"]);
    assert!(before.stdout.contains("conflicted"), "{}", before.stdout);

    let resolved = s.cairn(&[
        "memory", "reconcile", "--from", &new_id, "--to", &old_id,
        "--relation", "supersedes", "--basis", "explicit_user", "--json",
    ]);
    assert!(resolved.ok(), "{}", resolved.stderr);

    let after = s.cairn(&["memory", "subject", "service.api_port"]);
    assert!(after.stdout.contains("settled"), "{}", after.stdout);

    // And the reverse decision is refused rather than creating a cycle.
    let cycle = s.cairn(&[
        "memory", "reconcile", "--from", &old_id, "--to", &new_id,
        "--relation", "supersedes", "--basis", "explicit_user", "--json",
    ]);
    assert!(!cycle.ok(), "a mutual supersession was accepted");
    assert!(cycle.stdout.contains("relation_conflict") || cycle.stderr.contains("relation_conflict"),
            "stdout={} stderr={}", cycle.stdout, cycle.stderr);
}

fn extract_id(json: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(json).expect("json");
    v["data"]["memory"]["id"]
        .as_str()
        .or_else(|| v["memory"]["id"].as_str())
        .unwrap_or_else(|| panic!("no memory id in {json}"))
        .to_string()
}

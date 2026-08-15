//! US3 — conflicts are visible, and nothing decides them (T026).
//!
//! `no_clock_arbitration` is the mutation-style proof behind FR-303 and D49: a
//! subject is built whose members' `created_at`, `updated_at` and identifier
//! order all disagree with one another, derived, then rebuilt with every
//! timestamp inverted — and the two derivations must be identical.
//!
//! The pure types already make this hard to get wrong: `MemoryFacts` carries no
//! timestamp, so `derive_subject` cannot read one. This test closes the other
//! half, that the **store** does not smuggle one in through the query it feeds
//! the derivation.

use cairn_core::knowledge::SubjectView;
use cairn_e2e::store_fixture::Fixture;
use uuid::Uuid;

fn view_json(view: &SubjectView) -> String {
    serde_json::to_string(view).expect("a SubjectView serializes")
}

/// Metric 7's local half — no clock and no identifier order decides a winner.
#[test]
fn no_clock_arbitration() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // Three members of one subject, deliberately disagreeing on every
        // ordering a naive implementation might reach for.
        let a = f
            .propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("postgresql"),
                "The production database is PostgreSQL.",
            )
            .await;
        let b = f
            .propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("cockroachdb"),
                "The production database is CockroachDB.",
            )
            .await;
        let c = f
            .propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("mysql"),
                "The production database is MySQL.",
            )
            .await;

        // Identifier order is a < b < c (UUIDv7 is time-ordered). Give the
        // clocks the opposite order, so "newest" and "highest id" disagree.
        f.set_clock(a.memory.id, "2026-03-01T00:00:00Z", "2026-03-01T00:00:00Z")
            .await;
        f.set_clock(b.memory.id, "2026-02-01T00:00:00Z", "2026-02-15T00:00:00Z")
            .await;
        f.set_clock(c.memory.id, "2026-01-01T00:00:00Z", "2026-01-15T00:00:00Z")
            .await;

        let first = f.subject("infrastructure.production_database").await;
        assert_eq!(
            first.view.reconciliation.as_str(),
            "conflicted",
            "three incompatible values in one scope is a conflict"
        );
        assert_eq!(first.view.answers.len(), 3, "no winner was picked");

        // Now invert every timestamp and derive again.
        f.set_clock(a.memory.id, "2026-01-01T00:00:00Z", "2026-01-15T00:00:00Z")
            .await;
        f.set_clock(b.memory.id, "2026-02-01T00:00:00Z", "2026-02-15T00:00:00Z")
            .await;
        f.set_clock(c.memory.id, "2026-03-01T00:00:00Z", "2026-03-01T00:00:00Z")
            .await;

        let second = f.subject("infrastructure.production_database").await;

        assert_eq!(
            view_json(&first.view),
            view_json(&second.view),
            "inverting every clock changed the derivation, so something read one"
        );
    });
}

/// A conflict is reported, never resolved — and every member keeps its state.
#[test]
fn a_conflict_leaves_every_member_active_and_attributed() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let one = Uuid::now_v7();
        let two = Uuid::now_v7();
        let a = f
            .propose(one, Some("service.api_port"), Some("8080"), "The API listens on 8080.")
            .await;
        let b = f
            .propose(two, Some("service.api_port"), Some("9000"), "The API listens on 9000.")
            .await;

        let read = f.subject("service.api_port").await;
        assert_eq!(read.view.reconciliation.as_str(), "conflicted");
        assert_eq!(read.view.answers.len(), 2, "both competing answers returned");

        // Neither was marked superseded to make the conflict go away.
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memories WHERE state <> 'active'")
                .await,
            0
        );
        // And each keeps the session that proposed it.
        let origins: Vec<String> = sqlx::query_scalar(
            "SELECT origin_session_id FROM memories ORDER BY id",
        )
        .fetch_all(f.store.pool())
        .await
        .expect("origins");
        assert_eq!(origins, vec![one.to_string(), two.to_string()]);
        assert!(read.members.iter().any(|m| m.id == a.memory.id));
        assert!(read.members.iter().any(|m| m.id == b.memory.id));
    });
}

/// The two conflict kinds stay separate (FR-331).
///
/// A **semantic** conflict is a derived subject state — two applicable answers
/// that disagree. A **concurrent write** is absorbed by the write lock and the
/// relation primary key, and produces no state of its own. Feature 003
/// introduces no second mechanism for either.
#[test]
fn a_semantic_conflict_and_a_concurrent_write_are_different_things() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // Semantic: two disagreeing proposals, recorded as a conflict.
        f.propose(Uuid::now_v7(), Some("cache.backend"), Some("redis"), "Redis.")
            .await;
        f.propose(
            Uuid::now_v7(),
            Some("cache.backend"),
            Some("memcached"),
            "Memcached.",
        )
        .await;
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'conflicts_with'")
                .await,
            1
        );

        // Concurrent: the same decision recorded twice. The primary key absorbs
        // the second, and no second row and no error appears.
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM memories ORDER BY id")
            .fetch_all(f.store.pool())
            .await
            .expect("ids");
        let duplicate = cairn_store::knowledge::NewRelation {
            project_id: f.project,
            from: ids[1].parse().expect("uuid"),
            to: ids[0].parse().expect("uuid"),
            kind: cairn_core::RelationKind::ConflictsWith,
            decided_by_session: Uuid::now_v7(),
            basis: cairn_core::RelationBasis::DeterministicRule,
            basis_evidence_id: None,
            rationale: None,
        };
        let wrote = cairn_store::knowledge::record_relation(&f.store, duplicate)
            .await
            .expect("recording an existing decision is not an error");
        assert!(!wrote, "a second row was written for one fact");
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations WHERE kind = 'conflicts_with'")
                .await,
            1
        );
    });
}

/// Cairn never resolves a conflict on its own (FR-334).
///
/// Standing is a reported state, not an error. Deriving the same subject
/// repeatedly must not quietly settle it.
#[test]
fn a_standing_conflict_never_resolves_itself() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        f.propose(Uuid::now_v7(), Some("deploy.target"), Some("dokploy"), "Dokploy.")
            .await;
        f.propose(Uuid::now_v7(), Some("deploy.target"), Some("flyio"), "Fly.io.")
            .await;

        for _ in 0..5 {
            let read = f.subject("deploy.target").await;
            assert_eq!(read.view.reconciliation.as_str(), "conflicted");
            assert_eq!(read.view.answers.len(), 2);
        }

        // And nothing was written by reading.
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations").await,
            1,
            "deriving a subject wrote a decision"
        );
    });
}

/// SC-303 / metric 5 — 32 separate processes propose against one subject.
///
/// The assertion is 32 persisted proposals, zero lost writes, and a derivation
/// whose outcome does not depend on commit order. Concurrency is absorbed by
/// `BEGIN IMMEDIATE` and the relation primary key; Feature 003 introduces no
/// second mechanism for it (FR-336).
#[test]
fn concurrent_proposals() {
    let s = cairn_e2e::Sandbox::new();

    // Every proposal is a distinct statement about one subject, so the write
    // path has real work to do for each: none is a no-op duplicate.
    let outcomes: Vec<cairn_e2e::CliResult> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let s = &s;
                scope.spawn(move || {
                    s.cairn(&[
                        "memory",
                        "add",
                        &format!("Concurrent proposal number {i}."),
                        "--scope",
                        "project",
                        "--topic-key",
                        "infrastructure.production_database",
                        "--value-key",
                        &format!("candidate-{i}"),
                        "--json",
                    ])
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread")).collect()
    });

    let failed: Vec<&cairn_e2e::CliResult> = outcomes.iter().filter(|o| !o.ok()).collect();
    assert!(
        failed.is_empty(),
        "{} of 32 proposals failed: {:?}",
        failed.len(),
        failed.iter().map(|f| &f.stderr).collect::<Vec<_>>()
    );

    // Zero lost writes.
    let persisted = s.query_column(
        "SELECT CAST(COUNT(*) AS TEXT) FROM memories
          WHERE topic_key = 'infrastructure.production_database'",
    );
    assert_eq!(
        persisted,
        vec!["32".to_string()],
        "a write was lost under 32-way concurrency"
    );

    // Every one keeps its own identity and its own content.
    let distinct = s.query_column(
        "SELECT CAST(COUNT(DISTINCT content) AS TEXT) FROM memories
          WHERE topic_key = 'infrastructure.production_database'",
    );
    assert_eq!(distinct, vec!["32".to_string()]);

    // And the subject reports every one of them, with no winner.
    let subject = s.cairn(&[
        "memory",
        "subject",
        "infrastructure.production_database",
        "--json",
    ]);
    assert!(subject.ok(), "{}", subject.stderr);
    let v: serde_json::Value = serde_json::from_str(&subject.stdout).expect("json");
    let view = &v["data"]["subject"];
    assert_eq!(
        view["reconciliation"].as_str(),
        Some("conflicted"),
        "{}",
        subject.stdout
    );
    assert_eq!(
        view["answers"].as_array().map(|a| a.len()),
        Some(32),
        "the derivation dropped a proposal"
    );
}

/// The outcome does not depend on which write committed first.
///
/// Two orders, two sandboxes, the same derived shape. A conflict set that moved
/// with commit order would mean something arbitrated on arrival.
#[test]
fn the_outcome_does_not_depend_on_commit_order() {
    let shape = |reversed: bool| -> (String, usize) {
        let s = cairn_e2e::Sandbox::new();
        let mut values = vec![("postgresql", "PostgreSQL."), ("mysql", "MySQL."), ("cockroachdb", "CockroachDB.")];
        if reversed {
            values.reverse();
        }
        for (value, content) in values {
            let out = s.cairn(&[
                "memory", "add", content, "--scope", "project",
                "--topic-key", "infrastructure.production_database",
                "--value-key", value, "--json",
            ]);
            assert!(out.ok(), "{}", out.stderr);
        }
        let subject = s.cairn(&[
            "memory", "subject", "infrastructure.production_database", "--json",
        ]);
        let v: serde_json::Value = serde_json::from_str(&subject.stdout).expect("json");
        let view = &v["data"]["subject"];
        (
            view["reconciliation"].as_str().unwrap_or("?").to_string(),
            view["answers"].as_array().map(|a| a.len()).unwrap_or(0),
        )
    };

    assert_eq!(shape(false), shape(true));
    assert_eq!(shape(false), ("conflicted".to_string(), 3));
}

//! US2 — knowledge evolves safely, and history stays readable.
//!
//! The temporal claim is deliberately narrow (FR-342, D82). Cairn reconstructs
//! **proposal effectiveness** and **explicit supersession intervals**, because
//! those are what it stores authoritatively. A lifecycle transition with no
//! authoritative instant reports `applicability: unknown` rather than
//! presenting an unbounded interval as fact — which is the honest limit of the
//! stored evidence, and weaker than "it applied at T".

use cairn_core::domain::{MemoryScope, MemoryType};
use cairn_core::wire::MemoryQuery;
use cairn_core::{Applicability, MemoryState, RelationBasis, RelationKind};
use cairn_e2e::store_fixture::Fixture;
use cairn_store::search::{search, SearchContext};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

fn query() -> MemoryQuery {
    MemoryQuery {
        query: None,
        scope: None,
        scope_key: None,
        kind: None,
        state: None,
        limit: Some(50),
        topic_key: None,
        as_of: None,
        conflicted: false,
        corroborated: false,
        verification: None,
        authority: None,
    }
}

fn ctx() -> SearchContext {
    SearchContext {
        branch: None,
        task_id: None,
        session_id: None,
    }
}

/// Metric 8 — the historical answer is correct on both sides of a
/// supersession, and the predecessor is byte-identical throughout.
#[test]
fn as_of_returns_what_was_effective_then() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let old = f
            .propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("postgresql"),
                "The production database is PostgreSQL.",
            )
            .await;

        // A snapshot of the predecessor, taken before anything supersedes it.
        let before: Vec<String> = sqlx::query_scalar(
            "SELECT content || '|' || origin_session_id || '|' || created_at || '|' ||
                    COALESCE(topic_key,'') || '|' || COALESCE(value_key,'')
               FROM memories WHERE id = ?1",
        )
        .bind(old.memory.id.to_string())
        .fetch_all(f.store.pool())
        .await
        .expect("snapshot");

        let midpoint = Utc::now();
        // The supersession happens strictly after the midpoint.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        let new = f
            .propose(
                Uuid::now_v7(),
                Some("infrastructure.production_database"),
                Some("cockroachdb"),
                "The production database is CockroachDB.",
            )
            .await;
        cairn_store::knowledge::reconcile(
            &f.store,
            f.project,
            new.memory.id,
            old.memory.id,
            RelationKind::Supersedes,
            RelationBasis::ExplicitUser,
            None,
            None,
        )
        .await
        .expect("supersede");

        // Current knowledge is the successor, and only the successor.
        let current = search(&f.store, f.project, &query(), &ctx())
            .await
            .expect("search");
        let current_ids: Vec<Uuid> = current.iter().map(|r| r.id).collect();
        assert_eq!(current_ids, vec![new.memory.id], "{current_ids:?}");

        // Knowledge as of an instant before the supersession is the
        // predecessor.
        let historical = search(
            &f.store,
            f.project,
            &MemoryQuery {
                as_of: Some(midpoint),
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("historical search");
        let historical_ids: Vec<Uuid> = historical.iter().map(|r| r.id).collect();
        assert_eq!(
            historical_ids,
            vec![old.memory.id],
            "the historical answer is not what was effective then"
        );

        // The response carries the temporal facts, so a caller cannot mistake a
        // historical answer for a current one.
        let t = historical[0].temporal.as_ref().expect("temporal");
        assert!(t.effective_from.is_some());
        assert!(t.superseded_at.is_some(), "the interval has an end");
        assert_eq!(t.applicability, Applicability::Bounded);
        assert!(
            current[0].temporal.is_none(),
            "a current answer carries no historical framing"
        );

        // And the predecessor was not rewritten by any of it (SC-305).
        let after: Vec<String> = sqlx::query_scalar(
            "SELECT content || '|' || origin_session_id || '|' || created_at || '|' ||
                    COALESCE(topic_key,'') || '|' || COALESCE(value_key,'')
               FROM memories WHERE id = ?1",
        )
        .bind(old.memory.id.to_string())
        .fetch_all(f.store.pool())
        .await
        .expect("snapshot");
        assert_eq!(after, before, "the superseded memory's content changed");
    });
}

/// A historical query modifies nothing (FR-343).
#[test]
fn a_historical_query_writes_nothing() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        f.propose(
            Uuid::now_v7(),
            Some("infra.db"),
            Some("postgresql"),
            "PostgreSQL.",
        )
        .await;

        async fn fingerprint(f: &Fixture) -> Vec<String> {
            sqlx::query_scalar(
                "SELECT id || state || COALESCE(superseded_at,'') || COALESCE(stale_at,'')
                   FROM memories ORDER BY id",
            )
            .fetch_all(f.store.pool())
            .await
            .expect("fingerprint")
        }

        let before = fingerprint(&f).await;
        for _ in 0..3 {
            search(
                &f.store,
                f.project,
                &MemoryQuery {
                    as_of: Some(Utc::now()),
                    ..query()
                },
                &ctx(),
            )
            .await
            .expect("historical search");
        }
        assert_eq!(fingerprint(&f).await, before);
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations").await,
            0,
            "a historical query recorded a decision"
        );
    });
}

/// Metric 8a — a transition with no authoritative instant reports unknown
/// applicability, never a bounded fact.
#[test]
fn unknown_applicability() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // A branch-scoped memory whose branch has gone. Marked stale by a build
        // that predates staleness instants — exactly what migration leaves.
        let m = f
            .propose_scoped(
                Uuid::now_v7(),
                MemoryScope::Branch,
                Some("branch/gone"),
                Some("build.tool"),
                Some("cargo"),
                "The workspace builds with Cargo.",
            )
            .await;
        sqlx::query("UPDATE memories SET state = 'stale', stale_at = NULL WHERE id = ?1")
            .bind(m.memory.id.to_string())
            .execute(f.store.pool())
            .await
            .expect("stale");

        let results = search(
            &f.store,
            f.project,
            &MemoryQuery {
                as_of: Some(Utc::now()),
                scope: Some(MemoryScope::Branch),
                scope_key: Some("branch/gone".into()),
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("search");

        assert_eq!(
            results.len(),
            1,
            "the proposal was effective at that instant"
        );
        let t = results[0].temporal.as_ref().expect("temporal");
        assert_eq!(
            t.applicability,
            Applicability::Unknown,
            "a stale memory with no recorded instant claimed a bounded interval"
        );
        assert!(t.stale_at.is_none(), "NULL means unknown, and stays NULL");
    });
}

/// T035 — the maintenance tick records the instant it marks a memory stale,
/// going forward only.
#[test]
fn staleness_is_recorded_going_forward_and_never_inferred() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let live = f
            .propose_scoped(
                Uuid::now_v7(),
                MemoryScope::Branch,
                Some("main"),
                Some("api.style"),
                Some("rest"),
                "The API is REST.",
            )
            .await;
        let doomed = f
            .propose_scoped(
                Uuid::now_v7(),
                MemoryScope::Branch,
                Some("feature/gone"),
                Some("api.style"),
                Some("graphql"),
                "The API is GraphQL.",
            )
            .await;

        // An older memory already stale, with no instant — the migrated shape.
        let historic = f
            .propose_scoped(
                Uuid::now_v7(),
                MemoryScope::Branch,
                Some("branch/ancient"),
                Some("cache.backend"),
                Some("redis"),
                "The cache is Redis.",
            )
            .await;
        sqlx::query("UPDATE memories SET state = 'stale', stale_at = NULL WHERE id = ?1")
            .bind(historic.memory.id.to_string())
            .execute(f.store.pool())
            .await
            .expect("pre-stale");

        let marked =
            cairn_store::repo::mark_stale_scopes(&f.store, f.project, &["main".to_string()])
                .await
                .expect("mark stale");
        assert_eq!(marked, 1, "only the branch that is gone");

        let row: (String, Option<String>) =
            sqlx::query_as("SELECT state, stale_at FROM memories WHERE id = ?1")
                .bind(doomed.memory.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .expect("doomed");
        assert_eq!(row.0, "stale");
        assert!(
            row.1.is_some(),
            "the instant Cairn performed it was not recorded"
        );

        // The live one is untouched.
        let live_state: String = sqlx::query_scalar("SELECT state FROM memories WHERE id = ?1")
            .bind(live.memory.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .expect("live");
        assert_eq!(live_state, "active");

        // And the one that was already stale keeps its unknown instant: nothing
        // is inferred for a transition Cairn did not perform.
        let historic_at: Option<String> =
            sqlx::query_scalar("SELECT stale_at FROM memories WHERE id = ?1")
                .bind(historic.memory.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .expect("historic");
        assert_eq!(historic_at, None);
    });
}

/// T036 — the topic-key filter is an identity match, exact or by prefix.
#[test]
fn the_topic_key_filter_matches_exactly_or_by_prefix() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        for (topic, value) in [
            ("infrastructure.production_database", "postgresql"),
            ("infrastructure.cache", "redis"),
            ("service.api_port", "8080"),
        ] {
            f.propose(
                Uuid::now_v7(),
                Some(topic),
                Some(value),
                &format!("{topic} is {value}."),
            )
            .await;
        }

        let exact = search(
            &f.store,
            f.project,
            &MemoryQuery {
                topic_key: Some("infrastructure.cache".into()),
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("exact");
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].topic_key.as_deref(), Some("infrastructure.cache"));

        let prefix = search(
            &f.store,
            f.project,
            &MemoryQuery {
                topic_key: Some("infrastructure.".into()),
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("prefix");
        assert_eq!(prefix.len(), 2, "a trailing dot matches the subtree");

        let miss = search(
            &f.store,
            f.project,
            &MemoryQuery {
                topic_key: Some("infrastructure".into()),
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("miss");
        assert!(miss.is_empty(), "an exact match is exact");
    });
}

/// T036 — the derived subject filters.
#[test]
fn the_conflicted_and_corroborated_filters_read_the_derivation() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // Conflicted.
        f.propose(
            Uuid::now_v7(),
            Some("infra.db"),
            Some("postgresql"),
            "PostgreSQL.",
        )
        .await;
        f.propose(Uuid::now_v7(), Some("infra.db"), Some("mysql"), "MySQL.")
            .await;
        // Corroborated.
        f.propose(
            Uuid::now_v7(),
            Some("auth.strategy"),
            Some("jwt"),
            "JWT with HS256.",
        )
        .await;
        f.propose(
            Uuid::now_v7(),
            Some("auth.strategy"),
            Some("jwt"),
            "JWT with RS256.",
        )
        .await;
        // Settled.
        f.propose(Uuid::now_v7(), Some("build.tool"), Some("cargo"), "Cargo.")
            .await;

        let conflicted = search(
            &f.store,
            f.project,
            &MemoryQuery {
                conflicted: true,
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("conflicted");
        assert_eq!(conflicted.len(), 2);
        assert!(conflicted
            .iter()
            .all(|r| r.topic_key.as_deref() == Some("infra.db")));

        let corroborated = search(
            &f.store,
            f.project,
            &MemoryQuery {
                corroborated: true,
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("corroborated");
        assert_eq!(corroborated.len(), 2);
        assert!(corroborated
            .iter()
            .all(|r| r.topic_key.as_deref() == Some("auth.strategy")));

        // Neither filter touches the settled subject.
        let all = search(&f.store, f.project, &query(), &ctx())
            .await
            .expect("all");
        assert_eq!(all.len(), 5);
    });
}

/// A drifted memory stays returned by default retrieval (FR-373).
///
/// Hiding it would make an agent silently re-derive knowledge Cairn holds. It
/// stays lifecycle-`active`, so nothing about default retrieval changes.
#[test]
fn a_drifted_memory_is_still_returned_by_default() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let m = f
            .propose(
                Uuid::now_v7(),
                Some("service.api_port"),
                Some("8080"),
                "Port 8080.",
            )
            .await;
        sqlx::query("UPDATE memories SET verification = 'drifted' WHERE id = ?1")
            .bind(m.memory.id.to_string())
            .execute(f.store.pool())
            .await
            .expect("drift");

        let results = search(&f.store, f.project, &query(), &ctx())
            .await
            .expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].state, MemoryState::Active);
    });
}

/// A search with no Feature 003 parameter behaves exactly as it did (FR-497).
#[test]
fn omitting_every_new_parameter_gives_feature_001_behaviour() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // A free-form memory, a keyed one, and a superseded one.
        cairn_store::repo::create_memory(
            &f.store,
            cairn_store::repo::NewMemory::free_form(
                f.project,
                MemoryType::Decision,
                MemoryScope::Project,
                &f.scope_key,
                "A plain Feature 001 memory.",
                Uuid::now_v7(),
                false,
                &[],
            ),
            cairn_store::outbox::SyncPolicy {
                linked: false,
                server_project_id: None,
            },
        )
        .await
        .expect("create");
        f.propose(Uuid::now_v7(), Some("build.tool"), Some("cargo"), "Cargo.")
            .await;

        let results = search(&f.store, f.project, &query(), &ctx())
            .await
            .expect("search");
        assert_eq!(results.len(), 2);
        // The default lifecycle filter is unchanged, and the historical framing
        // is absent unless a historical question was asked.
        assert!(results.iter().all(|r| r.state == MemoryState::Active));
        assert!(results.iter().all(|r| r.temporal.is_none()));
    });
}

/// `as_of` before anything existed returns nothing, rather than everything.
#[test]
fn a_historical_query_before_the_first_proposal_is_empty() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        f.propose(Uuid::now_v7(), Some("build.tool"), Some("cargo"), "Cargo.")
            .await;
        let long_ago: DateTime<Utc> = Utc::now() - Duration::days(365);
        let results = search(
            &f.store,
            f.project,
            &MemoryQuery {
                as_of: Some(long_ago),
                ..query()
            },
            &ctx(),
        )
        .await
        .expect("search");
        assert!(results.is_empty(), "{results:?}");
    });
}

/// T037 — a merge elevates nothing on its own (FR-382), and a deleted branch
/// leaves its knowledge as history (FR-383).
#[test]
fn no_automatic_elevation() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let branch_memory = f
            .propose_scoped(
                Uuid::now_v7(),
                MemoryScope::Branch,
                Some("feature/x"),
                Some("api.style"),
                Some("graphql"),
                "The API on this branch is GraphQL.",
            )
            .await;

        // The branch is a candidate for elevation, and being a candidate is the
        // whole of what happens.
        let candidates = cairn_store::knowledge::branch_scoped_subjects(&f.store, f.project)
            .await
            .expect("candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].branch, "feature/x");
        assert_eq!(candidates[0].topic_key, "api.style");

        // Nothing moved to project scope, and no decision was recorded.
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memories WHERE scope = 'project'")
                .await,
            0,
            "a merge elevated knowledge to project scope on its own"
        );
        assert_eq!(
            f.count("SELECT COUNT(*) FROM memory_relations").await,
            0,
            "a merge recorded a decision on its own"
        );

        // The branch goes away. Its knowledge becomes history, not nothing.
        let marked =
            cairn_store::repo::mark_stale_scopes(&f.store, f.project, &["main".to_string()])
                .await
                .expect("mark stale");
        assert_eq!(marked, 1);

        let row: (String, Option<String>, String) =
            sqlx::query_as("SELECT state, stale_at, content FROM memories WHERE id = ?1")
                .bind(branch_memory.memory.id.to_string())
                .fetch_one(f.store.pool())
                .await
                .expect("row");
        assert_eq!(row.0, "stale", "branch deletion deleted the memory");
        assert!(
            row.1.is_some(),
            "the instant was not recorded going forward"
        );
        assert_eq!(
            row.2, "The API on this branch is GraphQL.",
            "the content was rewritten"
        );
        assert_eq!(f.count("SELECT COUNT(*) FROM memories").await, 1);
    });
}

/// T028 — the supersession corpus, driven through a real store.
///
/// Each case names, per supersession, which member was effective before it and
/// which after. The test rebuilds the chain, records an instant at each step,
/// and asserts the historical answer on both sides (SC-305, metric 8).
#[test]
fn the_supersession_corpus_answers_either_side_of_each_supersession() {
    let cases = cairn_core::corpus::load_group(&cairn_core::corpus::root(), "supersession")
        .expect("the supersession corpus loads");
    assert!(cases.len() >= 5, "{} cases", cases.len());

    let mut chains_exercised = 0usize;

    for case in &cases {
        let Some(chain) = case.expect.extra.get("chain").and_then(|c| c.as_array()) else {
            continue;
        };
        let expectations = case.expect.extra["as_of"]
            .as_array()
            .unwrap_or_else(|| panic!("{}", case.context("a chain case needs as_of")));
        chains_exercised += 1;

        let (rt, f) = Fixture::blocking();
        rt.block_on(async {
            // Rebuild the chain in order, recording an instant after each link.
            let mut ids = std::collections::BTreeMap::new();
            let mut instants: Vec<chrono::DateTime<Utc>> = Vec::new();
            let mut previous: Option<Uuid> = None;

            for (i, label) in chain.iter().enumerate() {
                let label = label.as_str().expect("label");
                let m = case
                    .input
                    .memories
                    .iter()
                    .find(|m| m.label == label)
                    .unwrap_or_else(|| panic!("{}", case.context(format!("no member {label}"))));

                let out = f
                    .propose(
                        Uuid::now_v7(),
                        m.topic_key.as_deref(),
                        m.value_key.as_deref(),
                        &m.content,
                    )
                    .await;
                ids.insert(label.to_string(), out.memory.id);

                if let Some(prior) = previous {
                    cairn_store::knowledge::reconcile(
                        &f.store,
                        f.project,
                        out.memory.id,
                        prior,
                        RelationKind::Supersedes,
                        RelationBasis::ExplicitUser,
                        None,
                        None,
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{}", case.context(format!("supersede: {e}"))));
                }
                previous = Some(out.memory.id);

                // A second between links, so `as_of` can land strictly between
                // two supersessions rather than on a boundary.
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                instants.push(Utc::now());
                let _ = i;
            }

            for expectation in expectations {
                let step = expectation["after_supersession"].as_u64().expect("step") as usize;
                let effective: Vec<Uuid> = expectation["effective"]
                    .as_array()
                    .expect("effective")
                    .iter()
                    .map(|l| ids[l.as_str().expect("label")])
                    .collect();

                let at = instants[step];
                let results = search(
                    &f.store,
                    f.project,
                    &MemoryQuery {
                        as_of: Some(at),
                        ..query()
                    },
                    &ctx(),
                )
                .await
                .expect("historical search");

                let mut got: Vec<Uuid> = results.iter().map(|r| r.id).collect();
                got.sort();
                let mut want = effective;
                want.sort();
                assert_eq!(
                    got,
                    want,
                    "{}",
                    case.context(format!(
                        "the historical answer after supersession {step} is wrong"
                    ))
                );
            }

            // Every predecessor is still intact and still retrievable: a chain
            // preserves history, it does not consume it.
            assert_eq!(
                f.count("SELECT COUNT(*) FROM memories").await as usize,
                chain.len()
            );
        });
    }

    assert!(
        chains_exercised >= 2,
        "only {chains_exercised} chain cases carried as_of expectations"
    );
}

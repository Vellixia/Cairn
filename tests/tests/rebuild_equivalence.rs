//! Every derived value equals its rebuild (T038, FR-517, I20, SC-324).
//!
//! Cairn has no event log, so the obligation "replay" translates to is that
//! **every derived value is rebuildable from durable records by a documented
//! deterministic procedure** (D43). Each procedure is exercised the same way:
//! discard the stored value, recompute it, and assert equality.
//!
//! `relation_order_invariance` is the companion property. `derive_subject`
//! consumes relations as a *set*, so applying them in any sequence must yield
//! the same `SubjectView` — which is what makes cross-device merge simple, because
//! there is no ordering authority to elect.

use cairn_core::knowledge::{derive_subject, Relation, SubjectView};
use cairn_core::tasks::{criteria_projection, CriterionFacts};
use cairn_core::{CriterionState, CriterionVerification, MemoryScope, RelationBasis, RelationKind};
use cairn_e2e::alpha4::Alpha4Store;
use cairn_e2e::store_fixture::Fixture;
use cairn_store::knowledge::{
    reconcile, rebuild_reinforcement, rebuild_supersession, reinforce, relations_for_project,
    subject,
};
use uuid::Uuid;

fn view_json(v: &SubjectView) -> String {
    serde_json::to_string(v).expect("serializes")
}

/// `rebuild_supersession` reproduces exactly what the relations imply.
#[test]
fn rebuild_supersession_equals_the_stored_state() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let a = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.").await;
        let b = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.").await;
        let c = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.").await;

        // A chain: c supersedes b supersedes a.
        reconcile(&f.store, f.project, b.memory.id, a.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect("b supersedes a");
        reconcile(&f.store, f.project, c.memory.id, b.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect("c supersedes b");

        let stored: Vec<String> = sqlx::query_scalar(
            "SELECT id || '=' || state || '=' || COALESCE(superseded_by_id, 'none')
               FROM memories ORDER BY id",
        )
        .fetch_all(f.store.pool())
        .await
        .expect("stored");

        // Discard the cached columns entirely, then rebuild them.
        sqlx::query("UPDATE memories SET state = 'active', superseded_by_id = NULL")
            .execute(f.store.pool())
            .await
            .expect("discard");
        let differed = rebuild_supersession(&f.store, f.project)
            .await
            .expect("rebuild");
        assert_eq!(differed, 2, "two rows had to be corrected");

        let rebuilt: Vec<String> = sqlx::query_scalar(
            "SELECT id || '=' || state || '=' || COALESCE(superseded_by_id, 'none')
               FROM memories ORDER BY id",
        )
        .fetch_all(f.store.pool())
        .await
        .expect("rebuilt");
        assert_eq!(rebuilt, stored, "the rebuild does not equal what was stored");

        // Idempotent: a second run finds nothing to correct. A difference is a
        // bug report, not a normal outcome.
        assert_eq!(
            rebuild_supersession(&f.store, f.project).await.expect("again"),
            0
        );
    });
}

/// `rebuild_reinforcement` reproduces both counts.
#[test]
fn rebuild_reinforcement_equals_the_stored_counts() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        let target = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("postgresql"), "PostgreSQL.")
            .await;
        // One duplicate from a second session, and one explicit reinforcement
        // from a third.
        f.propose(Uuid::now_v7(), Some("infra.db"), Some("postgresql"), "postgresql")
            .await;
        let confirming = f
            .propose(Uuid::now_v7(), Some("api.style"), Some("rest"), "Still true.")
            .await;
        reinforce(
            &f.store,
            f.project,
            confirming.memory.id,
            target.memory.id,
            Uuid::now_v7(),
            RelationBasis::ExplicitAgent,
        )
        .await
        .expect("reinforce");

        let stored: (i64, i64) = sqlx::query_as(
            "SELECT reinforcement_count, distinct_origin_count FROM memories WHERE id = ?1",
        )
        .bind(target.memory.id.to_string())
        .fetch_one(f.store.pool())
        .await
        .expect("stored");
        assert_eq!(stored, (2, 3), "one duplicate and one reinforcement, three origins");

        // Discard and rebuild.
        sqlx::query(
            "UPDATE memories SET reinforcement_count = 0, distinct_origin_count = 1 WHERE id = ?1",
        )
        .bind(target.memory.id.to_string())
        .execute(f.store.pool())
        .await
        .expect("discard");

        let rebuilt = rebuild_reinforcement(&f.store, target.memory.id)
            .await
            .expect("rebuild");
        assert_eq!(rebuilt, stored);
    });
}

/// Metric 33 — applying a relation set in any order yields the same derivation.
#[test]
fn relation_order_invariance() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        // A subject with every relation kind the derivation reads.
        let a = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.").await;
        let b = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.").await;
        let c = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.").await;
        let d = f.propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.").await;
        reconcile(&f.store, f.project, d.memory.id, a.memory.id,
                  RelationKind::Supersedes, RelationBasis::ExplicitUser, None, None)
            .await
            .expect("supersede");

        let read = subject(
            &f.store,
            f.project,
            MemoryScope::Project,
            &f.scope_key,
            "infra.db",
            64,
        )
        .await
        .expect("subject");
        let baseline = view_json(&read.view);

        let members = read.members.clone();
        let relations: Vec<Relation> = relations_for_project(&f.store, f.project)
            .await
            .expect("relations");
        assert!(relations.len() >= 2, "{relations:?}");

        // Every rotation of the relation set, and the reverse of each.
        for rotation in 0..relations.len() {
            let mut permuted: Vec<Relation> = relations.clone();
            permuted.rotate_left(rotation);
            assert_eq!(
                view_json(&derive_subject(&members, &permuted)),
                baseline,
                "rotation {rotation} changed the derivation"
            );

            permuted.reverse();
            assert_eq!(
                view_json(&derive_subject(&members, &permuted)),
                baseline,
                "reversing rotation {rotation} changed the derivation"
            );
        }

        // And shuffling the *members* changes nothing either: the derivation
        // sorts by identifier for stable output, never for arbitration.
        let mut shuffled = members.clone();
        shuffled.reverse();
        assert_eq!(
            view_json(&derive_subject(&shuffled, &relations)),
            baseline,
            "member order changed the derivation"
        );

        // Unused, but named so the intent of the fixture is legible.
        let _ = (b.memory.id, c.memory.id);
    });
}

/// The subject derivation stores nothing, so reading it repeatedly cannot
/// change it (D44).
#[test]
fn deriving_a_subject_is_a_pure_read() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        f.propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.").await;
        f.propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.").await;

        let mut seen: Option<String> = None;
        for _ in 0..5 {
            let read = f.subject("infra.db").await;
            let json = view_json(&read.view);
            if let Some(first) = &seen {
                assert_eq!(&json, first, "a read changed the derivation");
            }
            seen = Some(json);
        }
        // No table holds a canonical answer, so there is nothing to have
        // written: the only rows are the relation the conflict produced.
        assert_eq!(f.count("SELECT COUNT(*) FROM memory_relations").await, 1);
    });
}

/// migration.md §Proof assertions 5 and 14 — post-migration state equals
/// rebuild, for every value except the one documented approximation.
#[test]
fn rebuild_matches_migration_except_superseded_at() {
    let store = Alpha4Store::build();
    store.migrate_to_latest();

    let before: Vec<String> = store.query_column(
        "SELECT id || '=' || state || '=' || COALESCE(superseded_by_id, 'none')
           FROM memories ORDER BY id",
    );
    let superseded_at_before: Vec<String> = store.query_column(
        "SELECT id || '=' || COALESCE(superseded_at, 'none') FROM memories ORDER BY id",
    );

    let project = store.scalar("SELECT id FROM projects WHERE linked = 1");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let differed = rt.block_on(async {
        let s = cairn_store::Store::open(&store.db_path()).await.expect("open");
        let n = rebuild_supersession(&s, project.parse().expect("uuid"))
            .await
            .expect("rebuild");
        s.pool().close().await;
        n
    });

    assert_eq!(
        differed, 0,
        "the migration wrote a state the relations do not imply"
    );
    assert_eq!(
        store.query_column(
            "SELECT id || '=' || state || '=' || COALESCE(superseded_by_id, 'none')
               FROM memories ORDER BY id",
        ),
        before,
        "assertion 5: rebuild_supersession does not reproduce the migrated state"
    );

    // And `superseded_at` — the feature's single derived approximation — is
    // untouched by the rebuild, because no relation can imply it. It is named
    // explicitly rather than excluded silently (D74, R6).
    assert_eq!(
        store.query_column(
            "SELECT id || '=' || COALESCE(superseded_at, 'none') FROM memories ORDER BY id",
        ),
        superseded_at_before,
        "the rebuild rewrote the documented approximation"
    );
    assert!(
        superseded_at_before
            .iter()
            .any(|r| !r.ends_with("=none")),
        "the fixture has a superseded memory for the approximation to apply to"
    );
}

/// `rebuild_criteria_projection` equals every stored array (I11).
#[test]
fn the_criteria_projection_equals_its_rebuild() {
    let store = Alpha4Store::build();
    store.migrate_to_latest();

    for task_id in store.query_column("SELECT id FROM tasks WHERE deleted_at IS NULL ORDER BY id") {
        let stored: Vec<String> = serde_json::from_str(&store.scalar(&format!(
            "SELECT acceptance_criteria FROM tasks WHERE id = '{task_id}'"
        )))
        .expect("array");

        let rows: Vec<String> = store.query_column(&format!(
            "SELECT CAST(ordinal AS TEXT) || '\u{1f}' || text FROM task_criteria
              WHERE task_id = '{task_id}' AND deleted_at IS NULL"
        ));
        let facts: Vec<CriterionFacts> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let (ordinal, text) = row.split_once('\u{1f}').expect("row shape");
                CriterionFacts {
                    id: Uuid::from_u128(i as u128 + 1),
                    ordinal: ordinal.parse().expect("ordinal"),
                    text: text.to_string(),
                    state: CriterionState::Pending,
                    verification: CriterionVerification::Unverified,
                    deleted: false,
                }
            })
            .collect();

        assert_eq!(
            criteria_projection(&facts),
            stored,
            "task {task_id}: the rebuilt projection differs from the stored array"
        );
    }
}

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
    rebuild_reinforcement, rebuild_supersession, reconcile, reinforce, relations_for_project,
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
        let a = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.")
            .await;
        let b = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.")
            .await;
        let c = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.")
            .await;

        // A chain: c supersedes b supersedes a.
        reconcile(
            &f.store,
            f.project,
            b.memory.id,
            a.memory.id,
            RelationKind::Supersedes,
            RelationBasis::ExplicitUser,
            None,
            None,
        )
        .await
        .expect("b supersedes a");
        reconcile(
            &f.store,
            f.project,
            c.memory.id,
            b.memory.id,
            RelationKind::Supersedes,
            RelationBasis::ExplicitUser,
            None,
            None,
        )
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
        assert_eq!(
            rebuilt, stored,
            "the rebuild does not equal what was stored"
        );

        // Idempotent: a second run finds nothing to correct. A difference is a
        // bug report, not a normal outcome.
        assert_eq!(
            rebuild_supersession(&f.store, f.project)
                .await
                .expect("again"),
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
            .propose(
                Uuid::now_v7(),
                Some("infra.db"),
                Some("postgresql"),
                "PostgreSQL.",
            )
            .await;
        // One duplicate from a second session, and one explicit reinforcement
        // from a third.
        f.propose(
            Uuid::now_v7(),
            Some("infra.db"),
            Some("postgresql"),
            "postgresql",
        )
        .await;
        let confirming = f
            .propose(
                Uuid::now_v7(),
                Some("api.style"),
                Some("rest"),
                "Still true.",
            )
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
        assert_eq!(
            stored,
            (2, 3),
            "one duplicate and one reinforcement, three origins"
        );

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
        let a = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.")
            .await;
        let b = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.")
            .await;
        let c = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.")
            .await;
        let d = f
            .propose(Uuid::now_v7(), Some("infra.db"), Some("v3"), "Three.")
            .await;
        reconcile(
            &f.store,
            f.project,
            d.memory.id,
            a.memory.id,
            RelationKind::Supersedes,
            RelationBasis::ExplicitUser,
            None,
            None,
        )
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
        f.propose(Uuid::now_v7(), Some("infra.db"), Some("v1"), "One.")
            .await;
        f.propose(Uuid::now_v7(), Some("infra.db"), Some("v2"), "Two.")
            .await;

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
        let s = cairn_store::Store::open(&store.db_path())
            .await
            .expect("open");
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
        superseded_at_before.iter().any(|r| !r.ends_with("=none")),
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

/// `rebuild_criteria_projection` equals the stored array after live edits
/// (T070, I11, SC-324).
///
/// The test above proves the migration's backfill agrees with the projection.
/// This one proves the *repository* keeps agreeing after adds, state changes,
/// removals and the whole-list form — which is the case that can actually rot,
/// because every one of those writes the array in the same transaction as the
/// rows and a single missed call would leave the two silently apart.
#[test]
fn rebuild_criteria_projection_equals_the_stored_array() {
    let (rt, f) = Fixture::blocking();
    rt.block_on(async {
        const LOCAL: cairn_store::outbox::SyncPolicy = cairn_store::outbox::SyncPolicy {
            linked: false,
            server_project_id: None,
        };
        let session = Uuid::now_v7();
        let task = cairn_store::repo::create_task(
            &f.store,
            f.project,
            "Add rate limiting",
            "Requests over the limit get 429",
            &["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
            session,
            LOCAL,
        )
        .await
        .expect("a task is created");

        let assert_agrees = |label: &'static str| {
            let store = f.store.clone();
            let id = task.id;
            async move {
                let stored = cairn_store::repo::task(&store, id)
                    .await
                    .expect("task")
                    .acceptance_criteria;
                let rebuilt = cairn_store::criteria::rebuild_criteria_projection(&store, id)
                    .await
                    .expect("rebuild");
                assert_eq!(
                    stored, rebuilt,
                    "{label}: the stored projection differs from its rebuild"
                );
            }
        };

        assert_agrees("at creation").await;

        // An add.
        cairn_store::criteria::add_criterion(&f.store, task.id, "delta", session, LOCAL)
            .await
            .expect("add");
        assert_agrees("after an add").await;

        // A state change, which must move the counter and leave the text alone.
        let criteria = cairn_store::criteria::criteria(&f.store, task.id)
            .await
            .expect("criteria");
        let beta = criteria
            .iter()
            .find(|c| c.text == "beta")
            .expect("beta exists");
        cairn_store::criteria::set_criterion_state(
            &f.store,
            beta.id,
            CriterionState::Satisfied,
            Some(beta.revision),
            session,
            LOCAL,
        )
        .await
        .expect("set state");
        assert_agrees("after a state change").await;

        // A removal — the tombstone must leave the projection, not the rows.
        cairn_store::criteria::remove_criterion(&f.store, beta.id, session, LOCAL)
            .await
            .expect("remove");
        assert_agrees("after a removal").await;
        let stored = cairn_store::repo::task(&f.store, task.id)
            .await
            .expect("task")
            .acceptance_criteria;
        assert_eq!(stored, vec!["alpha", "gamma", "delta"]);

        // The Feature 001 whole-list form.
        cairn_store::repo::update_task(
            &f.store,
            task.id,
            None,
            None,
            Some(&["alpha".to_string(), "epsilon".to_string()]),
            None,
            session,
            LOCAL,
        )
        .await
        .expect("update");
        assert_agrees("after the whole-list form").await;

        // The counter advanced with every one of those, and never left.
        let revision: i64 = sqlx::query_scalar("SELECT local_revision FROM tasks WHERE id = ?1")
            .bind(task.id.to_string())
            .fetch_one(f.store.pool())
            .await
            .expect("local_revision");
        assert!(
            revision > 1,
            "the local counter must advance with every criterion change"
        );
    });
}

// ---------------------------------------------------------------------------
// T144 — `doctor --rebuild-derived` covers every derived value (FR-478,
// FR-518, SC-324)
// ---------------------------------------------------------------------------

/// The rebuild pass reports **all six** derived values.
///
/// The list is the point. A pass that silently skipped one would report "every
/// derived value equals its rebuild" over a value it never looked at, which is
/// worse than not having the command: a release would be told it was
/// consistent by a check that had not run.
#[test]
fn the_rebuild_covers_every_derived_value() {
    let s = cairn_e2e::Sandbox::new();
    s.must(&["init"]);

    let out = s.cairn(&["--json", "doctor", "--rebuild-derived"]);
    assert!(out.ok(), "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    let derived = v["data"]["derived"].as_array().cloned().unwrap_or_default();

    let mut names: Vec<&str> = derived
        .iter()
        .filter_map(|d| d["derived"].as_str())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "memory lifecycle state",
            "pattern trust",
            "reinforcement counts",
            "task criteria projection",
            "task state digest",
            "verification state and authority",
        ],
        "the rebuild does not cover every derived value"
    );
    assert_eq!(v["data"]["differed"], 0, "{}", out.stdout);
    assert_eq!(v["data"]["consistent"], true);
}

/// A store with real derived state rebuilds to the same answer, and the
/// command exits zero (SC-324).
#[test]
fn a_populated_project_rebuilds_to_the_same_answer() {
    let s = cairn_e2e::Sandbox::new();
    s.must(&["init"]);
    s.write_file("config/app.yml", "server:\n  port: 8080\n");

    // A subject with two members and a decision between them.
    let old = s.json(&[
        "memory",
        "add",
        "The API listens on port 8080",
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        "api.port",
        "--value-key",
        "8080",
    ]);
    let new = s.json(&[
        "memory",
        "add",
        "The API listens on port 9000",
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        "api.port",
        "--value-key",
        "9000",
    ]);
    let (old_id, new_id) = (
        old["memory"]["id"].as_str().expect("id").to_string(),
        new["memory"]["id"].as_str().expect("id").to_string(),
    );
    s.json(&[
        "memory",
        "reconcile",
        "--from",
        &new_id,
        "--to",
        &old_id,
        "--relation",
        "supersedes",
        "--basis",
        "explicit_user",
    ]);

    // A verified memory, and a task with criteria.
    let evidence = s.json(&[
        "evidence",
        "add",
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "9000",
        "--locator",
        "config/app.yml#server.port",
        "--collector",
        "cairn",
        "--memory",
        &new_id,
    ]);
    assert!(evidence.get("error").is_none(), "{evidence}");
    s.json(&["verify", "--memory", &new_id]);
    s.json(&[
        "task",
        "new",
        "--title",
        "Rebuildable",
        "--goal",
        "have derived state",
        "--criterion",
        "one",
        "--criterion",
        "two",
    ]);

    let out = s.cairn(&["--json", "doctor", "--rebuild-derived"]);
    assert!(
        out.ok(),
        "a populated project must rebuild to what it already held: {} {}",
        out.stdout,
        out.stderr
    );
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["data"]["differed"], 0, "{}", out.stdout);

    // And it actually looked at something, rather than finding nothing to check.
    let checked: i64 = v["data"]["derived"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|d| d["checked"].as_i64().unwrap_or(0))
        .sum();
    assert!(checked > 0, "the rebuild checked nothing: {}", out.stdout);
}

/// A derived value that disagrees with its records exits **non-zero**.
///
/// The negative that makes the command a gate. Without it, a rebuild that
/// found a difference and reported it cheerfully would let a release ship a
/// store nobody could trust.
#[test]
fn a_disagreeing_derived_value_fails_the_check() {
    let s = cairn_e2e::Sandbox::new();
    s.must(&["init"]);

    let a = s.json(&[
        "memory",
        "add",
        "A claim worth reinforcing",
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        "infra.db",
        "--value-key",
        "postgresql",
    ]);
    let id = a["memory"]["id"].as_str().expect("id").to_string();

    // Corrupt the derived column directly: nothing recorded justifies this
    // count, so the rebuild must notice.
    s.exec_sql(&format!(
        "UPDATE memories SET reinforcement_count = 7 WHERE id = '{id}'"
    ));

    let out = s.cairn(&["--json", "doctor", "--rebuild-derived"]);
    assert!(
        !out.ok(),
        "a derived value disagreeing with its records must fail the check: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("derived_inconsistent") || out.stderr.contains("derived_inconsistent"),
        "the failure must name what it is: {} {}",
        out.stdout,
        out.stderr
    );

    // And the rebuild corrected it, so a second run is clean — the command
    // repairs as well as reports.
    let again = s.cairn(&["--json", "doctor", "--rebuild-derived"]);
    assert!(
        again.ok(),
        "the rebuild did not correct what it found: {}",
        again.stdout
    );
}

/// The rebuild is a **check**, not a write that syncs (FR-478).
///
/// `rebuild_verification` re-queues a memory so a peer learns of a check that
/// happened — which is right when a check happened, and wrong here. A release
/// gate that generated sync traffic proportional to the project size would be
/// a command whose cost nobody expects, on a project where nothing changed.
#[test]
fn rebuilding_a_linked_project_queues_nothing() {
    let s = cairn_e2e::Sandbox::new();
    s.must(&["init"]);
    s.write_file("config/app.yml", "server:\n  port: 8080\n");

    for i in 0..5 {
        let m = s.json(&[
            "memory",
            "add",
            &format!("Claim number {i}"),
            "--type",
            "fact",
            "--scope",
            "project",
            "--topic-key",
            &format!("topic.number_{i}"),
            "--value-key",
            &format!("v{i}"),
        ]);
        let id = m["memory"]["id"].as_str().expect("id").to_string();
        s.json(&[
            "evidence",
            "add",
            "--type",
            "configuration",
            "--subject",
            "API port",
            "--value",
            "8080",
            "--locator",
            "config/app.yml#server.port",
            "--collector",
            "cairn",
            "--memory",
            &id,
        ]);
        s.json(&["verify", "--memory", &id]);
    }

    // Linked, so anything queueable would be queued.
    s.exec_sql("UPDATE projects SET linked = 1, server_project_id = id");

    let queued = |s: &cairn_e2e::Sandbox| -> i64 {
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM outbox")[0]
            .parse()
            .expect("a count")
    };
    let before = queued(&s);
    let out = s.cairn(&["--json", "doctor", "--rebuild-derived"]);
    assert!(out.ok(), "{} {}", out.stdout, out.stderr);
    let after = queued(&s);

    assert_eq!(
        after,
        before,
        "the rebuild queued {} outbox row(s) on a project where nothing changed",
        after - before
    );
}

// ===========================================================================
// T189 / FR-442 — the same rebuild obligation, per domain
// ===========================================================================
//
// Personal and team knowledge reuse `classify_proposal` and `derive_subject`
// unchanged, so the rebuild obligation applies to them too. It applies in a
// slightly different form, and the difference is worth stating: for project
// memory there are stored derived values to compare against a recomputation,
// while the two global domains store **no** derived value at all — their subject
// reads compute from durable rows on every call.
//
// That makes the obligation stronger rather than weaker, and it is what these
// tests assert: there is nothing cached that could drift, and the computation
// itself does not depend on the order the durable rows are read in.

fn global_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Neither global domain stores a derived value.
///
/// A `reconciliation`, `subject_state` or `answers` column would be a cached
/// derivation, and a cached derivation is a thing that can disagree with its own
/// inputs. The project domain has such columns and pays for them with the rebuild
/// checks above; these two domains avoid the cost by not having them, and this
/// test is what keeps that true.
#[test]
fn no_global_domain_stores_a_derived_value_to_drift() {
    global_runtime().block_on(async {
        let store = cairn_store::Store::open_memory().await.unwrap();
        for table in ["personal_knowledge", "team_knowledge"] {
            let columns: Vec<String> =
                sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                    .fetch_all(store.pool())
                    .await
                    .unwrap();
            assert!(
                !columns.is_empty(),
                "{table} does not exist; this test would pass vacuously"
            );
            for derived in [
                "reconciliation",
                "subject_state",
                "answers",
                "answer_count",
                "distinct_origin_count",
                "reinforcement_count",
            ] {
                assert!(
                    !columns.iter().any(|c| c == derived),
                    "{table} stores `{derived}`, a derived value that can disagree with \
                     the rows it was derived from"
                );
            }
        }
    });
}

/// A personal subject derives the same answer however its relations were
/// recorded.
///
/// `derive_subject` consumes relations as a set, and that is what makes
/// cross-device merge simple: there is no ordering authority to elect. The
/// project-domain version of this claim is `relation_order_invariance` above; this
/// is the same claim for a domain that reaches the function through a different
/// table.
#[test]
fn a_personal_subject_derives_the_same_answer_in_any_relation_order() {
    global_runtime().block_on(async {
        use cairn_store::global::{create_personal, personal_subject, NewPersonalKnowledge};

        // Two orders of the same three writes. The relations `classify_proposal`
        // records depend on which rows it has already seen, so writing them in a
        // different order is what varies the relation set's construction while
        // leaving the durable facts the same.
        let mut rendered: Vec<String> = Vec::new();
        for reversed in [false, true] {
            let store = cairn_store::Store::open_memory().await.unwrap();
            let owner = cairn_core::domain::new_id();
            let mut writes = vec![
                ("the retry budget is four attempts", "four"),
                ("retry budget: four attempts", "four"),
                ("the retry budget is two attempts", "two"),
            ];
            if reversed {
                writes.reverse();
            }
            for (content, value) in writes {
                create_personal(
                    &store,
                    NewPersonalKnowledge::direct(
                        owner,
                        cairn_core::domain::MemoryType::Fact,
                        content,
                        Some("retry.budget"),
                        Some(value),
                        Vec::new(),
                    ),
                    &[],
                )
                .await
                .expect("create");
            }

            let subject = personal_subject(&store, owner, "retry.budget")
                .await
                .expect("subject");
            rendered.push(format!(
                "{:?}|{}|{}",
                subject.view.reconciliation,
                subject.view.answers.len(),
                subject.members.len()
            ));
        }
        assert_eq!(
            rendered[0], rendered[1],
            "the order the personal records were written in changed the derived answer"
        );
    });
}

/// Deriving a global subject is a pure read: calling it twice changes nothing.
///
/// The companion to `deriving_a_subject_is_a_pure_read` above, for the two
/// domains that were added after it. A derivation that wrote — a cached answer, a
/// touched counter — would make recall a write path, which is the thing
/// `no_read_path_creates_global_content` asserts from the other end.
#[test]
fn deriving_a_global_subject_writes_nothing() {
    global_runtime().block_on(async {
        use cairn_store::global::{
            create_personal, personal_subject, propose_team, team_subject, NewPersonalKnowledge,
            NewTeamKnowledge,
        };

        let store = cairn_store::Store::open_memory().await.unwrap();
        let owner = cairn_core::domain::new_id();
        create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                cairn_core::domain::MemoryType::Fact,
                "the retry budget is four attempts",
                Some("retry.budget"),
                Some("four"),
                Vec::new(),
            ),
            &[],
        )
        .await
        .expect("create");
        propose_team(
            &store,
            NewTeamKnowledge::direct(
                owner,
                cairn_core::domain::MemoryType::Convention,
                "release tags are annotated",
                Some("release.tags"),
                Some("annotated"),
                Vec::new(),
            ),
            &[],
        )
        .await
        .expect("propose");

        let fingerprint = |store: &cairn_store::Store| {
            let pool = store.pool().clone();
            async move {
                let mut out = String::new();
                for table in [
                    "personal_knowledge",
                    "personal_knowledge_relations",
                    "team_knowledge",
                    "team_knowledge_relations",
                ] {
                    let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                    out.push_str(&format!("{table}={n};"));
                }
                out
            }
        };

        let before = fingerprint(&store).await;
        for _ in 0..3 {
            personal_subject(&store, owner, "retry.budget")
                .await
                .unwrap();
            team_subject(&store, "release.tags").await.unwrap();
        }
        assert_eq!(
            before,
            fingerprint(&store).await,
            "deriving a global subject wrote to the store"
        );
    });
}

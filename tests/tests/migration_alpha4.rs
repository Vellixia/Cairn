//! Migration from a **real** v0.1.0-alpha.4 store (migration.md §Proof).
//!
//! The fixture is built by running migrations 1–4 through `cairn_store::migrate`
//! itself, not by hand-writing the historical DDL. A hand-written approximation
//! proves the migration works against the schema someone wrote down; users have
//! the one the migration scripts produced.
//!
//! Every assertion migration.md lists is here, except the three that depend on
//! code later phases add and are named where they will land:
//!
//! - assertion 5 and 14 — `rebuild_supersession` / `rebuild_reinforcement`
//!   equality (T038)
//! - assertion 11 — the Feature 001 and 002 end-to-end suites against a
//!   migrated store (T138)
//! - the second half of 13b — a historical query reporting unknown
//!   applicability, which needs the `as_of` predicate (T036)

use cairn_core::tasks::{criteria_projection, CriterionFacts};
use cairn_core::{CriterionState, CriterionVerification};
use cairn_e2e::alpha4::{ids, Alpha4Store, PRE_EXISTING_TABLES, SCHEMA};
use cairn_e2e::Sandbox;
use serde_json::json;
use uuid::Uuid;

/// Every column the fixture's tables carried at schema 4, so the byte-identity
/// comparison names them explicitly rather than trusting `SELECT *` to keep its
/// shape across a migration that adds columns.
fn pre_existing_columns(store: &Alpha4Store, table: &str) -> Vec<String> {
    store.query_column(&format!("SELECT name FROM pragma_table_info('{table}')"))
}

#[test]
fn the_fixture_stands_at_schema_four_and_carries_a_store_in_use() {
    let store = Alpha4Store::build();

    assert_eq!(
        store.schema_version(),
        SCHEMA,
        "the fixture must stop at alpha.4's schema, not run ahead to the one under test"
    );

    let feature_003_tables = store.query_column(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN
         ('memory_relations','evidence_facts','verification_runs','continuity_checkpoints',
          'reusable_patterns','pattern_applications','task_criteria','task_blockers','task_changes')",
    );
    assert!(
        feature_003_tables.is_empty(),
        "the fixture already carries Feature 003 tables: {feature_003_tables:?}"
    );

    assert_eq!(store.row_count("projects"), 2, "linked and unlinked");
    assert_eq!(store.row_count("tasks"), 4);
    assert_eq!(store.row_count("sessions"), 4);
    assert_eq!(store.row_count("memories"), 8);
    assert_eq!(store.row_count("memory_evidence"), 2);
    assert_eq!(store.row_count("handoffs"), 3, "all three triggers");
    assert_eq!(store.row_count("outbox"), 4, "all four states");

    assert_eq!(
        store.query_column("SELECT DISTINCT state FROM memories ORDER BY state"),
        vec!["active", "stale", "superseded"]
    );
    assert_eq!(
        store.query_column("SELECT DISTINCT trigger FROM handoffs ORDER BY trigger"),
        vec!["pre_compact", "recovered", "session_end"]
    );
    assert_eq!(
        store.query_column("SELECT DISTINCT state FROM outbox ORDER BY state"),
        vec!["delivered", "failed", "in_flight", "pending"]
    );

    assert_eq!(
        store.scalar(&format!(
            "SELECT superseded_by_id FROM memories WHERE id = '{}'",
            ids::MEM_CHAIN_1
        )),
        ids::MEM_CHAIN_2
    );
    assert_eq!(
        store.scalar(&format!(
            "SELECT superseded_by_id FROM memories WHERE id = '{}'",
            ids::MEM_CHAIN_3
        )),
        ids::MEM_CHAIN_4
    );

    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memory_evidence me
              JOIN observations o ON o.id = me.observation_id
             WHERE o.deleted_at IS NOT NULL"
        ),
        "1"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM sessions WHERE handoff_pending = 1"),
        "1"
    );
    assert!(!store.scalar("SELECT pull_cursor FROM sync_meta").is_empty());
    assert_eq!(
        store.scalar(&format!(
            "SELECT acceptance_criteria FROM tasks WHERE id = '{}'",
            ids::TASK_EMPTY_CRITERIA
        )),
        "[]"
    );
}

#[test]
fn the_fixture_is_reproducible() {
    // Two builds are byte-identical, so a later diff attributes every
    // difference to the migration rather than to the fixture.
    let a = Alpha4Store::build();
    let b = Alpha4Store::build();
    for table in PRE_EXISTING_TABLES {
        let columns = pre_existing_columns(&a, table);
        let refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            a.snapshot(table, &refs),
            b.snapshot(table, &refs),
            "{table} differs between two builds of the fixture"
        );
    }
}

/// Assertions 1 and 2 — zero rows lost, zero pre-existing values rewritten.
#[test]
fn no_row_is_lost_and_no_pre_existing_value_is_rewritten() {
    let store = Alpha4Store::build();

    let before_counts = store.row_counts();
    let mut before_snapshots = Vec::new();
    for table in PRE_EXISTING_TABLES {
        let columns = pre_existing_columns(&store, table);
        let refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
        before_snapshots.push((*table, columns.clone(), store.snapshot(table, &refs)));
    }

    assert_eq!(store.migrate_to_latest(), 5);

    assert_eq!(
        store.row_counts(),
        before_counts,
        "a row was lost or created by the migration"
    );

    for (table, columns, before) in before_snapshots {
        let refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
        let after = store.snapshot(table, &refs);
        assert_eq!(
            after, before,
            "{table}: a pre-existing column value changed. \
             The migration is additive; only the two documented backfills may write, \
             and they write columns that did not exist before"
        );
    }
}

/// Assertion 3 — every new column at its documented default, and the two
/// backfills exactly as specified.
#[test]
fn new_columns_carry_their_documented_defaults() {
    let store = Alpha4Store::build();
    store.migrate_to_latest();

    // Nothing is fabricated to satisfy a new column (FR-515).
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE topic_key IS NOT NULL"),
        "0",
        "inferring a subject from prose is what FR-317 forbids"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE value_key IS NOT NULL"),
        "0"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE content_norm_digest IS NOT NULL"
        ),
        "0"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE verification <> 'unverified'"
        ),
        "0",
        "no evidence exists, so nothing is verified"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE verification_authority IS NOT NULL"
        ),
        "0",
        "authority is meaningless unless verified"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE importance <> 'normal'"),
        "0"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE pinned <> 0"),
        "0"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE reinforcement_count <> 0"),
        "0"
    );
    assert_eq!(
        store
            .scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE distinct_origin_count <> 1"),
        "0",
        "exactly one origin session, which is true"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM tasks WHERE local_revision <> 1"),
        "0"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM sessions WHERE task_snapshot_at_bind IS NOT NULL"
        ),
        "0",
        "a session that bound before this feature genuinely does not know"
    );

    // Backfill (a): exact.
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE effective_from IS NULL"),
        "0"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE effective_from <> created_at"
        ),
        "0",
        "a memory was effective from when it was created"
    );

    // Backfill (b): the feature's single documented approximation, scoped to
    // rows that were already superseded.
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories
              WHERE state = 'superseded' AND superseded_at IS NULL"
        ),
        "0"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories
              WHERE state <> 'superseded' AND superseded_at IS NOT NULL"
        ),
        "0",
        "the approximation is scoped; it never touches a row that was not superseded"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories
              WHERE state = 'superseded' AND superseded_at <> updated_at"
        ),
        "0"
    );
}

/// Assertion 13b — `stale_at` is deliberately not inferred.
#[test]
fn a_memory_that_went_stale_before_this_feature_has_an_unknown_instant() {
    let store = Alpha4Store::build();
    store.migrate_to_latest();

    assert!(
        store.row_count("memories") > 0,
        "the fixture carries memories"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE stale_at IS NOT NULL"),
        "0",
        "NULL means UNKNOWN, and inferring one would be a second approximation"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE state = 'stale' AND stale_at IS NULL"
        ),
        "1",
        "the fixture's stale memory keeps an unknown instant"
    );
}

/// Assertion 4 — one `supersedes` relation per pre-existing link, and no
/// others.
#[test]
fn every_pre_existing_supersession_becomes_exactly_one_relation() {
    let store = Alpha4Store::build();

    let links: Vec<String> = store.query_column(
        "SELECT superseded_by_id || '>' || id FROM memories
          WHERE superseded_by_id IS NOT NULL ORDER BY id",
    );
    assert_eq!(links.len(), 3, "the fixture's chain is three links deep");

    store.migrate_to_latest();

    let relations: Vec<String> = store.query_column(
        "SELECT from_memory_id || '>' || to_memory_id FROM memory_relations
          WHERE kind = 'supersedes' ORDER BY to_memory_id",
    );
    assert_eq!(relations, links);
    assert_eq!(
        store.row_count("memory_relations"),
        3,
        "and no relation of any other kind was invented"
    );

    // `basis` is `explicit_user` because a Feature 001 supersession was always
    // an explicit act, and the rationale is honest about where it came from.
    assert_eq!(
        store.query_column("SELECT DISTINCT basis FROM memory_relations"),
        vec!["explicit_user"]
    );
    assert!(store
        .scalar("SELECT rationale FROM memory_relations LIMIT 1")
        .contains("migrated from Feature 001"));
}

/// Assertions 6 and 7 — criteria rows, and the retained projection.
#[test]
fn criteria_become_rows_and_the_projection_is_unchanged() {
    let store = Alpha4Store::build();

    let arrays: Vec<String> = store
        .query_column("SELECT acceptance_criteria FROM tasks WHERE deleted_at IS NULL ORDER BY id");
    let expected_total: usize = arrays
        .iter()
        .map(|a| {
            serde_json::from_str::<Vec<String>>(a)
                .unwrap_or_default()
                .len()
        })
        .sum();

    store.migrate_to_latest();

    assert_eq!(
        store.row_count("task_criteria") as usize,
        expected_total,
        "one row per element of every non-deleted task's array"
    );
    assert!(expected_total > 0, "the fixture has criteria to convert");

    // A deleted task contributes none.
    assert_eq!(
        store.scalar(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM task_criteria WHERE task_id = '{}'",
            ids::TASK_DELETED
        )),
        "0"
    );
    // An empty array contributes none.
    assert_eq!(
        store.scalar(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM task_criteria WHERE task_id = '{}'",
            ids::TASK_EMPTY_CRITERIA
        )),
        "0"
    );

    // Duplicate strings produce distinct rows: they were distinct entries, and
    // merging them would lose one.
    let duplicates: Vec<String> = store.query_column(&format!(
        "SELECT text FROM task_criteria WHERE task_id = '{}' ORDER BY ordinal",
        ids::TASK_DUPLICATE_CRITERIA
    ));
    assert_eq!(
        duplicates,
        vec!["Do the thing", "Do the thing", "Then stop"],
        "position order preserved, duplicates kept"
    );
    let ids_for_duplicates: Vec<String> = store.query_column(&format!(
        "SELECT id FROM task_criteria WHERE task_id = '{}'",
        ids::TASK_DUPLICATE_CRITERIA
    ));
    assert_eq!(
        ids_for_duplicates
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3,
        "distinct identifiers"
    );

    // Labels and ordinals in position order.
    let labelled: Vec<String> = store.query_column(&format!(
        "SELECT label || '=' || CAST(ordinal AS TEXT) FROM task_criteria
          WHERE task_id = '{}' ORDER BY ordinal",
        ids::TASK_DUPLICATE_CRITERIA
    ));
    assert_eq!(labelled, vec!["AC-1=1", "AC-2=2", "AC-3=3"]);

    // Every identifier is a UUIDv7, matching the convention every other
    // identifier in this schema follows.
    for id in store.query_column("SELECT id FROM task_criteria") {
        let parsed = Uuid::parse_str(&id).unwrap_or_else(|e| panic!("{id} is not a UUID: {e}"));
        assert_eq!(parsed.get_version_num(), 7, "{id} is not a UUIDv7");
    }

    // States and timestamps.
    assert_eq!(
        store.query_column("SELECT DISTINCT state FROM task_criteria"),
        vec!["pending"]
    );
    assert_eq!(
        store.query_column("SELECT DISTINCT verification FROM task_criteria"),
        vec!["unverified"]
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM task_criteria c
               JOIN tasks t ON t.id = c.task_id
              WHERE c.created_at <> t.created_at"
        ),
        "0",
        "the criterion is as old as the task, not as old as the migration"
    );

    // Assertion 7 — `rebuild_criteria_projection` equals every array, byte for
    // byte. The projection is the one denormalization the feature keeps, and
    // this is what stops it drifting.
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

/// Assertions 8, 9 and 13a — the outbox is untouched and still deliverable.
#[test]
fn the_outbox_is_untouched_and_nothing_becomes_blocked() {
    let store = Alpha4Store::build();

    let before: Vec<String> = store.query_column(
        "SELECT id || '=' || state || '=' || idempotency_key FROM outbox ORDER BY id",
    );
    let cursor_before = store.scalar("SELECT pull_cursor FROM sync_meta");

    store.migrate_to_latest();

    let after: Vec<String> = store.query_column(
        "SELECT id || '=' || state || '=' || idempotency_key FROM outbox ORDER BY id",
    );
    assert_eq!(after, before, "an outbox row changed state or identity");

    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE state = 'blocked'"),
        "0",
        "`blocked` is only ever reached by an actual capability refusal (D81)"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE blocked_reason IS NOT NULL"),
        "0"
    );
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE state = 'pending'"),
        "1",
        "the pending row is still claimable"
    );
    assert_eq!(
        store.scalar("SELECT pull_cursor FROM sync_meta"),
        cursor_before,
        "a cursor part-way through a pull is not disturbed"
    );
    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM sync_meta WHERE server_capability IS NOT NULL"
        ),
        "0",
        "no capability has been observed yet"
    );

    // Assertion 9 — a `local_only` memory still produces no outbox row.
    assert_eq!(
        store.scalar(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM outbox WHERE entity_id = '{}'",
            ids::MEM_LOCAL_ONLY
        )),
        "0"
    );
}

/// Assertion 10 — FTS is not disturbed.
///
/// Worth its place: `memory_fts` is an external-content table with three
/// triggers, and the most likely way a careless additive migration breaks a
/// user's store is by rebuilding the table those triggers hang off.
#[test]
fn full_text_search_returns_the_same_rows_in_the_same_order() {
    let store = Alpha4Store::build();

    let query = "SELECT m.id FROM memory_fts f
                   JOIN memories m ON m.rowid = f.rowid
                  WHERE memory_fts MATCH 'database'
                  ORDER BY rank, m.id";
    let before = store.query_column(query);
    assert!(
        !before.is_empty(),
        "the fixture is searchable to begin with"
    );

    let triggers_before =
        store.query_column("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name");

    store.migrate_to_latest();

    assert_eq!(store.query_column(query), before, "FTS ranking changed");
    assert_eq!(
        store.query_column("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name"),
        triggers_before,
        "an FTS trigger was recreated or dropped"
    );
}

/// Assertion 13 — running the migration twice is a no-op.
#[test]
fn running_the_migration_twice_changes_nothing() {
    let store = Alpha4Store::build();
    assert_eq!(store.migrate_to_latest(), 5);

    let snapshot: Vec<String> =
        store.query_column("SELECT id || '=' || text FROM task_criteria ORDER BY task_id, ordinal");
    let relations = store.row_count("memory_relations");
    let criteria = store.row_count("task_criteria");

    assert_eq!(store.migrate_to_latest(), 5);

    assert_eq!(store.row_count("memory_relations"), relations);
    assert_eq!(store.row_count("task_criteria"), criteria);
    assert_eq!(
        store.query_column("SELECT id || '=' || text FROM task_criteria ORDER BY task_id, ordinal"),
        snapshot,
        "a second run duplicated or reassigned a criterion"
    );
}

/// Assertion 12 — an interrupted migration leaves a working alpha.4 store.
///
/// A user who loses power mid-upgrade has the store they started with, not a
/// half-migrated one. The failure is injected by pre-creating a column the
/// migration adds, so a statement part-way through the script fails.
#[test]
fn an_interrupted_migration_rolls_back_entirely() {
    let store = Alpha4Store::build();
    let counts_before = store.row_counts();

    // `pinned` is added late in step 1, so several statements succeed first.
    store.execute("ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0");

    let result = store.migrate_to(5);
    assert!(
        result.is_err(),
        "the injected failure did not stop the migration: {result:?}"
    );

    assert_eq!(
        store.schema_version(),
        SCHEMA,
        "schema_migrations advanced despite the failure"
    );
    assert_eq!(
        store.row_counts(),
        counts_before,
        "the store lost or gained rows while rolling back"
    );
    // The tables the script had not reached do not exist, and neither do the
    // ones it had: DDL is transactional in SQLite, so the whole script rolled
    // back together.
    assert!(
        store
            .query_column(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'memory_relations'"
            )
            .is_empty(),
        "a table survived the rollback"
    );
    // And the store is still usable by the build that wrote it.
    assert_eq!(store.row_count("memories"), 8);
    assert!(!store
        .query_column("SELECT id FROM memories WHERE state = 'active'")
        .is_empty());
}

/// T023 — the schema-version guard (FR-516).
#[test]
fn an_older_build_refuses_a_newer_store_and_a_newer_build_migrates_an_older_one() {
    let store = Alpha4Store::build();

    // A schema-5 build opens a schema-4 store by migrating it.
    assert_eq!(store.migrate_to_latest(), 5);

    // A build that supports only schema 4 refuses it rather than writing
    // against a schema it does not understand.
    let refused = store
        .migrate_to(4)
        .expect_err("a schema-4 build must refuse");
    assert!(
        refused.contains("newer than this build supports"),
        "the refusal does not name the version guard: {refused}"
    );
    assert!(refused.contains('5') && refused.contains('4'), "{refused}");

    // And the refusal changed nothing.
    assert_eq!(store.schema_version(), 5);
}

/// The DDL constraints on the new tables are live, not decorative.
///
/// A `CHECK` calling a JSON1 function is accepted at `CREATE TABLE` whether or
/// not it does anything at insert time, so "the migration applied" says nothing
/// about whether the constraint constrains. Both halves are exercised here
/// rather than discovered in Phase 12.
#[test]
fn the_new_tables_enforce_their_check_constraints() {
    let store = Alpha4Store::build();
    store.migrate_to_latest();

    let pattern = |signals: &str| {
        format!(
            "INSERT INTO reusable_patterns
                (id, title, problem, signals, signal_digest, root_cause, root_cause_digest,
                 approach, trust, origin_ref, created_at, updated_at)
             VALUES (lower(hex(randomblob(16))), 't', 'p', '{signals}', lower(hex(randomblob(16))),
                     'rc', lower(hex(randomblob(16))), 'a', 'candidate',
                     lower(hex(randomblob(16))), '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')"
        )
    };

    // A pattern that matches indiscriminately is worse than none, so the
    // minimum signal count is structural as well as a gate check.
    assert!(
        store.try_execute(&pattern(r#"["only one"]"#)).is_err(),
        "a one-signal pattern was accepted; the json_array_length CHECK is inert"
    );
    let sixteen: Vec<String> = (0..17).map(|i| format!("\"s{i}\"")).collect();
    assert!(
        store
            .try_execute(&pattern(&format!("[{}]", sixteen.join(","))))
            .is_err(),
        "a seventeen-signal pattern was accepted"
    );
    store
        .try_execute(&pattern(r#"["first signal","second signal"]"#))
        .expect("two signals is the documented minimum");

    // The duplicate-refusal index is structural too, so a race cannot create
    // one (gate check 10).
    let duplicate = "INSERT INTO reusable_patterns
            (id, title, problem, signals, signal_digest, root_cause, root_cause_digest,
             approach, trust, origin_ref, created_at, updated_at)
         VALUES (lower(hex(randomblob(16))), 't', 'p', '[\"a signal\",\"b signal\"]', 'sig-1',
                 'rc', 'rc-1', 'a', 'candidate', 'origin', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')";
    store.try_execute(duplicate).expect("the first lands");
    assert!(
        store.try_execute(duplicate).is_err(),
        "a duplicate (signal_digest, root_cause_digest) was accepted"
    );

    // And the enum columns on the new tables really are constrained.
    assert!(store
        .try_execute(
            "INSERT INTO memory_relations
                (from_memory_id, to_memory_id, kind, project_id, decided_by_session, decided_at, basis)
             VALUES ('a', 'b', 'invented_kind', 'p', 's', '2026-01-01T00:00:00Z', 'explicit_user')"
        )
        .is_err());
}

/// A relation may name a memory that has not arrived yet.
///
/// `contracts/records-and-rebuild.md` §Fail-closed requires a dangling endpoint
/// to be **ignored by the derivation and reported by `doctor`** — never
/// deleted, because it may be a memory that has not synced yet. A foreign key
/// on those columns would refuse the insert instead, so a decision that arrived
/// before its proposal would be dropped silently on import.
#[test]
fn a_relation_may_reference_a_memory_that_has_not_arrived() {
    let store = Alpha4Store::build();
    store.migrate_to_latest();

    let project = store.scalar("SELECT id FROM projects LIMIT 1");
    let present = ids::MEM_ACTIVE_A;

    store
        .try_execute(&format!(
            "INSERT INTO memory_relations
                (from_memory_id, to_memory_id, kind, project_id, decided_by_session, decided_at, basis)
             VALUES ('{present}', 'not-yet-synced', 'conflicts_with', '{project}',
                     '{}', '2026-01-01T00:00:00Z', 'deterministic_rule')",
            ids::SESSION_ACTIVE
        ))
        .expect("a relation whose endpoint has not synced yet must be storable");

    assert_eq!(
        store.scalar(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memory_relations WHERE to_memory_id = 'not-yet-synced'"
        ),
        "1"
    );
}

// ---------------------------------------------------------------------------
// T138 — the Feature 001 and 002 behaviours, on a migrated alpha.4 store
// (FR-519, SC-323)
// ---------------------------------------------------------------------------

/// A sandbox whose database **is** a migrated alpha.4 store.
///
/// The daemon is stopped, the fixture is put in place of the empty store the
/// sandbox created, and every later command starts the daemon again against
/// it. What runs afterwards is the real CLI, the real daemon and the real MCP
/// server — the point is not that migration succeeds in isolation but that a
/// developer who upgrades keeps working.
fn sandbox_on_migrated_alpha4() -> (Sandbox, Alpha4Store) {
    let s = Sandbox::new();
    let fixture = Alpha4Store::build();
    let version = fixture.migrate_to_latest();
    assert_eq!(
        version,
        cairn_store::migrate::latest_version(),
        "the fixture must be migrated all the way before anything reads it"
    );

    let stopped = s.cairn(&["daemon", "stop"]);
    assert!(
        stopped.ok() || stopped.stderr.contains("not running"),
        "could not stop the daemon before replacing its store: {}",
        stopped.stderr
    );

    // The sidecars go too: a `-wal` from the empty store would contradict the
    // database replacing it.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(s.sidecar(suffix));
    }
    std::fs::copy(fixture.db_path(), s.db_path()).expect("install the migrated store");

    // The fixture's project belongs to the fixture's repository, and Cairn
    // resolves a project by its Git common directory. Without this the daemon
    // would come up, find no project for *this* worktree, register a second
    // empty one, and every assertion below would be about a store that was
    // migrated and then ignored — which is exactly the shape of an upgrade
    // that looks successful and loses everything.
    //
    // The fixture holds more than one project and `git_common_dir` is unique,
    // so the one that gets this worktree is the one that owns the memories —
    // the project a developer upgrading would actually be standing in.
    // Canonicalized: macOS reports the sandbox at `/var/folders/...` and
    // resolves it to `/private/var/folders/...`, which is what the daemon
    // stores. The two are the same directory and not the same string.
    let git_common_dir = std::fs::canonicalize(s.repo_path())
        .expect("the sandbox repository")
        .join(".git")
        .display()
        .to_string();
    s.exec_sql(&format!(
        "UPDATE projects SET git_common_dir = '{}'
          WHERE id = (SELECT project_id FROM memories
                       GROUP BY project_id ORDER BY COUNT(*) DESC LIMIT 1)",
        git_common_dir.replace('\'', "''")
    ));

    (s, fixture)
}

/// Every Feature 001 and 002 behaviour a developer depends on still holds
/// (FR-519, SC-323).
///
/// Asserted against the surfaces rather than the schema. A migration that
/// preserved every row and broke `cairn context` would pass every test in this
/// file above this one.
#[test]
fn the_feature_001_and_002_surfaces_work_on_a_migrated_store() {
    let (s, _fixture) = sandbox_on_migrated_alpha4();

    // --- The daemon comes up against the migrated store and knows the schema.
    let status = s.json(&["status"]);
    assert_eq!(
        status["local_schema_version"].as_i64(),
        Some(cairn_store::migrate::latest_version()),
        "{status}"
    );
    assert_eq!(status["daemon"], "running", "{status}");
    assert!(
        status["memory_count"].as_i64().unwrap_or(0) > 0,
        "the alpha.4 memories did not survive: {status}"
    );

    // --- Feature 001: a session, a memory, a search, a briefing, a handoff.
    s.hook(
        "SessionStart",
        json!({ "session_id": "migrated-1", "source": "startup" }),
    );
    s.settle_session_count(1);

    let added = s.json(&[
        "memory",
        "add",
        "A decision recorded after the upgrade",
        "--type",
        "decision",
        "--scope",
        "project",
    ]);
    assert!(added.get("error").is_none(), "{added}");

    let found = s.json(&["memory", "search", "upgrade"]);
    assert!(
        found["total"].as_i64().unwrap_or(0) > 0,
        "search stopped working on a migrated store: {found}"
    );

    let context = s.cairn(&["context", "--json"]);
    assert!(context.ok(), "{}", context.stderr);
    let briefing: serde_json::Value = serde_json::from_str(&context.stdout).expect("json");
    let briefing = &briefing["data"]["briefing"];
    for required in ["project", "repository", "memory"] {
        assert!(
            briefing.get(required).is_some(),
            "the briefing lost `{required}`: {briefing}"
        );
    }

    let task = s.json(&[
        "task",
        "new",
        "--title",
        "Post-upgrade work",
        "--goal",
        "prove the surfaces",
        "--criterion",
        "the suite runs",
    ]);
    let task_id = task["task"]["id"].as_str().expect("a task id").to_string();
    let got = s.json(&["task", "show", &task_id]);
    assert_eq!(
        got["task"]["acceptance_criteria"][0], "the suite runs",
        "the criteria projection is wrong after migration: {got}"
    );

    s.hook(
        "SessionEnd",
        json!({ "session_id": "migrated-1", "reason": "clear" }),
    );
    s.settle("the closed session's handoff", |s| {
        s.json(&["status"])["sessions_awaiting_handoff"].as_i64() == Some(0)
    });
    let handoff = s.json(&["handoff", "show"]);
    assert!(
        handoff.get("error").is_none(),
        "a handoff could not be read after migration: {handoff}"
    );

    // --- Feature 002: the integration surfaces answer.
    let agents = s.cairn(&["--json", "agents"]);
    assert!(agents.ok(), "{}", agents.stderr);
    let doctor = s.cairn(&["--json", "doctor"]);
    assert!(doctor.ok(), "{}", doctor.stderr);
    assert!(
        doctor.stdout.contains("local_schema_version"),
        "doctor lost its core block: {}",
        doctor.stdout
    );

    // --- And the pre-existing rows are still what they were, reachable
    //     through the surface rather than only through SQL.
    let historical = s.json(&["memory", "search", "--state", "superseded"]);
    assert!(
        historical["total"].as_i64().unwrap_or(0) > 0,
        "the alpha.4 supersession chain is no longer retrievable: {historical}"
    );
}

/// The Feature 003 surfaces work on the migrated store too, without any of the
/// backfilled values being invented.
///
/// The migration deliberately leaves `topic_key`, `value_key` and
/// `content_norm_digest` NULL, because inferring them is what FR-315 and FR-317
/// forbid. A subject read must therefore find nothing rather than find
/// something made up — and the memory must still be perfectly usable.
#[test]
fn a_migrated_memory_has_no_invented_subject_and_still_works() {
    let (s, _fixture) = sandbox_on_migrated_alpha4();

    let with_subject =
        s.query_column("SELECT CAST(COUNT(*) AS TEXT) FROM memories WHERE topic_key IS NOT NULL");
    assert_eq!(
        with_subject,
        vec!["0".to_string()],
        "the migration invented a subject identity for a pre-existing memory"
    );

    // Free-form and fully usable: searchable, briefable, and syncable exactly
    // as it was before the upgrade.
    let found = s.json(&["memory", "search"]);
    assert!(
        found["total"].as_i64().unwrap_or(0) > 0,
        "migrated memories are not retrievable: {found}"
    );
    let adoption = s.json(&["status"])["knowledge"].clone();
    assert_eq!(
        adoption["with_subject"].as_i64(),
        Some(0),
        "the adoption metric disagrees with the store: {adoption}"
    );
}

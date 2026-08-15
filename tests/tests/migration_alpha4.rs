//! Migration from a **real** v0.1.0-alpha.4 store (migration.md §Proof).
//!
//! The fixture is built by running migrations 1–4 through `cairn_store::migrate`
//! itself, not by hand-writing the historical DDL. A hand-written approximation
//! proves the migration works against the schema someone wrote down; users have
//! the one the migration scripts produced.
//!
//! Phase 1 lands the fixture and its sanity check. The sixteen migration
//! assertions arrive with T022, once migration 0005 exists.

use cairn_e2e::alpha4::{ids, Alpha4Store, PRE_EXISTING_TABLES, SCHEMA};

#[test]
fn the_fixture_stands_at_schema_four_and_carries_a_store_in_use() {
    let store = Alpha4Store::build();

    assert_eq!(
        store.schema_version(),
        SCHEMA,
        "the fixture must stop at alpha.4's schema, not run ahead to the one under test"
    );

    // Every table an alpha.4 store has, and none of Feature 003's.
    for table in PRE_EXISTING_TABLES {
        store.row_count(table);
    }
    let feature_003_tables = store.query_column(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN
         ('memory_relations','evidence_facts','verification_runs','continuity_checkpoints',
          'reusable_patterns','pattern_applications','task_criteria','task_blockers','task_changes')",
    );
    assert!(
        feature_003_tables.is_empty(),
        "the fixture already carries Feature 003 tables: {feature_003_tables:?}"
    );

    // The state migration.md says a store in use actually contains.
    assert_eq!(store.row_count("projects"), 2, "linked and unlinked");
    assert_eq!(store.row_count("tasks"), 4);
    assert_eq!(store.row_count("sessions"), 4);
    assert_eq!(store.row_count("memories"), 8);
    assert_eq!(store.row_count("memory_evidence"), 2);
    assert_eq!(store.row_count("handoffs"), 3, "all three triggers");
    assert_eq!(store.row_count("outbox"), 4, "all four states");

    assert_eq!(
        store.query_column("SELECT DISTINCT state FROM memories ORDER BY state"),
        vec!["active", "stale", "superseded"],
        "all three lifecycle states are present"
    );
    assert_eq!(
        store.query_column("SELECT DISTINCT trigger FROM handoffs ORDER BY trigger"),
        vec!["pre_compact", "recovered", "session_end"]
    );
    assert_eq!(
        store.query_column("SELECT DISTINCT state FROM outbox ORDER BY state"),
        vec!["delivered", "failed", "in_flight", "pending"]
    );

    // A supersession chain three links deep. Two links hide ordering defects.
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

    // A `memory_evidence` row whose observation is tombstoned — the case
    // FR-505 says must keep resolving to "evidence deleted".
    assert_eq!(
        store.scalar(&format!(
            "SELECT CAST(COUNT(*) AS TEXT) FROM memory_evidence me
              JOIN observations o ON o.id = me.observation_id
             WHERE o.deleted_at IS NOT NULL"
        )),
        "1"
    );

    // A session owed a handoff, and a cursor part-way through a pull.
    assert_eq!(
        store.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM sessions WHERE handoff_pending = 1"),
        "1"
    );
    assert!(!store
        .scalar("SELECT pull_cursor FROM sync_meta")
        .is_empty());

    // Criteria arrays as they really occur.
    assert_eq!(
        store.scalar(&format!(
            "SELECT acceptance_criteria FROM tasks WHERE id = '{}'",
            ids::TASK_EMPTY_CRITERIA
        )),
        "[]"
    );
    assert!(store
        .scalar(&format!(
            "SELECT acceptance_criteria FROM tasks WHERE id = '{}'",
            ids::TASK_DUPLICATE_CRITERIA
        ))
        .contains("Do the thing\",\"Do the thing"));
}

#[test]
fn the_fixture_is_reproducible() {
    // Two builds are byte-identical, so a later diff attributes every
    // difference to the migration rather than to the fixture.
    let a = Alpha4Store::build();
    let b = Alpha4Store::build();
    for table in PRE_EXISTING_TABLES {
        let columns = column_names(&a, table);
        let refs: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            a.snapshot(table, &refs),
            b.snapshot(table, &refs),
            "{table} differs between two builds of the fixture"
        );
    }
}

fn column_names(store: &Alpha4Store, table: &str) -> Vec<String> {
    store.query_column(&format!("SELECT name FROM pragma_table_info('{table}')"))
}

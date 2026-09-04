//! Local schema v8 — the migration, and the constraints that carry meaning
//! (T006, `data-model.md` §5).
//!
//! Three kinds of assertion live here, and they are not interchangeable:
//!
//! - **The migration happens.** v7 → v8 adds the tables Feature 005 needs.
//! - **The migration is atomic.** A v8 script that fails part way leaves a v7
//!   database, not a database half way between two schemas. This is asserted by
//!   actually making the real script fail, not by reasoning about the
//!   transaction it runs in.
//! - **The constraints refuse things.** `retained_local`'s discriminator and
//!   `legacy_pattern_claims`' two uniqueness rules are load-bearing: the first
//!   is what lets one table name three record shapes without a column that
//!   means different things in different rows, and the second is what makes
//!   promoting a pattern twice yield one record while two people's identical
//!   patterns stay two.

use cairn_e2e::feature005::{
    Local, LocalAt, LOCAL_SCHEMA_V10, LOCAL_SCHEMA_V7, LOCAL_SCHEMA_V8, LOCAL_SCHEMA_V9,
};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Every table v8 introduces, and nothing else.
const V8_TABLES: &[&str] = &[
    "session_event_seq",
    "command_seq",
    "event_spool",
    "command_spool",
    "capture_disposition_counts",
    "authority_mode",
    "migration_state",
    "retained_local",
    "legacy_pattern_claims",
];

#[test]
fn v7_does_not_have_the_v8_tables() {
    rt().block_on(async {
        let db = LocalAt::new(LOCAL_SCHEMA_V7).await;
        assert_eq!(db.schema_version().await, LOCAL_SCHEMA_V7);
        for table in V8_TABLES {
            assert_eq!(
                db.count(&format!(
                    "SELECT count(*) FROM sqlite_master
                      WHERE type = 'table' AND name = '{table}'"
                ))
                .await,
                0,
                "v7 already has {table}, so v8 is not what introduces it"
            );
        }
    });
}

#[test]
fn v7_migrates_to_v8_and_gains_every_table() {
    rt().block_on(async {
        let db = LocalAt::new(LOCAL_SCHEMA_V7).await;
        let db = db.migrate_to_latest().await;
        // Migrating to *latest* lands past v8, so the assertion is that v8's
        // tables all arrived and not that the store stopped there. Pinning this
        // to a particular later version is what made it fail twice already —
        // once at v9 and once at v10 — so it now asserts only that latest is at
        // least v8, and the table check below is what carries the rule.
        assert!(db.schema_version().await >= LOCAL_SCHEMA_V8);
        for table in V8_TABLES {
            assert!(db.table_exists(table).await, "v8 did not create {table}");
        }
    });
}

/// The spool's server-instance binding arrives in v10.
///
/// Both columns and both re-created claim indexes, because the binding is only
/// enforceable if the claim predicate can use it: a column nothing indexes
/// means a claim that scans another deployment's backlog to discard it.
#[test]
fn v9_migrates_to_v10_and_binds_the_spools_to_a_server_instance() {
    rt().block_on(async {
        let db = LocalAt::new(LOCAL_SCHEMA_V9).await;
        for table in ["event_spool", "command_spool"] {
            assert_eq!(
                db.count(&format!(
                    "SELECT count(*) FROM pragma_table_info('{table}')
                      WHERE name = 'server_instance_id'"
                ))
                .await,
                0,
                "v9 already binds {table}, so v10 is not what introduces it"
            );
        }
        let db = db.migrate_to_latest().await;
        assert!(db.schema_version().await >= LOCAL_SCHEMA_V10);
        for table in ["event_spool", "command_spool"] {
            assert!(
                db.column_exists(table, "server_instance_id").await,
                "v10 did not bind {table}"
            );
        }
    });
}

/// The pattern cache arrives in v9, and it is a table of its own.
///
/// Asserted alongside the absence of the fields that forced it: a
/// `cached_patterns` that grew a `signals` column would be `reusable_patterns`
/// again, and the six refused names would be back on a table a pull writes.
#[test]
fn v8_migrates_to_v9_and_gains_the_pattern_cache() {
    rt().block_on(async {
        let db = LocalAt::new(LOCAL_SCHEMA_V8).await;
        assert_eq!(
            db.count(
                "SELECT count(*) FROM sqlite_master
                  WHERE type = 'table' AND name = 'cached_patterns'"
            )
            .await,
            0,
            "v8 already has cached_patterns, so v9 is not what introduces it"
        );
        let db = db.migrate_to_latest().await;
        assert!(db.schema_version().await >= LOCAL_SCHEMA_V9);
        assert!(db.table_exists("cached_patterns").await);
        for refused in [
            "signals",
            "signal_digest",
            "origin_ref",
            "sanitization_report",
            "source_memory_id",
            "origin_deleted",
        ] {
            assert!(
                !db.column_exists("cached_patterns", refused).await,
                "the pattern cache has a `{refused}` column; that is a name the \
                 privacy boundary refuses and a server row could never fill it"
            );
        }
    });
}

#[test]
fn the_migration_seeds_exactly_one_authority_mode_row_at_feature_004() {
    rt().block_on(async {
        let db = Local::new().await;
        assert_eq!(
            db.count("SELECT count(*) FROM authority_mode").await,
            1,
            "authority_mode is a single-row table"
        );
        let mode: String = sqlx::query_scalar("SELECT mode FROM authority_mode WHERE id = 1")
            .fetch_one(db.store.pool())
            .await
            .expect("the seeded row");
        assert_eq!(
            mode, "feature_004",
            "a store starts where Feature 004 left it and migrates forward; \
             seeding it already cut over would be claiming a migration ran"
        );
        // The timestamp is the same RFC 3339 shape every other timestamp in
        // this schema uses. A `datetime('now')` value would parse as neither.
        let changed_at: String =
            sqlx::query_scalar("SELECT changed_at FROM authority_mode WHERE id = 1")
                .fetch_one(db.store.pool())
                .await
                .expect("the seeded row");
        assert!(
            chrono::DateTime::parse_from_rfc3339(&changed_at).is_ok(),
            "changed_at {changed_at:?} is not RFC 3339"
        );
    });
}

#[test]
fn authority_mode_refuses_a_second_row_and_an_unknown_mode() {
    rt().block_on(async {
        let db = Local::new().await;
        let second = sqlx::query(
            "INSERT INTO authority_mode (id, mode, changed_at)
             VALUES (2, 'feature_004', '2026-09-02T00:00:00Z')",
        )
        .execute(db.store.pool())
        .await;
        assert!(second.is_err(), "authority_mode accepted a second row");

        let bogus = sqlx::query("UPDATE authority_mode SET mode = 'whatever' WHERE id = 1")
            .execute(db.store.pool())
            .await;
        assert!(bogus.is_err(), "authority_mode accepted an unknown mode");
    });
}

#[test]
fn an_interrupted_v8_migration_leaves_a_v7_database() {
    rt().block_on(async {
        let db = LocalAt::new(LOCAL_SCHEMA_V7).await;

        // Make the real v8 script fail part way through, by putting a table in
        // its path that it will try to create. `authority_mode` comes after
        // `event_spool` in the script, so at the moment of failure the
        // transaction is holding several already-created tables — which is
        // exactly the state that must not survive.
        db.execute("CREATE TABLE authority_mode (decoy INTEGER)")
            .await;

        let outcome = db.try_migrate_to(LOCAL_SCHEMA_V8).await;
        assert!(
            outcome.is_err(),
            "the migration was supposed to collide with the decoy table"
        );

        assert_eq!(
            db.schema_version().await,
            LOCAL_SCHEMA_V7,
            "a failed migration recorded itself as applied"
        );
        for table in V8_TABLES {
            if *table == "authority_mode" {
                continue; // the decoy, which was there before the migration ran
            }
            assert_eq!(
                db.count(&format!(
                    "SELECT count(*) FROM sqlite_master
                      WHERE type = 'table' AND name = '{table}'"
                ))
                .await,
                0,
                "{table} survived a rolled-back migration"
            );
        }
    });
}

#[test]
fn v7_rows_survive_the_migration_unchanged() {
    rt().block_on(async {
        let db = LocalAt::new(LOCAL_SCHEMA_V7).await;
        let project = db.project;
        let session = seed_session(&db, project).await;
        let pattern = seed_pattern(&db).await;

        db.execute(&format!(
            "INSERT INTO memories
                 (id, project_id, type, scope, scope_key, content, state,
                  origin_session_id, created_at, updated_at)
             VALUES ('{}', '{project}', 'fact', 'project', '{project}',
                     'a v7 memory', 'active', '{session}',
                     '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')",
            uuid::Uuid::now_v7()
        ))
        .await;

        let before = (
            db.count("SELECT count(*) FROM memories").await,
            db.count("SELECT count(*) FROM sessions").await,
            db.count("SELECT count(*) FROM reusable_patterns").await,
        );

        let db = db.migrate_to_latest().await;

        let after = (
            db.count("SELECT count(*) FROM memories").await,
            db.count("SELECT count(*) FROM sessions").await,
            db.count("SELECT count(*) FROM reusable_patterns").await,
        );
        assert_eq!(before, after, "v8 lost or added pre-existing rows");

        let content: String = sqlx::query_scalar("SELECT content FROM memories LIMIT 1")
            .fetch_one(db.store.pool())
            .await
            .expect("the v7 memory");
        assert_eq!(content, "a v7 memory", "v8 rewrote an existing row");

        // The pattern is still there and still un-owned: v8 adds a place to
        // record a claim, it does not make one.
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM reusable_patterns WHERE id = '{pattern}'"
            ))
            .await,
            1
        );
        assert_eq!(
            db.count("SELECT count(*) FROM legacy_pattern_claims").await,
            0,
            "the migration attributed an owner-less pattern to somebody"
        );
    });
}

// ---------------------------------------------------------------------------
// retained_local — the three-shape discriminator
// ---------------------------------------------------------------------------

#[test]
fn retained_local_accepts_each_of_its_three_shapes() {
    rt().block_on(async {
        let db = Local::new().await;
        for (sql, what) in [
            (
                "INSERT INTO retained_local
                     (ref_kind, domain, knowledge_id, relation_key, reason, detected_at,
                      dedupe_key)
                 VALUES ('knowledge', 'personal', 'k-1', NULL, 'server_refused',
                         '2026-09-02T00:00:00Z', 'knowledge:personal:k-1')",
                "a knowledge record",
            ),
            (
                "INSERT INTO retained_local
                     (ref_kind, domain, knowledge_id, relation_key, reason, detected_at,
                      dedupe_key)
                 VALUES ('pattern', NULL, 'p-1', NULL, 'owner_unclaimed',
                         '2026-09-02T00:00:00Z', 'pattern:p-1')",
                "a pattern",
            ),
            (
                "INSERT INTO retained_local
                     (ref_kind, domain, knowledge_id, relation_key, reason, detected_at,
                      dedupe_key)
                 VALUES ('relation', NULL, NULL, 'a|b|supersedes', 'local_only',
                         '2026-09-02T00:00:00Z', 'relation:a|b|supersedes')",
                "a relation, which has no id of its own",
            ),
        ] {
            sqlx::query(sql)
                .execute(db.store.pool())
                .await
                .unwrap_or_else(|e| panic!("retained_local refused {what}: {e}"));
        }
        assert_eq!(db.count("SELECT count(*) FROM retained_local").await, 3);
    });
}

#[test]
fn retained_local_refuses_every_mixed_shape() {
    rt().block_on(async {
        let db = Local::new().await;
        let refused = [
            (
                "knowledge with no domain",
                "('knowledge', NULL, 'k', NULL, 'local_only', 't', 'd1')",
            ),
            (
                "knowledge with no id",
                "('knowledge', 'project', NULL, NULL, 'local_only', 't', 'd2')",
            ),
            (
                "knowledge carrying a relation key as well",
                "('knowledge', 'project', 'k', 'a|b|c', 'local_only', 't', 'd3')",
            ),
            (
                "a pattern claiming a domain slot",
                "('pattern', 'personal', 'p', NULL, 'local_only', 't', 'd4')",
            ),
            (
                "a pattern with no id",
                "('pattern', NULL, NULL, NULL, 'local_only', 't', 'd5')",
            ),
            (
                "a relation with an id",
                "('relation', NULL, 'k', 'a|b|c', 'local_only', 't', 'd6')",
            ),
            (
                "a relation with no relation key",
                "('relation', NULL, NULL, NULL, 'local_only', 't', 'd7')",
            ),
            (
                "an unknown ref_kind",
                "('memory', NULL, 'k', NULL, 'local_only', 't', 'd8')",
            ),
            (
                "an unknown reason",
                "('pattern', NULL, 'p', NULL, 'felt_like_it', 't', 'd9')",
            ),
        ];
        for (what, values) in refused {
            let outcome = sqlx::query(&format!(
                "INSERT INTO retained_local
                     (ref_kind, domain, knowledge_id, relation_key, reason, detected_at,
                      dedupe_key)
                 VALUES {values}"
            ))
            .execute(db.store.pool())
            .await;
            assert!(outcome.is_err(), "retained_local accepted {what}");
        }
        assert_eq!(db.count("SELECT count(*) FROM retained_local").await, 0);
    });
}

#[test]
fn retained_local_deduplicates_on_the_discriminator_key_not_the_nullable_columns() {
    rt().block_on(async {
        let db = Local::new().await;
        async fn insert(db: &Local, key: &str) -> Result<(), sqlx::Error> {
            let sql = format!(
                "INSERT INTO retained_local
                     (ref_kind, domain, knowledge_id, relation_key, reason, detected_at,
                      dedupe_key)
                 VALUES ('relation', NULL, NULL, 'a|b|supersedes', 'local_only',
                         '2026-09-02T00:00:00Z', '{key}')"
            );
            sqlx::query(&sql).execute(db.store.pool()).await.map(|_| ())
        }
        insert(&db, "relation:a|b|supersedes")
            .await
            .expect("the first retention");

        // The whole reason `dedupe_key` exists. Every relation row has three
        // NULLs, and SQLite treats NULLs as distinct in a UNIQUE index, so a
        // UNIQUE over the natural columns would let this second insert through
        // and `--retry-retained` would double every record it re-tried.
        let again = insert(&db, "relation:a|b|supersedes").await;
        assert!(
            again.is_err(),
            "retained_local recorded the same relation twice"
        );
        assert_eq!(db.count("SELECT count(*) FROM retained_local").await, 1);
    });
}

// ---------------------------------------------------------------------------
// legacy_pattern_claims — the two uniqueness rules
// ---------------------------------------------------------------------------

#[test]
fn one_owner_claiming_the_same_content_twice_is_one_claim() {
    rt().block_on(async {
        let db = Local::new().await;
        let a = seed_pattern_in(&db, "a").await;
        let b = seed_pattern_in(&db, "b").await;

        claim(&db, &a, "owner-1", "content-key-1", "pattern-id-1")
            .await
            .expect("the first claim");

        // A different local pattern, same owner, same normalized content: the
        // derived `pattern_id` is the same value, so this is the same record
        // being claimed again rather than a second one (SC-760).
        let again = claim(&db, &b, "owner-1", "content-key-1", "pattern-id-1").await;
        assert!(
            again.is_err(),
            "the same owner claiming the same content produced two records"
        );
        assert_eq!(
            db.count("SELECT count(*) FROM legacy_pattern_claims").await,
            1
        );
    });
}

#[test]
fn two_owners_claiming_identical_content_are_two_claims() {
    rt().block_on(async {
        let db = Local::new().await;
        let a = seed_pattern_in(&db, "a").await;
        let b = seed_pattern_in(&db, "b").await;

        claim(&db, &a, "owner-1", "content-key-1", "pattern-id-1")
            .await
            .expect("the first owner's claim");
        // Identity is UUIDv5(owner ‖ content), so a different owner derives a
        // different id. Two people whose patterns read alike own two patterns,
        // and collapsing them would hand one person's record to the other.
        claim(&db, &b, "owner-2", "content-key-1", "pattern-id-2")
            .await
            .expect("the second owner's claim");

        assert_eq!(
            db.count("SELECT count(*) FROM legacy_pattern_claims").await,
            2
        );
    });
}

#[test]
fn a_claim_must_name_a_pattern_that_exists() {
    rt().block_on(async {
        let db = Local::new().await;
        let orphan = claim(
            &db,
            "no-such-pattern",
            "owner-1",
            "content-key-1",
            "pattern-id-1",
        )
        .await;
        assert!(
            orphan.is_err(),
            "a claim was accepted for a pattern this store does not have"
        );
    });
}

#[test]
fn one_local_pattern_cannot_be_claimed_by_two_owners() {
    rt().block_on(async {
        let db = Local::new().await;
        let a = seed_pattern_in(&db, "a").await;
        claim(&db, &a, "owner-1", "content-key-1", "pattern-id-1")
            .await
            .expect("the first claim");
        // `local_pattern_id` is the primary key: ownership of a local pattern
        // is established once, and a second establishment is a contradiction
        // rather than an update.
        let contested = claim(&db, &a, "owner-2", "content-key-2", "pattern-id-2").await;
        assert!(
            contested.is_err(),
            "one local pattern was claimed by two owners"
        );
    });
}

// ---------------------------------------------------------------------------
// Seeding
// ---------------------------------------------------------------------------

async fn claim(
    db: &Local,
    local_pattern_id: &str,
    owner: &str,
    content_key: &str,
    pattern_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO legacy_pattern_claims
             (local_pattern_id, owner_user_id, content_key, pattern_id, claimed_at)
         VALUES (?1, ?2, ?3, ?4, '2026-09-02T00:00:00Z')",
    )
    .bind(local_pattern_id)
    .bind(owner)
    .bind(content_key)
    .bind(pattern_id)
    .execute(db.store.pool())
    .await
    .map(|_| ())
}

async fn seed_session(db: &LocalAt, project: uuid::Uuid) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    db.execute(&format!(
        "INSERT INTO sessions
             (id, project_id, user_id, agent, branch, worktree_path, agent_session_key,
              status, started_at, last_event_at, daemon_run_id)
         VALUES ('{id}', '{project}', 'tester', 'claude-code', 'main', '/fixture',
                 'key-{id}', 'active', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z',
                 'run-1')"
    ))
    .await;
    id
}

async fn seed_pattern(db: &LocalAt) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    db.execute(&pattern_sql(id, "a")).await;
    id
}

async fn seed_pattern_in(db: &Local, salt: &str) -> String {
    let id = uuid::Uuid::now_v7();
    db.execute(&pattern_sql(id, salt)).await;
    id.to_string()
}

fn pattern_sql(id: uuid::Uuid, salt: &str) -> String {
    format!(
        "INSERT INTO reusable_patterns
             (id, title, problem, signals, signal_digest, root_cause, root_cause_digest,
              approach, trust, origin_ref, created_at, updated_at)
         VALUES ('{id}', 'a pattern', 'a problem',
                 '[\"signal one\",\"signal two\"]', 'sig-{salt}-{id}',
                 'a root cause', 'root-{salt}-{id}', 'an approach',
                 'candidate', 'origin-{id}',
                 '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z')"
    )
}

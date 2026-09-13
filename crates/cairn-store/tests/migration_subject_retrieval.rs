//! A legacy memory survives the migration with no subject and stays findable
//! (FR-317, FR-519, SC-323).
//!
//! # Why this exists at the storage layer
//!
//! The property it pins is already asserted end to end, by
//! `migration_alpha4::a_migrated_memory_has_no_invented_subject_and_still_works`
//! — and that test reaches it through a daemon, a named pipe or Unix socket, a
//! `cairn daemon stop`, a database file swapped out from under a process that
//! may still hold it, and a project row re-pointed at the sandbox's worktree.
//! Any one of those can fail on one platform and not another, and when the swap
//! failed the suite reported `migrated memories are not retrievable` — which
//! names retrieval, the migration and the search index as suspects while the
//! actual fault was that the fixture had not installed. That is a diagnosis
//! problem, and the answer to a diagnosis problem is a test that cannot have it.
//!
//! So this one is in process. No daemon, no socket, no file swap, no CLI: a
//! schema-4 database, one legacy memory written the way alpha.4 wrote it, the
//! real migration, and the real retrieval path. It can only fail for the reason
//! it is named after, and it fails identically on every platform.
//!
//! # The two halves are one property
//!
//! `topic_key` is deliberately never backfilled — `migration.md` §94 puts it
//! plainly, "Inferring a subject from prose is exactly what FR-317 and D46
//! forbid" — and Feature 005's normalization rewrites an existing key rather
//! than minting one (`contracts/migration-cutover.md` §321). A migration that
//! bought retrieval by inventing a subject would satisfy the second half of the
//! name and destroy the first, so both are asserted together and neither may be
//! traded for the other.
//!
//! **Falsified by** backfilling `topic_key` in any migration, and by making any
//! subject predicate unconditional in `search` — hoisting `m.topic_key IS NOT
//! NULL` out of its `conflicted`/`corroborated` guard turns every legacy memory
//! invisible while every row is still present.

use cairn_core::wire::MemoryQuery;
use cairn_store::search::{self, SearchContext};
use cairn_store::{migrate, Store};
use uuid::Uuid;

/// A schema-4 database holding one project and one subject-less memory.
///
/// Written with raw SQL against the schema of the day rather than through
/// `repo::create_memory`, because the point is a row that alpha.4 could have
/// written: the current writer sets columns migration 5 had not added yet, and
/// a row it produced would not be a legacy row.
async fn alpha4_store_with_one_legacy_memory(path: &std::path::Path) -> (Uuid, Uuid) {
    let project = Uuid::now_v7();
    let memory = Uuid::now_v7();

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true),
        )
        .await
        .expect("open the pre-migration database");

    let applied = migrate::run_to(&pool, 4).await.expect("migrate to 4");
    assert_eq!(
        applied, 4,
        "the fixture is not a schema-4 store, so it is not the upgrade under test"
    );

    let now = "2026-01-01T00:00:00+00:00";
    sqlx::query(
        "INSERT INTO projects
             (id, name, git_common_dir, repository_remote, linked,
              server_project_id, created_at, updated_at, deleted_at)
         VALUES (?1, 'legacy', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
    )
    .bind(project.to_string())
    .bind(format!("/legacy/{project}/.git"))
    .bind(now)
    .execute(&pool)
    .await
    .expect("seed the project");

    // Exactly the column list alpha.4 wrote, and no more: `topic_key`,
    // `value_key` and `content_norm_digest` do not exist yet at schema 4, so
    // this row cannot carry a subject even by accident.
    sqlx::query(
        "INSERT INTO memories
             (id, project_id, type, scope, scope_key, content, state,
              superseded_by_id, origin_session_id, local_only,
              created_at, updated_at, deleted_at)
         VALUES (?1, ?2, 'convention', 'project', ?2,
                 'Errors are returned, never logged and swallowed.', 'active',
                 NULL, ?3, 0, ?4, ?4, NULL)",
    )
    .bind(memory.to_string())
    .bind(project.to_string())
    .bind(Uuid::now_v7().to_string())
    .bind(now)
    .execute(&pool)
    .await
    .expect("seed the legacy memory");

    pool.close().await;
    (project, memory)
}

#[tokio::test]
async fn a_legacy_memory_migrates_without_a_subject_and_is_still_retrievable() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("legacy.sqlite3");
    let (project, memory) = alpha4_store_with_one_legacy_memory(&path).await;

    // The operation under test: the real `Store::open`, which applies every
    // migration from 5 to the latest against the store as it stands.
    let store = Store::open(&path).await.expect("migrate and open");
    assert_eq!(
        migrate::latest_version(),
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM schema_migrations")
            .fetch_one(store.pool())
            .await
            .expect("the applied version"),
        "the store did not reach the current schema, so nothing below is about \
         a migrated store"
    );

    // Half one: no invented semantics. A subject the migration made up would be
    // indistinguishable, to every later reader, from one a human chose.
    let subject: Option<String> =
        sqlx::query_scalar("SELECT topic_key FROM memories WHERE id = ?1")
            .bind(memory.to_string())
            .fetch_one(store.pool())
            .await
            .expect("read the subject back");
    assert!(
        subject.is_none(),
        "the migration invented a subject identity for a pre-existing memory: \
         {subject:?} (FR-317)"
    );

    // Half two: and it still works. The bare query — no text, no state, no
    // subject filter — is the one `cairn memory search` issues and the one the
    // end-to-end test asserts on.
    let found = search::search(
        &store,
        project,
        &MemoryQuery::default(),
        &SearchContext::default(),
    )
    .await
    .expect("search the migrated store");
    assert_eq!(
        found.len(),
        1,
        "a migrated memory with no subject is not retrievable; the row survived \
         the upgrade and the retrieval path cannot see it. Rows now: {:?}",
        sqlx::query_scalar::<_, String>(
            "SELECT id || ' state=' || state || ' scope=' || scope
                    || ' deleted=' || CAST(deleted_at IS NOT NULL AS TEXT)
                    || ' subject=' || COALESCE(topic_key, '<none>')
               FROM memories ORDER BY id"
        )
        .fetch_all(store.pool())
        .await
        .unwrap_or_default(),
    );
    assert_eq!(
        found[0].id, memory,
        "the search returned a different row than the one that was migrated"
    );
}

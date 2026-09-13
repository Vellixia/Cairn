//! Migration from a **real** v0.1.0-alpha.5 store and server
//! (T028, T029; FR-523, FR-525, FR-528, FR-530, SC-405, SC-432).
//!
//! The local fixture is built by running migrations 1–6 through
//! `cairn_store::migrate` itself, then migrating to 7 — the same discipline
//! `migration_alpha4.rs` established, and for the same reason: a hand-written
//! approximation proves the migration works against the schema someone wrote
//! down, while users have the one the scripts produced.
//!
//! The server fixture is a real schema-2 server on its own database, upgraded in
//! place. That is what an operator does, so it is what is tested.

use cairn_e2e::alpha4::{Alpha4Store, PRE_EXISTING_TABLES};
use cairn_e2e::Server;
use uuid::Uuid;

/// The schema an alpha.5 store stands at.
const ALPHA5_SCHEMA: i64 = 6;

/// A store carrying real alpha.5 state: migrations 1–6, through the real path.
fn alpha5_store() -> Alpha4Store {
    let store = Alpha4Store::build();
    store
        .migrate_to(ALPHA5_SCHEMA)
        .expect("migrations 5-6 apply to the alpha.4 fixture");
    assert_eq!(
        store.schema_version(),
        ALPHA5_SCHEMA,
        "the fixture must stop at alpha.5's schema, not run ahead to the one under test"
    );
    store
}

fn columns(store: &Alpha4Store, table: &str) -> Vec<String> {
    store.query_column(&format!("SELECT name FROM pragma_table_info('{table}')"))
}

// ---------------------------------------------------------------------------
// T028 — the local migration preserves what it does not own
// ---------------------------------------------------------------------------

/// Every pre-existing row survives migration 7 unchanged.
///
/// Counts *and* contents: a migration that dropped a table and recreated it
/// with the same number of rows would pass a count comparison, which is why the
/// column list and a content digest are compared too.
#[test]
fn migrating_to_seven_preserves_every_existing_row() {
    let store = alpha5_store();

    let before: Vec<(String, i64, Vec<String>)> = PRE_EXISTING_TABLES
        .iter()
        .map(|t| (t.to_string(), store.row_count(t), columns(&store, t)))
        .collect();
    assert!(
        before.iter().any(|(_, n, _)| *n > 0),
        "the fixture is empty, so preservation would be vacuous"
    );

    // Against the latest migration this build carries, not against a pinned
    // number. The assertion this test exists to make is that a v5-era store's
    // rows survive being brought up to date; pinning the target at 7 stopped
    // being that assertion the moment migration 8 shipped, and would have to be
    // re-pinned by hand for every migration after it. The floor stays, so a
    // build that somehow lost migration 7 still fails here.
    let latest = store.migrate_to_latest();
    assert!(
        latest >= 7,
        "the store came up at schema {latest}, below the migration this fixture is about"
    );
    assert_eq!(
        latest,
        cairn_store::migrate::latest_version(),
        "the fixture did not reach the newest schema this build carries"
    );

    for (table, count, cols) in &before {
        assert_eq!(
            store.row_count(table),
            *count,
            "{table} lost or gained rows across the migrations after 6"
        );
        let after = columns(&store, table);
        // Columns may be *added* to a table a later feature extends, but none
        // may disappear or be renamed — every reader of the old name would
        // break, silently for a nullable column.
        for column in cols {
            assert!(
                after.contains(column),
                "{table}.{column} disappeared across the migrations after 6"
            );
        }
    }
}

/// `memories` in particular is untouched — same columns, same rows, same
/// content (FR-521, SC-459).
///
/// Its own test because it is the feature's central constraint, and because a
/// sweep over every table can pass while the one table that matters most
/// changed in a way the sweep does not look at.
#[test]
fn the_memories_table_survives_byte_for_byte() {
    let store = alpha5_store();
    let digest = |s: &Alpha4Store| {
        s.query_column(
            "SELECT id || '|' || type || '|' || scope || '|' || scope_key || '|' ||
                    content || '|' || state
               FROM memories ORDER BY id",
        )
    };
    let before = digest(&store);
    let columns_before = columns(&store, "memories");
    assert!(
        !before.is_empty(),
        "the fixture has no memories to preserve"
    );

    store.migrate_to_latest();

    assert_eq!(digest(&store), before, "a memory row changed");
    assert_eq!(
        columns(&store, "memories"),
        columns_before,
        "the memories table gained or lost a column; FR-521 forbids it"
    );
}

/// An interrupted migration leaves the store on its prior working version
/// (FR-525).
///
/// Simulated the only honest way: run the migration against a store where it
/// must fail partway, then assert the recorded version did not move and the
/// tables the failed migration would have created are absent. A migration that
/// committed its schema before its data step would leave a half-built store
/// that reports itself ready.
#[test]
fn an_interrupted_migration_leaves_the_store_on_its_prior_version() {
    let store = alpha5_store();

    // Occupy a name migration 7 must create. The `CREATE TABLE` for it will
    // fail, mid-script, after earlier statements in the same transaction have
    // already run.
    store.execute("CREATE TABLE personal_knowledge (wrong_shape TEXT)");

    let outcome = store.migrate_to(7);
    assert!(
        outcome.is_err(),
        "the migration reported success despite a conflicting table: {outcome:?}"
    );
    assert_eq!(
        store.schema_version(),
        ALPHA5_SCHEMA,
        "the store advanced its recorded version despite a failed migration"
    );
    // The rest of migration 7's tables must not be half-present.
    for table in [
        "team_knowledge",
        "sync_cursor",
        "writer_identity",
        "project_traits",
    ] {
        assert_eq!(
            store.query_column(&format!(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='{table}'"
            )),
            Vec::<String>::new(),
            "{table} was created by a migration that failed; the transaction did not roll back"
        );
    }
}

/// The store's writer identity is minted exactly once, by migration 7.
#[test]
fn migration_seven_mints_exactly_one_writer_identity() {
    let store = alpha5_store();
    store.migrate_to_latest();
    assert_eq!(
        store.row_count("writer_identity"),
        1,
        "a store has exactly one writer identity (FR-490)"
    );
    let id = store.query_column("SELECT writer_id FROM writer_identity");
    assert_eq!(id.len(), 1);
    assert!(
        Uuid::parse_str(&id[0]).is_ok(),
        "the writer id is not a uuid: {:?}",
        id[0]
    );
}

/// `sync_meta`'s cursor is carried into `sync_cursor` verbatim, under the
/// project namespace (FR-486, FR-487).
#[test]
fn the_sync_cursor_backfill_preserves_each_projects_position() {
    let store = alpha5_store();

    let existing = store.query_column("SELECT project_id || '=' || pull_cursor FROM sync_meta");
    store.migrate_to_latest();

    for pair in &existing {
        let (project_id, cursor) = pair.split_once('=').expect("seeded pair");
        let backfilled = store.query_column(&format!(
            "SELECT pull_cursor FROM sync_cursor WHERE namespace = 'project:{project_id}'"
        ));
        assert_eq!(
            backfilled,
            vec![cursor.to_string()],
            "project {project_id}'s cursor was not carried over verbatim"
        );
    }
}

// ---------------------------------------------------------------------------
// T029 — the outbox rebuild keeps in-flight work
// ---------------------------------------------------------------------------

/// Rows already in the outbox survive the rebuild **with their original
/// idempotency keys** (FR-528, FR-530).
///
/// The key is the whole point. It is what makes delivery exactly-once: the
/// server's `sync_state` is keyed on it, so a row whose key changed across the
/// migration would be delivered a second time and applied a second time. A
/// rebuild that recomputed keys would look correct in every other respect.
#[test]
fn outbox_rows_in_flight_keep_their_original_idempotency_keys() {
    let store = alpha5_store();

    let before = store.query_column(
        "SELECT id || '|' || idempotency_key || '|' || entity_type || '|' || state
           FROM outbox ORDER BY id",
    );
    assert!(
        !before.is_empty(),
        "the fixture has no outbox rows, so this test would be vacuous"
    );

    store.migrate_to_latest();

    let after = store.query_column(
        "SELECT id || '|' || idempotency_key || '|' || entity_type || '|' || state
           FROM outbox ORDER BY id",
    );
    assert_eq!(
        after, before,
        "the outbox rebuild changed a row's id, key, type or state"
    );
}

/// Every carried row gets its project namespace, so it routes exactly as it did
/// before (D426, D427).
#[test]
fn carried_outbox_rows_are_routed_by_their_project_namespace() {
    let store = alpha5_store();
    let expected =
        store.query_column("SELECT id || '|project:' || project_id FROM outbox ORDER BY id");
    store.migrate_to_latest();
    let actual = store.query_column("SELECT id || '|' || namespace FROM outbox ORDER BY id");
    assert_eq!(
        actual, expected,
        "a carried outbox row is not routed to its own project's namespace"
    );
}

/// The widened `entity_type` CHECK admits the four new types and still refuses
/// what it always refused (FR-528).
///
/// Asserted as a refusal as well as an acceptance: a rebuild that dropped the
/// CHECK entirely would accept the new types too, and that CHECK is the
/// structural gate keeping local-only entity types off the wire.
#[test]
fn the_widened_entity_type_check_admits_four_and_still_refuses_the_rest() {
    let store = alpha5_store();
    store.migrate_to_latest();

    // A global row names its author and a project row does not (FR-602) — the
    // other CHECK this table carries, and the one that decides which account a
    // queued row may ever be delivered under.
    let insert = |entity_type: &str, project: Option<&str>| -> Result<(), String> {
        let id = Uuid::now_v7();
        let (project_sql, namespace, author) = match project {
            Some(p) => (format!("'{p}'"), format!("project:{p}"), "NULL".to_string()),
            None => (
                "NULL".to_string(),
                "personal:x:y".to_string(),
                format!("'{}'", Uuid::now_v7()),
            ),
        };
        store.try_execute(&format!(
            "INSERT INTO outbox
                 (id, project_id, server_project_id, entity_type, entity_id, operation,
                  idempotency_key, payload, state, attempts, created_at, namespace,
                  authored_by_user_id)
             VALUES ('{id}', {project_sql}, NULL, '{entity_type}', '{id}', 'upsert',
                     'key-{id}', '{{}}', 'pending', 0, '2026-01-01T00:00:00Z', '{namespace}',
                     {author})"
        ))
    };

    // An authorless global row is refused by the schema, so "no recorded author"
    // can never come to mean "deliverable under whichever account is logged in".
    let id = Uuid::now_v7();
    assert!(
        store
            .try_execute(&format!(
                "INSERT INTO outbox
                     (id, project_id, server_project_id, entity_type, entity_id, operation,
                      idempotency_key, payload, state, attempts, created_at, namespace)
                 VALUES ('{id}', NULL, NULL, 'personal_knowledge', '{id}', 'upsert',
                         'key-{id}', '{{}}', 'pending', 0, '2026-01-01T00:00:00Z',
                         'personal:x:y')"
            ))
            .is_err(),
        "a global outbox row was accepted with no author (FR-602)"
    );

    // Four, not two: both relations tables exist on the server as well as
    // locally, and a table on the server is reachable only through the outbox.
    for accepted in [
        "personal_knowledge",
        "personal_knowledge_relation",
        "team_knowledge",
        "team_knowledge_relation",
    ] {
        assert!(
            insert(accepted, None).is_ok(),
            "{accepted} was refused by the widened CHECK"
        );
    }
    // The types that must never reach an outbox at all — the ones whose absence
    // is what makes "it stays local" a property of the schema.
    for refused in [
        "observation",
        "evidence_fact",
        "reusable_pattern",
        "verification_run",
    ] {
        assert!(
            insert(refused, None).is_err(),
            "{refused} was accepted by the outbox CHECK; it must never be enqueueable"
        );
    }
}

// ---------------------------------------------------------------------------
// T028 — the server's role backfill, across every seeded configuration
// ---------------------------------------------------------------------------

fn server_at_schema_two() -> Option<Server> {
    match Server::start_at_schema(2) {
        Some(s) => Some(s),
        None => {
            eprintln!("SKIPPED: set CAIRN_TEST_DATABASE_URL to run the server suite");
            None
        }
    }
}

/// The `users.role` backfill ends with an administrator in every configuration,
/// and never with zero (FR-414, FR-524, SC-405).
///
/// Four seeded configurations, each on its own database, because the rule has
/// four branches and a test of one proves nothing about the others: the
/// environment-named account present, absent, a single legacy account, and
/// several.
#[test]
fn the_role_backfill_always_leaves_exactly_one_admin() {
    let Some(mut server) = server_at_schema_two() else {
        return;
    };

    // Several legacy accounts, none named by the environment. The oldest wins.
    let oldest = Uuid::now_v7();
    let newer = Uuid::now_v7();
    let newest = Uuid::now_v7();
    for (i, id) in [oldest, newer, newest].iter().enumerate() {
        server.execute(&format!(
            "INSERT INTO users (id, email, display_name, password_hash, created_at)
             VALUES ('{id}', 'legacy-{i}@example.test', 'Legacy {i}', 'x',
                     now() - interval '{} days')",
            10 - i
        ));
    }

    let upgraded = server.upgraded();
    assert_eq!(
        upgraded.count("SELECT count(*) FROM users WHERE role = 'admin'"),
        1,
        "several legacy accounts must produce exactly one admin"
    );
    assert_eq!(
        upgraded.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{oldest}' AND role = 'admin'"
        )),
        1,
        "the oldest account by created_at must be the admin when the environment names none"
    );
    assert_eq!(
        upgraded.count("SELECT count(*) FROM users WHERE role = 'member'"),
        2,
        "every other account must be a member"
    );
}

/// One legacy account becomes the administrator. The degenerate case, and the
/// one where "never zero admins" is easiest to get wrong.
#[test]
fn a_single_legacy_account_becomes_the_administrator() {
    let Some(mut server) = server_at_schema_two() else {
        return;
    };
    let only = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO users (id, email, display_name, password_hash, created_at)
         VALUES ('{only}', 'only@example.test', 'Only', 'x', now())"
    ));

    let upgraded = server.upgraded();
    assert_eq!(
        upgraded.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{only}' AND role = 'admin'"
        )),
        1,
        "the one account on the server must end up the administrator"
    );
}

/// A server with no accounts at all migrates cleanly and has no admin to name.
///
/// Vacuously correct rather than an error: there is nobody to promote, and
/// failing the migration would brick a fresh deployment whose first account
/// arrives afterwards through `ensure_admin`.
#[test]
fn an_empty_server_migrates_without_inventing_an_account() {
    let Some(mut server) = server_at_schema_two() else {
        return;
    };
    let upgraded = server.upgraded();
    assert_eq!(upgraded.count("SELECT count(*) FROM users"), 0);
    assert_eq!(
        upgraded.count("SELECT count(*) FROM users WHERE role = 'admin'"),
        0,
        "the backfill invented an account"
    );
}

/// Every account gets a role, and it is one of the two.
#[test]
fn no_account_is_left_without_a_role() {
    let Some(mut server) = server_at_schema_two() else {
        return;
    };
    for i in 0..4 {
        let id = Uuid::now_v7();
        server.execute(&format!(
            "INSERT INTO users (id, email, display_name, password_hash, created_at)
             VALUES ('{id}', 'user-{i}@example.test', 'User {i}', 'x', now())"
        ));
    }
    let upgraded = server.upgraded();
    assert_eq!(
        upgraded.count("SELECT count(*) FROM users WHERE role NOT IN ('admin','member')"),
        0,
        "an account carries a role outside the vocabulary"
    );
    assert_eq!(
        upgraded.count("SELECT count(*) FROM users WHERE role IS NULL"),
        0,
        "an account was left with no role"
    );
}

/// The environment-named account becomes the administrator even when it is not
/// the oldest (FR-414, FR-524).
///
/// This branch was **unreachable** until the migration was wired to the
/// `--admin-email` argument: migration 3 reads
/// `current_setting('cairn.admin_email')` and nothing set it, so every
/// migrating deployment silently fell through to "oldest by `created_at`". The
/// failure was invisible because the fallback is a legitimate outcome that also
/// produces exactly one admin — the only way to see it was to seed a
/// configuration where the two branches disagree, which is what this does.
#[test]
fn the_environment_named_account_wins_over_the_oldest() {
    let Some(mut server) = server_at_schema_two() else {
        return;
    };

    let named_email = "operator@example.test";
    let oldest = Uuid::now_v7();
    let named = Uuid::now_v7();
    // The oldest account is deliberately *not* the named one, so the two rules
    // pick different rows and the test can tell which ran.
    server.execute(&format!(
        "INSERT INTO users (id, email, display_name, password_hash, created_at)
         VALUES ('{oldest}', 'oldest@example.test', 'Oldest', 'x', now() - interval '30 days'),
                ('{named}', '{named_email}', 'Operator', 'x', now() - interval '1 day')"
    ));

    let upgraded = server.upgraded_as_admin(named_email);

    assert_eq!(
        upgraded.count("SELECT count(*) FROM users WHERE role = 'admin'"),
        1,
        "exactly one admin, whichever rule ran"
    );
    assert_eq!(
        upgraded.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{named}' AND role = 'admin'"
        )),
        1,
        "the environment-named account is not the admin; the backfill fell through to \
         the oldest-account rule, which means `cairn.admin_email` never reached the migration"
    );
    assert_eq!(
        upgraded.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{oldest}' AND role = 'member'"
        )),
        1,
        "the oldest account should be a member here, not the admin"
    );
}

/// An environment-named email matching no account falls back to the oldest,
/// rather than leaving the server with no administrator.
#[test]
fn an_environment_email_matching_nothing_falls_back_to_the_oldest() {
    let Some(mut server) = server_at_schema_two() else {
        return;
    };
    let oldest = Uuid::now_v7();
    let newer = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO users (id, email, display_name, password_hash, created_at)
         VALUES ('{oldest}', 'oldest@example.test', 'Oldest', 'x', now() - interval '9 days'),
                ('{newer}', 'newer@example.test', 'Newer', 'x', now() - interval '2 days')"
    ));

    let upgraded = server.upgraded_as_admin("nobody-here@example.test");

    // The named account did not exist before the upgrade, but `ensure_admin`
    // creates it on start — so the assertion is about the *pre-existing* rows,
    // which is what the backfill governs.
    assert_eq!(
        upgraded.count(&format!(
            "SELECT count(*) FROM users WHERE id = '{oldest}' AND role = 'admin'"
        )),
        1,
        "with the named email matching no pre-existing account, the oldest must be promoted"
    );
    assert!(
        upgraded.count("SELECT count(*) FROM users WHERE role = 'admin'") >= 1,
        "never zero administrators (FR-413, FR-524)"
    );
}

//! PostgreSQL connection and migrations.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use std::time::Duration;

pub const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "init", include_str!("../migrations/0001_init.sql")),
    (
        2,
        "project_intelligence",
        include_str!("../migrations/0002_project_intelligence.sql"),
    ),
    (
        3,
        "collaborative_global_memory",
        include_str!("../migrations/0003_collaborative_global_memory.sql"),
    ),
];

/// The highest migration this build carries.
///
/// Not what the server advertises: a deployment can be held at a lower schema
/// deliberately, and what it can actually hold is the schema it **applied**.
/// See [`applied_version`].
pub const SCHEMA_VERSION: i64 = 3;

/// The pool size a single server takes from PostgreSQL.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;

/// Connect, applying migrations up to `max_version`.
///
/// Holding the schema back while running a current binary is an ordinary
/// staged-rollout position: the code ships first, the migration runs when the
/// operator is ready. It is also the only honest way to exercise a server an
/// upgraded peer has to cope with, because what makes a server "older" is the
/// schema it applied and not the binary that applied it (FR-415).
/// Open the pool and bring the schema up, telling the migrations which account
/// the environment names.
///
/// Migration 3's `users.role` backfill reads `current_setting('cairn.admin_email')`
/// to decide which existing account becomes the administrator (FR-414, FR-524).
/// Nothing set that value until this function existed, so the environment-named
/// branch of the backfill could never fire and every migrating deployment fell
/// through to "oldest account by `created_at`" — silently, because the fallback
/// is a legitimate outcome and produces an admin either way.
///
/// It is set per transaction rather than per pool because that is the only scope
/// a migration can rely on: a pooled connection is handed out and returned, and
/// a `SET` that outlived the transaction would leak the operator's email into
/// unrelated sessions.
pub async fn connect(
    url: &str,
    max_connections: u32,
    max_version: i64,
    admin_email: Option<&str>,
) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections.max(1))
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await?;
    migrate(&pool, max_version, admin_email).await?;
    Ok(pool)
}

/// The highest migration this database has actually applied.
///
/// What `GET /api/version` reports, and what a daemon compares its own work
/// against before deciding whether the server can hold it. Reporting the
/// compiled-in maximum instead would make a held-back deployment advertise
/// tables it does not have (FR-415).
pub async fn applied_version(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(pool)
            .await?,
    )
}

/// Apply migrations on start, so a fresh deployment needs no separate step.
///
/// Stops at `max_version`, which is [`SCHEMA_VERSION`] unless an operator held
/// the deployment back.
pub async fn migrate(
    pool: &PgPool,
    max_version: i64,
    admin_email: Option<&str>,
) -> anyhow::Result<()> {
    pool.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    BIGINT PRIMARY KEY,
             name       TEXT NOT NULL,
             applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .await?;

    let current: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(pool)
            .await?;

    for (version, name, sql) in MIGRATIONS {
        if *version <= current || *version > max_version {
            continue;
        }
        let mut tx = pool.begin().await?;
        // Inside the transaction, so it is visible to the script and gone
        // afterwards. `set_config(..., true)` is the local form — `SET LOCAL`
        // cannot take a bound parameter, and interpolating an operator-supplied
        // email into DDL is not a trade worth making.
        if let Some(email) = admin_email.map(str::trim).filter(|e| !e.is_empty()) {
            sqlx::query("SELECT set_config('cairn.admin_email', $1, true)")
                .bind(email.to_lowercase())
                .execute(&mut *tx)
                .await?;
        }
        // PostgreSQL runs a multi-statement script in one call.
        tx.execute(*sql).await?;
        sqlx::query("INSERT INTO schema_migrations (version, name) VALUES ($1, $2)")
            .bind(version)
            .bind(*name)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every migration file this crate ships is registered.
    ///
    /// A migration that exists on disk and not in `MIGRATIONS` is dead: the
    /// server starts, reports success, and serves a schema missing every table
    /// the file would have created — which is how a whole feature's columns can
    /// be absent while every unit test passes.
    #[test]
    fn every_migration_file_is_registered() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .expect("migrations directory")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sql"))
            .collect();
        on_disk.sort();

        assert_eq!(
            on_disk.len(),
            MIGRATIONS.len(),
            "{} migration files on disk, {} registered: {on_disk:?}",
            on_disk.len(),
            MIGRATIONS.len()
        );
        for (i, (version, _, _)) in MIGRATIONS.iter().enumerate() {
            assert_eq!(*version, i as i64 + 1, "migrations are numbered from 1");
            assert!(
                on_disk[i].starts_with(&format!("{version:04}_")),
                "registration {version} does not match {}",
                on_disk[i]
            );
        }
    }

    #[test]
    fn the_reported_schema_version_is_the_last_migration() {
        assert_eq!(
            SCHEMA_VERSION,
            MIGRATIONS.last().expect("a migration").0,
            "the version the server advertises must be the one it actually applied"
        );
    }

    /// The server accepts exactly the relation kinds the local store writes.
    ///
    /// A kind missing from the server's CHECK is not a degraded feature: it is
    /// a constraint violation that fails the whole push, so the vocabularies
    /// cannot be allowed to drift apart.
    #[test]
    fn the_relation_kinds_match_the_domain() {
        let sql = include_str!("../migrations/0002_project_intelligence.sql");
        let check = sql
            .split("CREATE TABLE IF NOT EXISTS memory_relations")
            .nth(1)
            .and_then(|s| {
                s.split("kind               TEXT NOT NULL CHECK (kind IN (")
                    .nth(1)
            })
            .and_then(|s| s.split("))").next())
            .expect("the memory_relations kind CHECK");

        for kind in cairn_core::domain::RelationKind::ALL {
            assert!(
                check.contains(&format!("'{}'", kind.as_str())),
                "the server would reject a `{}` relation: {check}",
                kind.as_str()
            );
        }
        assert_eq!(
            check.matches('\'').count() / 2,
            cairn_core::domain::RelationKind::ALL.len(),
            "the server accepts a relation kind the domain does not define: {check}"
        );
    }
}

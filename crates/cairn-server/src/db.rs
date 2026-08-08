//! PostgreSQL connection and migrations.

use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, PgPool};
use std::time::Duration;

pub const MIGRATIONS: &[(i64, &str, &str)] =
    &[(1, "init", include_str!("../migrations/0001_init.sql"))];

/// The pool size a single server takes from PostgreSQL.
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;

pub async fn connect(url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(max_connections.max(1))
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

/// Apply migrations on start, so a fresh deployment needs no separate step.
pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
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
        if *version <= current {
            continue;
        }
        let mut tx = pool.begin().await?;
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

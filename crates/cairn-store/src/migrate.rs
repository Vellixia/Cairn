//! Versioned, forward-only migrations with a schema-version guard.
//!
//! The database records its schema version and refuses to open a newer one, so
//! an older `cairnd` can never write against a schema it does not understand.

use sqlx::{Executor, SqlitePool};

/// Migrations in application order. Embedded, so there is nothing to install.
pub const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "init", include_str!("../migrations/0001_init.sql")),
    (
        2,
        "memory_fts",
        include_str!("../migrations/0002_memory_fts.sql"),
    ),
    (
        3,
        "outbox_claim",
        include_str!("../migrations/0003_outbox_claim.sql"),
    ),
    (
        4,
        "integrations",
        include_str!("../migrations/0004_integrations.sql"),
    ),
    (
        5,
        "project_intelligence",
        include_str!("../migrations/0005_project_intelligence.sql"),
    ),
    (
        6,
        "sync_deferred",
        include_str!("../migrations/0006_sync_deferred.sql"),
    ),
];

/// The schema version this build knows how to use.
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|(v, _, _)| *v).unwrap_or(0)
}

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); \
         upgrade Cairn"
    )]
    TooNew { found: i64, supported: i64 },
}

/// Apply every migration the database has not seen yet.
pub async fn run(pool: &SqlitePool) -> Result<i64, MigrateError> {
    run_to(pool, latest_version()).await
}

/// Apply migrations up to and including `target`, and refuse a database that
/// is already past it.
///
/// This is what `run` does with `target = latest_version()`. It is separate so
/// that a caller can stand a database up at an *older* schema through the real
/// migration scripts rather than through hand-written DDL — which is how the
/// alpha.4 fixture is built and how the schema-version guard is exercised
/// (migration.md §Proof, assertions 11–12). A hand-written approximation of a
/// historical schema proves the migration works against the schema someone
/// wrote down, not against the one users actually have.
pub async fn run_to(pool: &SqlitePool, target: i64) -> Result<i64, MigrateError> {
    pool.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT NOT NULL,
             applied_at TEXT NOT NULL
         )",
    )
    .await?;

    let current: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(pool)
            .await?;

    let supported = target;
    if current > supported {
        return Err(MigrateError::TooNew {
            found: current,
            supported,
        });
    }

    for (version, name, sql) in MIGRATIONS {
        if *version <= current || *version > target {
            continue;
        }
        let mut tx = pool.begin().await?;
        // SQLite executes multi-statement scripts one statement at a time.
        for statement in split_statements(sql) {
            tx.execute(statement.as_str()).await?;
        }
        // A migration step that SQL cannot express honestly, run inside the
        // same transaction so an interruption still rolls the whole migration
        // back.
        finish(*version, &mut tx).await?;
        sqlx::query(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
        )
        .bind(version)
        .bind(*name)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    Ok(supported)
}

/// The part of a migration SQL cannot express honestly.
///
/// Runs inside the migration's own transaction, after its script and before its
/// `schema_migrations` row, so the two are atomic together.
async fn finish(version: i64, tx: &mut sqlx::SqliteConnection) -> Result<(), MigrateError> {
    match version {
        5 => criteria_from_acceptance_arrays(tx).await,
        _ => Ok(()),
    }
}

/// Migration 5, step 6 — one `task_criteria` row per acceptance-criteria entry.
///
/// In Rust rather than SQL because a criterion's `id` is a UUIDv7 by the
/// convention every other identifier in this schema follows, and SQLite can
/// only produce a random value *shaped* like one. Claiming a time ordering the
/// value does not have would be a small lie in a table other code sorts by.
///
/// Rules, from migration.md §Step 5:
///
/// - position order, `ordinal` 1-based, `label` = `AC-<ordinal>`;
/// - `created_at` from the **task**, not from now — the criterion is as old as
///   the task it belongs to;
/// - duplicate strings produce distinct rows, because they were distinct
///   entries and merging them would lose one;
/// - an empty array produces no rows, and a deleted task produces none;
/// - `tasks.acceptance_criteria` is **not** modified: it is already exactly the
///   projection `rebuild_criteria_projection` computes.
async fn criteria_from_acceptance_arrays(
    tx: &mut sqlx::SqliteConnection,
) -> Result<(), MigrateError> {
    let tasks: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, acceptance_criteria, created_at FROM tasks WHERE deleted_at IS NULL",
    )
    .fetch_all(&mut *tx)
    .await?;

    for (task_id, criteria_json, created_at) in tasks {
        // A malformed array is left alone rather than guessed at. The task
        // keeps working exactly as it did, and `doctor` can report it.
        let Ok(items) = serde_json::from_str::<Vec<String>>(&criteria_json) else {
            continue;
        };
        for (index, text) in items.iter().enumerate() {
            let ordinal = index as i64 + 1;
            sqlx::query(
                "INSERT INTO task_criteria
                     (id, task_id, ordinal, label, text, state, verification,
                      revision, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 'unverified', 1, ?6, ?6)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&task_id)
            .bind(ordinal)
            .bind(format!("AC-{ordinal}"))
            .bind(text)
            .bind(&created_at)
            .execute(&mut *tx)
            .await?;
        }
    }
    Ok(())
}

/// Split a script on `;` boundaries, honouring `BEGIN ... END` trigger bodies.
///
/// SQLite executes one statement per call, and a trigger body contains
/// semicolons of its own — so a naive split on `;` produces "incomplete input".
fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth = 0usize;

    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") || trimmed.is_empty() {
            continue;
        }
        buf.push_str(line);
        buf.push('\n');

        // Count BEGIN/END as whole words, wherever they appear on the line.
        for token in trimmed.split(|c: char| !c.is_ascii_alphanumeric()) {
            match token.to_ascii_uppercase().as_str() {
                "BEGIN" => depth += 1,
                "END" => depth = depth.saturating_sub(1),
                _ => {}
            }
        }

        if depth == 0 && trimmed.ends_with(';') {
            let stmt = buf.trim().to_string();
            if !stmt.is_empty() {
                out.push(stmt);
            }
            buf.clear();
        }
    }
    let tail = buf.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_trigger_bodies_intact() {
        let sql = "CREATE TABLE t (a);\n\
                   CREATE TRIGGER g AFTER INSERT ON t BEGIN\n\
                     INSERT INTO t VALUES (1);\n\
                   END;\n";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2, "{stmts:#?}");
        assert!(stmts[1].contains("END;"));
    }
}

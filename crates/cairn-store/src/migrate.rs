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

    let supported = latest_version();
    if current > supported {
        return Err(MigrateError::TooNew {
            found: current,
            supported,
        });
    }

    for (version, name, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        let mut tx = pool.begin().await?;
        // SQLite executes multi-statement scripts one statement at a time.
        for statement in split_statements(sql) {
            tx.execute(statement.as_str()).await?;
        }
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

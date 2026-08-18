//! Debug-only contention reporting.
//!
//! When a SQLite write loses a race, the message that reaches the user —
//! "database is locked" — says nothing about *which* write, or at which stage
//! of the transaction. Chasing that by reasoning is how a fix ends up aimed at
//! the wrong statement, so the store reports it instead.
//!
//! Off unless `CAIRN_CONTENTION_LOG` names a file. Nothing here records
//! payloads, content, paths or identifiers: a contention report is an
//! operation name, a stage, and two integers.

use crate::{Result, Store};
use std::io::Write;
use std::str::FromStr;
use std::sync::OnceLock;
use uuid::Uuid;

/// Where in a write the contention appeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Acquiring the write lock with `BEGIN IMMEDIATE`.
    Begin,
    /// A statement inside an open transaction.
    Body,
    /// `COMMIT`.
    Commit,
    /// `ROLLBACK`.
    Rollback,
    /// A single statement in autocommit, outside any transaction.
    Autocommit,
    /// Reported by the catch-all, which sees the error but not the stage.
    Unknown,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Begin => "begin_immediate",
            Stage::Body => "statement",
            Stage::Commit => "commit",
            Stage::Rollback => "rollback",
            Stage::Autocommit => "autocommit",
            Stage::Unknown => "unknown",
        }
    }
}

fn sink() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| sink_path(std::env::var("CAIRN_CONTENTION_LOG").ok().as_deref()))
        .as_deref()
}

/// The path reporting writes to, for a given raw value of the variable.
///
/// Split out from [`sink`] so it can be tested. `sink` memoises in a
/// `OnceLock`, which is right for a per-process setting but means a test that
/// sets the variable and calls [`enabled`] observes only whatever the first
/// caller in the process happened to cache — it proves nothing about the
/// decision itself. This is the decision.
fn sink_path(raw: Option<&str>) -> Option<String> {
    raw.filter(|p| !p.is_empty()).map(str::to_owned)
}

/// True when contention reporting is switched on.
pub fn enabled() -> bool {
    sink().is_some()
}

/// The extended and primary result codes of a database error, if it has any.
pub fn codes(e: &sqlx::Error) -> Option<(i64, i64)> {
    let sqlx::Error::Database(db) = e else {
        return None;
    };
    let extended = db.code().and_then(|c| c.parse::<i64>().ok())?;
    Some((extended, extended & 0xff))
}

/// Whether SQLite is saying "try again": `SQLITE_BUSY` or `SQLITE_LOCKED`, in
/// any of their extended forms.
pub fn is_contention(e: &sqlx::Error) -> bool {
    matches!(codes(e), Some((_, 5 | 6)))
}

/// Record one contention event. A no-op unless reporting is switched on.
pub fn record(op: &str, stage: Stage, entity: &str, attempt: u32, e: &sqlx::Error) {
    let Some(path) = sink() else { return };
    let Some((extended, primary)) = codes(e) else {
        return;
    };

    let mut line = format!(
        "op={op} stage={} entity={entity} extended={extended} primary={primary} attempt={attempt}",
        stage.as_str()
    );
    // The catch-all knows the error but not who raised it; the stack does.
    if stage == Stage::Unknown {
        line.push_str(&format!(" frames=[{}]", store_frames()));
    }
    line.push('\n');

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// The `cairn` frames of the current stack, innermost first, names only.
///
/// Needs `RUST_BACKTRACE=1`; without it this is empty and the report still
/// carries its codes.
fn store_frames() -> String {
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let mut seen = Vec::new();
    for line in bt.lines() {
        let line = line.trim();
        let Some(rest) = line.split_once(": ").map(|(_, r)| r) else {
            continue;
        };
        if !(rest.starts_with("cairn_store::") || rest.starts_with("cairnd::")) {
            continue;
        }
        let name = rest.split("::{{").next().unwrap_or(rest).to_string();
        if !seen.contains(&name) {
            seen.push(name);
        }
        if seen.len() >= 6 {
            break;
        }
    }
    seen.join(" <- ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stage_has_a_stable_string_form() {
        assert_eq!(Stage::Begin.as_str(), "begin_immediate");
        assert_eq!(Stage::Body.as_str(), "statement");
        assert_eq!(Stage::Commit.as_str(), "commit");
        assert_eq!(Stage::Rollback.as_str(), "rollback");
        assert_eq!(Stage::Autocommit.as_str(), "autocommit");
        assert_eq!(Stage::Unknown.as_str(), "unknown");
    }

    /// Reporting is opt-in, and an empty value is not an opt-in.
    ///
    /// Asserted against `sink_path` rather than `enabled`, which reads a
    /// `OnceLock`: a test that set the variable and called `enabled` would be
    /// asserting on whatever the process's first caller cached, and would
    /// pass or fail on test *order* and on whether the developer happens to
    /// have the variable exported. This has neither dependency.
    #[test]
    fn reporting_is_off_unless_a_non_empty_path_is_given() {
        assert_eq!(sink_path(None), None, "unset means off");
        assert_eq!(sink_path(Some("")), None, "empty is not an opt-in");
        assert_eq!(
            sink_path(Some("/tmp/contention.log")),
            Some("/tmp/contention.log".to_string()),
            "a path switches reporting on"
        );
    }

    #[tokio::test]
    async fn a_constraint_violation_is_not_contention() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::Store::open(&dir.path().join("d.sqlite3"))
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (a INTEGER PRIMARY KEY)")
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1)")
            .execute(store.pool())
            .await
            .unwrap();
        let err = sqlx::query("INSERT INTO t VALUES (1)")
            .execute(store.pool())
            .await
            .expect_err("duplicate");
        assert!(!is_contention(&err), "a constraint violation is not a lock");
        let (_, primary) = codes(&err).expect("a database error");
        assert_eq!(primary, 19, "SQLITE_CONSTRAINT's primary code");
        store.close().await;
    }

    #[tokio::test]
    async fn a_non_database_error_has_no_codes() {
        let err = sqlx::Error::RowNotFound;
        assert!(codes(&err).is_none());
        assert!(!is_contention(&err));
    }
}

/// What one derived value's rebuild found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RebuildOutcome {
    /// The derived value, named as a developer would name it.
    pub derived: &'static str,
    /// How many records were recomputed.
    pub checked: i64,
    /// How many disagreed with their rebuild. A release where any of these is
    /// non-zero ships a known inconsistency (FR-478, SC-324).
    pub differed: i64,
}

/// Recompute **every** derived value in a project and report what differed
/// (FR-478, FR-518, SC-324).
///
/// Six derived values, and the list is exhaustive on purpose: a rebuild that
/// silently skips one is worse than no rebuild at all, because it reports "no
/// differences" over a value it never looked at.
///
///   1. `memories.state` / `superseded_by_id` — a view of the `supersedes`
///      relations
///   2. `reinforcement_count` / `distinct_origin_count` — counted from the
///      reinforcing and duplicating decisions
///   3. `memories.verification` / `verification_authority` — derived from the
///      recorded runs and the collectors behind them
///   4. `tasks.acceptance_criteria` — the projection of the criteria rows
///   5. the task state digest — derived from the criteria and blockers
///   6. `reusable_patterns.trust` — derived from the applications
pub async fn rebuild_derived(store: &Store, project_id: Uuid) -> Result<Vec<RebuildOutcome>> {
    let mut out = Vec::new();

    // 1. Supersession. The rebuild itself returns how many rows disagreed.
    let differed = crate::knowledge::rebuild_supersession(store, project_id).await? as i64;
    let memories: Vec<Uuid> = sqlx::query_scalar::<_, String>(
        "SELECT id FROM memories WHERE project_id = ?1 AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?
    .iter()
    .filter_map(|s| Uuid::from_str(s).ok())
    .collect();
    out.push(RebuildOutcome {
        derived: "memory lifecycle state",
        checked: memories.len() as i64,
        differed,
    });

    // 2 and 3. Per memory: the counts, then the verification.
    let mut counts_differed = 0;
    let mut verification_differed = 0;
    for id in &memories {
        let before: (i64, i64) = sqlx::query_as(
            "SELECT reinforcement_count, distinct_origin_count FROM memories WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await?;
        let after = crate::knowledge::rebuild_reinforcement(store, *id).await?;
        if before != after {
            counts_differed += 1;
        }

        let stored: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT verification, verification_authority FROM memories WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await?;
        let (state, authority) = crate::evidence::rebuild_verification(store, *id).await?;
        let rebuilt = (
            Some(state.as_str().to_string()),
            authority.map(|a| a.as_str().to_string()),
        );
        // A memory that never had a verification reads as NULL and rebuilds as
        // `unverified`; that is the same answer written two ways, not a
        // difference.
        let same = stored.0.as_deref().unwrap_or("unverified")
            == rebuilt.0.as_deref().unwrap_or("")
            && stored.1 == rebuilt.1;
        if !same {
            verification_differed += 1;
        }
    }
    out.push(RebuildOutcome {
        derived: "reinforcement counts",
        checked: memories.len() as i64,
        differed: counts_differed,
    });
    out.push(RebuildOutcome {
        derived: "verification state and authority",
        checked: memories.len() as i64,
        differed: verification_differed,
    });

    // 4 and 5. Per task: the criteria projection, then the state digest.
    let tasks: Vec<Uuid> = sqlx::query_scalar::<_, String>(
        "SELECT id FROM tasks WHERE project_id = ?1 AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?
    .iter()
    .filter_map(|s| Uuid::from_str(s).ok())
    .collect();

    let mut projection_differed = 0;
    let mut digest_differed = 0;
    for id in &tasks {
        let before = crate::repo::task(store, *id).await?.acceptance_criteria;
        let digest_before = crate::criteria::state_digest(store, *id).await?;

        let after = crate::criteria::rebuild_criteria_projection(store, *id).await?;
        let digest_after = crate::criteria::state_digest(store, *id).await?;

        if before != after {
            projection_differed += 1;
        }
        if digest_before != digest_after {
            digest_differed += 1;
        }
    }
    out.push(RebuildOutcome {
        derived: "task criteria projection",
        checked: tasks.len() as i64,
        differed: projection_differed,
    });
    out.push(RebuildOutcome {
        derived: "task state digest",
        checked: tasks.len() as i64,
        differed: digest_differed,
    });

    // 6. Patterns have no project, so they are rebuilt whole. A pattern's trust
    // is derived from applications that may come from any project, and there is
    // no per-project slice of it to rebuild.
    let patterns = crate::patterns::list(store, None).await?;
    let mut trust_differed = 0;
    for p in &patterns {
        let after = crate::patterns::rebuild_pattern_trust(store, p.id).await?;
        if after != p.trust {
            trust_differed += 1;
        }
    }
    out.push(RebuildOutcome {
        derived: "pattern trust",
        checked: patterns.len() as i64,
        differed: trust_differed,
    });

    Ok(out)
}

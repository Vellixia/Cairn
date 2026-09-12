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
use cairn_core::domain::SyncNamespace;
use chrono::{DateTime, Utc};
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

// ---------------------------------------------------------------------------
// Durability classes (T087, FR-703, FR-705, FR-706, FR-707, FR-709, FR-710a)
// ---------------------------------------------------------------------------

/// What deleting this local store would cost the category holding a record.
///
/// Four classes, and the distinction that matters most is between the middle
/// two. `Cache` and `ServerDurable` both survive local loss; `Cache` and
/// `QueuedForServer` both describe rows the server does not own yet or does not
/// own here. Collapsing either pair is how a durability report becomes a
/// reassurance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum DurabilityClass {
    /// The server holds it and is its canonical owner (FR-701). Nothing local
    /// is in this class — see [`local_inventory`] — and the variant exists
    /// because a caller reporting durability reports both sides of the ledger,
    /// and a ledger with only one side is not a ledger.
    ServerDurable,
    /// A non-authoritative local copy of something the server owns (FR-710).
    /// Refilled by the merge paths in [`crate::global`] and by
    /// [`crate::repo::import_memory`].
    Cache,
    /// A write that has not been accepted yet and has a path to acceptance
    /// (FR-709). Not an alternative truth — a request in a queue — but the rows
    /// are the only copy until the server takes them.
    QueuedForServer,
    /// It exists here and nowhere else, by design (FR-706, FR-707). Deleting
    /// the store destroys it.
    LocalOnly,
}

impl DurabilityClass {
    /// The stable string form, for a report a user or a test reads.
    pub fn as_str(&self) -> &'static str {
        match self {
            DurabilityClass::ServerDurable => "server_durable",
            DurabilityClass::Cache => "cache",
            DurabilityClass::QueuedForServer => "queued_for_server",
            DurabilityClass::LocalOnly => "local_only",
        }
    }

    /// Whether deleting this store would destroy the knowledge in this class.
    ///
    /// True for [`DurabilityClass::ServerDurable`] and [`DurabilityClass::Cache`]
    /// only, and the two survive differently — the difference is the whole
    /// reason they are separate variants rather than one "safe" flag.
    ///
    /// A cache survives **the knowledge, not the rows**. The rows here are
    /// destroyed with the store, every one of them; what survives is the
    /// server's copy, and a later pull writes new rows that say the same things
    /// (FR-703, FR-704). So the honest statement about a cache is "nothing is
    /// lost that the server accepted", which is narrower than "nothing is lost"
    /// in two ways worth stating out loud: a refill needs the server to be
    /// reachable, and until it happens the cache is empty rather than absent —
    /// which is why [`cache_status`] exists and why FR-710a requires an empty
    /// cache to be reportable as empty.
    ///
    /// [`DurabilityClass::QueuedForServer`] is false, and that is not a
    /// pessimistic reading. A queued row has a path to acceptance, but it has
    /// not been accepted; deleting the store deletes the only copy, and the
    /// server never learns the write existed.
    pub fn survives_local_loss(&self) -> bool {
        matches!(
            self,
            DurabilityClass::ServerDurable | DurabilityClass::Cache
        )
    }
}

/// One category of local data, and what deleting the store would do to it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CategoryDurability {
    /// The category, named as a developer would name it rather than as the
    /// schema does — one category can span several tables.
    pub category: &'static str,
    pub class: DurabilityClass,
    pub rows: i64,
    /// Exactly which tables this category speaks for.
    ///
    /// Carried in the report rather than kept private to the definition,
    /// because a category name is a label and the claim being made about it is
    /// a durability guarantee. A reader asked to accept "cached patterns
    /// survive local loss" can see that the statement covers `cached_patterns`
    /// and not `reusable_patterns`, which is the distinction most likely to be
    /// misread.
    pub tables: &'static [&'static str],
}

/// One category's definition: how it is counted, and which tables it accounts
/// for.
///
/// `tables` is not read by the count — it is what makes the completeness claim
/// checkable, and it is passed through to the report so a reader can see which
/// tables a category speaks for. Every table in the local schema has to appear
/// in this list somewhere, and `every_local_table_is_accounted_for` in this
/// module's tests is what holds the two in step: a table added by a future
/// migration and forgotten here fails that test rather than quietly vanishing
/// from the inventory.
struct CategorySpec {
    category: &'static str,
    class: DurabilityClass,
    /// A statement returning exactly one integer column.
    count_sql: &'static str,
    tables: &'static [&'static str],
}

/// Every category of data this store holds, with the class each one is in.
///
/// **The list is exhaustive and fixed**, for the reason [`rebuild_derived`]
/// gives about derived values: an inventory that silently omits a category is
/// worse than no inventory, because it answers "here is what you would lose"
/// over data it never looked at, and the omission is invisible in the answer.
/// FR-705 requires Cairn to be able to state which categories would not survive
/// deletion, and a statement with a hole in it does not satisfy that — it
/// contradicts it.
///
/// **The class is not a judgement made here.** A category is `Cache` when the
/// record has a path to the server, `LocalOnly` when the schema gives it none,
/// and the schema is where that is already written down: `OutboxEntityType` has
/// no variant for observations, evidence facts, verification runs, continuity
/// checkpoints, reusable patterns, pattern applications, task changes,
/// criterion evidence, project traits or writer identity, and that absence is
/// what makes "they stay local" a property of the schema rather than a promise
/// (FR-503, FR-707). This table restates that classification; it does not form
/// a second opinion about it.
const CATEGORIES: &[CategorySpec] = &[
    // -- Cache: the server owns these, and a refill writes them back ---------
    CategorySpec {
        // Project memory, which is the bulk of what a user would think they
        // were losing. Soft-deleted rows are excluded: they are already gone
        // from every read, and counting them here would inflate the loss with
        // content the user themselves ended.
        category: "project memory",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM memories \
                    WHERE local_only = 0 AND deleted_at IS NULL",
        tables: &["memories"],
    },
    CategorySpec {
        // A relation names two memories and belongs to neither, so it travels
        // as its own outbox entity rather than inside a row's payload — which
        // is exactly why it is cached rather than local.
        category: "memory relations",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM memory_relations WHERE deleted_at IS NULL",
        tables: &["memory_relations"],
    },
    CategorySpec {
        category: "personal knowledge",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM personal_knowledge WHERE forgotten_at IS NULL",
        tables: &[
            "personal_knowledge",
            "personal_knowledge_applicability",
            "personal_knowledge_relations",
        ],
    },
    CategorySpec {
        category: "team knowledge",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM team_knowledge",
        tables: &[
            "team_knowledge",
            "team_knowledge_applicability",
            "team_knowledge_relations",
        ],
    },
    CategorySpec {
        // The pattern cache proper (migration 9). Distinct from
        // `reusable_patterns` below, which is local and is counted separately —
        // reporting the two as one number would be the exact confusion the two
        // tables exist to prevent.
        category: "cached patterns",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM cached_patterns WHERE forgotten_at IS NULL",
        tables: &["cached_patterns"],
    },
    CategorySpec {
        category: "projects",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM projects WHERE deleted_at IS NULL",
        tables: &["projects"],
    },
    CategorySpec {
        category: "tasks",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM tasks WHERE deleted_at IS NULL",
        tables: &["tasks", "task_criteria", "task_blockers"],
    },
    CategorySpec {
        category: "sessions",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM sessions WHERE deleted_at IS NULL",
        tables: &["sessions"],
    },
    CategorySpec {
        category: "handoffs",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM handoffs WHERE deleted_at IS NULL",
        tables: &["handoffs"],
    },
    CategorySpec {
        // Accounts this store has seen. The account itself lives on the server;
        // the local row is a name and an email cached for display.
        category: "accounts",
        class: DurabilityClass::Cache,
        count_sql: "SELECT COUNT(*) FROM users",
        tables: &["users"],
    },
    CategorySpec {
        // The FTS indexes and their shadow tables. Counted as one category and
        // classed with what they index: an index holds no knowledge of its own
        // and is rebuilt from the rows, so losing it costs whatever losing
        // those rows costs and nothing more.
        category: "lexical search indexes",
        class: DurabilityClass::Cache,
        count_sql: "SELECT (SELECT COUNT(*) FROM memory_fts) \
                         + (SELECT COUNT(*) FROM personal_fts) \
                         + (SELECT COUNT(*) FROM team_fts)",
        tables: &[
            "memory_fts",
            "memory_fts_config",
            "memory_fts_data",
            "memory_fts_docsize",
            "memory_fts_idx",
            "personal_fts",
            "personal_fts_config",
            "personal_fts_data",
            "personal_fts_docsize",
            "personal_fts_idx",
            "team_fts",
            "team_fts_config",
            "team_fts_data",
            "team_fts_docsize",
            "team_fts_idx",
        ],
    },
    // -- Queued for the server: the only copy, until it is accepted ----------
    CategorySpec {
        // Only rows the server has not taken. `delivered` is excluded because a
        // delivered row is a receipt, not a pending write, and counting it
        // would report accepted work as at risk.
        category: "spooled events",
        class: DurabilityClass::QueuedForServer,
        count_sql: "SELECT COUNT(*) FROM event_spool WHERE state <> 'delivered'",
        tables: &["event_spool"],
    },
    CategorySpec {
        category: "spooled knowledge commands",
        class: DurabilityClass::QueuedForServer,
        count_sql: "SELECT COUNT(*) FROM command_spool WHERE state <> 'delivered'",
        tables: &["command_spool"],
    },
    CategorySpec {
        // Feature 004's outbox, which is still how projects, tasks, sessions,
        // handoffs, relations and global knowledge reach the server. Its
        // undelivered rows are queued writes by exactly FR-709's definition, so
        // they are reported as such rather than folded into the cached
        // categories whose records they carry.
        category: "outbox",
        class: DurabilityClass::QueuedForServer,
        count_sql: "SELECT COUNT(*) FROM outbox WHERE state <> 'delivered'",
        tables: &["outbox"],
    },
    // -- Local only: FR-707's list, and the machine state around it ----------
    CategorySpec {
        // The raw record of what an agent did. Never synchronized (FR-055, D9)
        // and never will be: there is no `OutboxEntityType` variant for it.
        category: "observations",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL",
        tables: &["observations"],
    },
    CategorySpec {
        // The links from a memory to the observations behind it. Local for the
        // same reason the observations are: the link is worthless without the
        // record it points at.
        category: "memory evidence references",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM memory_evidence",
        tables: &["memory_evidence"],
    },
    CategorySpec {
        category: "evidence facts",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM evidence_facts WHERE deleted_at IS NULL",
        tables: &["evidence_facts", "memory_evidence_facts"],
    },
    CategorySpec {
        category: "verification runs",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM verification_runs",
        tables: &["verification_runs"],
    },
    CategorySpec {
        category: "continuity checkpoints",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM continuity_checkpoints WHERE deleted_at IS NULL",
        tables: &["continuity_checkpoints"],
    },
    CategorySpec {
        // The local, pre-promotion pattern rows — the ones carrying `signals`,
        // `signal_digest` and `origin_ref`, which the privacy boundary refuses
        // and which therefore have nowhere to go. A pattern becomes durable by
        // being promoted into the safe shape, not by this table syncing.
        category: "reusable patterns (local)",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM reusable_patterns WHERE deleted_at IS NULL",
        tables: &["reusable_patterns"],
    },
    CategorySpec {
        // Applications stay local under FR-707 even though the pattern they
        // apply to does not. They are evidence about one machine's experience,
        // and they are what local `validated`/`contested` trust is derived from
        // — which is why the server can never establish either level.
        category: "pattern applications",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM pattern_applications",
        tables: &["pattern_applications"],
    },
    CategorySpec {
        category: "task change history",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM task_changes",
        tables: &["task_changes"],
    },
    CategorySpec {
        category: "criterion evidence",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM criterion_evidence",
        tables: &["criterion_evidence"],
    },
    CategorySpec {
        // FR-706's exclusion, counted separately from `project memory` and
        // never folded into it. A user who marked something local-only was told
        // at the point of choosing that it would not survive; reporting it
        // inside the cached total would take that back.
        category: "local-only memory",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM memories WHERE local_only = 1 AND deleted_at IS NULL",
        tables: &["memories"],
    },
    CategorySpec {
        // Records the server would not accept, kept rather than dropped
        // (FR-871). Local by definition: being here is what "the server refused
        // it" means.
        category: "retained local records",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM retained_local",
        tables: &["retained_local"],
    },
    CategorySpec {
        // Derived from this machine's own checkout, never synchronized
        // (FR-438). Cheap to rebuild, which is not the same as durable, so it
        // is reported as lost like anything else that is.
        category: "project traits",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM project_traits",
        tables: &["project_traits"],
    },
    CategorySpec {
        // The store's one opaque writer identity (D407, FR-490). Losing it is
        // not losing knowledge, but it is losing continuity: a new store mints
        // a new one, and the writer sequences peers were tracking end.
        category: "writer identity",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM writer_identity",
        tables: &["writer_identity"],
    },
    CategorySpec {
        // Which agents and managers are installed here, what was written into
        // their config, and the evidence behind a capability probe. Machine
        // state in FR-705's own words, and it describes this machine only.
        category: "integration state",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT (SELECT COUNT(*) FROM agent_integrations) \
                         + (SELECT COUNT(*) FROM manager_integrations) \
                         + (SELECT COUNT(*) FROM installed_resources) \
                         + (SELECT COUNT(*) FROM resource_bindings) \
                         + (SELECT COUNT(*) FROM capability_evidence) \
                         + (SELECT COUNT(*) FROM recovery_artifacts) \
                         + (SELECT COUNT(*) FROM migration_states)",
        tables: &[
            "agent_integrations",
            "manager_integrations",
            "installed_resources",
            "resource_bindings",
            "capability_evidence",
            "recovery_artifacts",
            "migration_states",
        ],
    },
    CategorySpec {
        // How far each lane has been read, and what could not be placed when it
        // arrived. Losing it costs a full re-read rather than any knowledge —
        // every merge is idempotent by id — but a re-read is work, and an
        // inventory that hid it would be understating what deletion costs.
        category: "synchronization state",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT (SELECT COUNT(*) FROM sync_cursor) \
                         + (SELECT COUNT(*) FROM sync_meta) \
                         + (SELECT COUNT(*) FROM sync_deferred)",
        tables: &["sync_cursor", "sync_meta", "sync_deferred"],
    },
    CategorySpec {
        // Which side this store treats as authoritative, how far the cutover
        // got, and who claimed which legacy pattern. A fresh store starts this
        // over at `feature_004` and walks the phases again, which is correct
        // and is also why the rows are worth naming as lost.
        category: "authority and migration state",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT (SELECT COUNT(*) FROM authority_mode) \
                         + (SELECT COUNT(*) FROM migration_state) \
                         + (SELECT COUNT(*) FROM legacy_pattern_claims)",
        tables: &["authority_mode", "migration_state", "legacy_pattern_claims"],
    },
    CategorySpec {
        // The durable event and command ordinals. They exist precisely because
        // they must not be re-derived from a spool that drains, so losing them
        // is not neutral: a new store restarts at 1 and can re-issue an
        // identity a delivered event already used.
        category: "durable ordinals",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT (SELECT COUNT(*) FROM session_event_seq) \
                         + (SELECT COUNT(*) FROM command_seq)",
        tables: &["session_event_seq", "command_seq"],
    },
    CategorySpec {
        // What happened to each attempted capture, counted per day. Local-only
        // diagnostic records in FR-705's own enumeration.
        category: "capture dispositions",
        class: DurabilityClass::LocalOnly,
        count_sql: "SELECT COUNT(*) FROM capture_disposition_counts",
        tables: &["capture_disposition_counts"],
    },
];

/// Count every category of data this store holds and state each one's class
/// (T087, FR-705).
///
/// The categories come from [`CATEGORIES`], which is exhaustive over the local
/// schema and is held that way by a test rather than by care. See that
/// constant's doc comment for why a partial inventory is worse than none, and
/// for where each category's class actually comes from.
///
/// Note what is **not** here: no category is [`DurabilityClass::ServerDurable`].
/// That is the honest answer rather than an omission — nothing the local store
/// holds is canonically owned by the local store, which is FR-702 stated as a
/// row count. The variant exists so a caller reporting durability can put the
/// server's side of the ledger next to this one; inventing a local category to
/// fill it would be claiming local authority that FR-702 denies.
pub async fn local_inventory(store: &Store) -> Result<Vec<CategoryDurability>> {
    let mut out = Vec::with_capacity(CATEGORIES.len());
    for spec in CATEGORIES {
        let rows: i64 = sqlx::query_scalar(spec.count_sql)
            .fetch_one(store.pool())
            .await?;
        out.push(CategoryDurability {
            category: spec.category,
            class: spec.class,
            rows,
            tables: spec.tables,
        });
    }
    Ok(out)
}

/// How long a lane may go without a successful pull before its cache is called
/// stale.
///
/// Twenty-four hours, and the number is a judgement that deserves stating
/// rather than a constant to be tuned. The pull worker runs continuously while
/// the daemon is up, so a lane succeeds many times a day in ordinary use. A day
/// without one is therefore not idleness: either the daemon has been down
/// across a working day or the lane has been failing, and both are the point at
/// which "what you are reading may be behind the server" stops being pedantry
/// and becomes something a reader would want to know before trusting a recall.
///
/// Shorter would flag every laptop that was closed overnight. Longer would let
/// a lane that has been broken since yesterday morning present itself as
/// current.
pub const CACHE_STALE_AFTER_HOURS: i64 = 24;

/// What a lane's cache can be, from the reader's point of view.
///
/// The four are distinct answers to distinct questions, and FR-710a exists
/// because collapsing them is the failure: an empty cache presented as an
/// absence of knowledge tells a user they have nothing when what is true is
/// that this device has not fetched anything yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum CacheState {
    /// A pull has succeeded within [`CACHE_STALE_AFTER_HOURS`] and the cache
    /// holds rows.
    Fresh,
    /// A pull has succeeded, but not recently enough to vouch for what is here.
    Stale,
    /// A pull has succeeded and the cache holds nothing. The lane is genuinely
    /// empty on the server, or everything in it has been forgotten — either
    /// way this is a statement about the *content*, and the reader can rely on
    /// it.
    Empty,
    /// No pull has ever succeeded for this lane. The cache holds nothing
    /// **and nothing is known about what it should hold** — the state a store
    /// is in immediately after local loss (FR-704), and the one that must never
    /// be reported as `Empty`.
    NeverRefilled,
}

impl CacheState {
    pub fn as_str(&self) -> &'static str {
        match self {
            CacheState::Fresh => "fresh",
            CacheState::Stale => "stale",
            CacheState::Empty => "empty",
            CacheState::NeverRefilled => "never_refilled",
        }
    }
}

/// One synchronization lane's cache, as this store can honestly describe it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CacheStatus {
    /// The lane's cursor key, verbatim from `sync_cursor`.
    pub namespace: String,
    /// How many rows the cache holds for this lane. Zero for a lane whose key
    /// this build cannot parse — see [`cache_status`].
    pub rows: i64,
    pub last_refilled_at: Option<DateTime<Utc>>,
    pub state: CacheState,
}

/// Report every synchronization lane's cache: how much it holds, when it last
/// refilled, and whether that is good enough to rely on (T087, FR-710a).
///
/// Read from `sync_cursor`, which is where a lane's `last_success_at` already
/// lives. Deliberately not from a second table: staleness is a property of the
/// pull, the pull already records its own success there, and a parallel record
/// of the same fact is a second thing to keep true.
///
/// The classification, in the order it is decided and with the reason for the
/// order:
///
/// 1. no `last_success_at` at all → [`CacheState::NeverRefilled`]. Checked
///    first, and it wins even over a lane that somehow holds rows: those rows
///    did not come from a pull, so nothing about them is vouched for by one.
/// 2. zero rows → [`CacheState::Empty`]. Checked before staleness because the
///    two say different things and only one of them is actionable to a reader:
///    told "stale", a reader assumes there is something here that is merely
///    old, which for an empty cache is exactly the false impression FR-710a
///    forbids. A cache that is both empty and overdue is reported empty, and
///    `last_refilled_at` is on the same record for a caller that wants to say
///    so.
/// 3. `last_success_at` older than [`CACHE_STALE_AFTER_HOURS`] →
///    [`CacheState::Stale`].
/// 4. otherwise → [`CacheState::Fresh`].
///
/// A lane whose key this build cannot parse is still reported, with `rows: 0`.
/// Skipping it would be the same omission [`CATEGORIES`] refuses: a lane
/// written by a newer build is exactly the lane a reader most needs to be told
/// about, and reporting it as absent would say the store has nothing there.
pub async fn cache_status(store: &Store) -> Result<Vec<CacheStatus>> {
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT namespace, last_success_at FROM sync_cursor ORDER BY namespace")
            .fetch_all(store.pool())
            .await?;

    let now = chrono::Utc::now();
    let mut out = Vec::with_capacity(rows.len());
    for (namespace, last_success) in rows {
        let last_refilled_at = last_success
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc));
        let cached = cached_rows_for(store, &namespace).await?;
        let state = if last_refilled_at.is_none() {
            CacheState::NeverRefilled
        } else if cached == 0 {
            CacheState::Empty
        } else if last_refilled_at
            .is_some_and(|t| now - t > chrono::Duration::hours(CACHE_STALE_AFTER_HOURS))
        {
            CacheState::Stale
        } else {
            CacheState::Fresh
        };
        out.push(CacheStatus {
            namespace,
            rows: cached,
            last_refilled_at,
            state,
        });
    }
    Ok(out)
}

/// How many cached rows one lane holds.
///
/// The lane's own slice, not the table's total: two accounts' personal caches
/// live in one table, and reporting one lane's freshness against both counts
/// would call an account's empty cache full because another account's is not.
async fn cached_rows_for(store: &Store, namespace: &str) -> Result<i64> {
    let Some(lane) = crate::cursor::parse(namespace) else {
        // Unparseable, so this build cannot say what the lane holds. Reported
        // as zero rather than guessed at, and never skipped.
        return Ok(0);
    };
    let (sql, key) = match lane {
        SyncNamespace::Project(project) => (
            "SELECT COUNT(*) FROM memories \
             WHERE project_id = ?1 AND local_only = 0 AND deleted_at IS NULL",
            project.to_string(),
        ),
        SyncNamespace::Personal(_, user) => (
            "SELECT COUNT(*) FROM personal_knowledge \
             WHERE owner_user_id = ?1 AND forgotten_at IS NULL",
            user.to_string(),
        ),
        // Team knowledge is bound to one server instance for the whole store
        // (FR-496), so the lane's own instance id is not a filter the table
        // needs — there is nothing else in it to exclude. The bind is still
        // taken so every arm shapes the same, and it is ignored by the
        // statement rather than by the caller.
        SyncNamespace::Team(instance) => (
            "SELECT COUNT(*) FROM team_knowledge WHERE ?1 IS NOT NULL",
            instance.to_string(),
        ),
        SyncNamespace::Patterns(_, owner) => (
            "SELECT COUNT(*) FROM cached_patterns \
             WHERE owner_user_id = ?1 AND forgotten_at IS NULL",
            owner.to_string(),
        ),
    };
    Ok(sqlx::query_scalar(sql)
        .bind(key)
        .fetch_one(store.pool())
        .await?)
}

#[cfg(test)]
mod durability_tests {
    use super::*;
    use crate::Store;

    /// Every table the local schema creates, as the database itself reports it.
    ///
    /// Read from `sqlite_master` rather than from a list in this file, because
    /// a list here would be the same thing [`CATEGORIES`] already is and would
    /// go out of date in the same way at the same time. `schema_migrations` is
    /// the one exclusion: it is the migrator's own bookkeeping about which
    /// scripts ran, not data the store holds, and it is recreated by opening
    /// the database.
    async fn tables_in_schema(store: &Store) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master
              WHERE type = 'table'
                AND name NOT LIKE 'sqlite_%'
                AND name <> 'schema_migrations'
              ORDER BY name",
        )
        .fetch_all(store.pool())
        .await
        .unwrap()
    }

    /// The completeness proof behind [`CATEGORIES`]' claim to be exhaustive.
    ///
    /// This is the test that makes "an inventory that silently omits a category
    /// is worse than none" enforceable rather than aspirational. A migration
    /// that adds a table and does not classify it fails here, at the point the
    /// table is added, instead of producing a durability report that quietly
    /// says nothing about it.
    ///
    /// It runs in both directions on purpose. A table with no category is the
    /// omission; a category naming a table that does not exist is a
    /// classification aimed at nothing, which is how a list stays "complete"
    /// while drifting off the schema it describes.
    #[tokio::test]
    async fn every_local_table_is_accounted_for() {
        let store = Store::open_memory().await.unwrap();
        let schema = tables_in_schema(&store).await;

        let mut claimed: Vec<&str> = CATEGORIES.iter().flat_map(|c| c.tables).copied().collect();
        claimed.sort_unstable();
        claimed.dedup();

        let unclassified: Vec<&String> = schema
            .iter()
            .filter(|t| !claimed.contains(&t.as_str()))
            .collect();
        assert!(
            unclassified.is_empty(),
            "these tables are in the schema and in no durability category: {unclassified:?}"
        );

        let phantom: Vec<&&str> = claimed
            .iter()
            .filter(|t| !schema.iter().any(|s| s == *t))
            .collect();
        assert!(
            phantom.is_empty(),
            "these tables are classified and are not in the schema: {phantom:?}"
        );
    }

    /// A cache survives the knowledge, not the rows — and a queued write
    /// survives neither.
    #[test]
    fn only_the_server_backed_classes_survive_local_loss() {
        assert!(DurabilityClass::ServerDurable.survives_local_loss());
        assert!(
            DurabilityClass::Cache.survives_local_loss(),
            "the rows go; the server's copy is what a refill writes back"
        );
        assert!(
            !DurabilityClass::QueuedForServer.survives_local_loss(),
            "a path to acceptance is not acceptance"
        );
        assert!(!DurabilityClass::LocalOnly.survives_local_loss());
    }

    /// Every category is named on every run, whether or not it holds anything.
    ///
    /// A zero is an answer — "you would lose nothing here" — and dropping the
    /// row would make an empty category indistinguishable from one this
    /// inventory forgot to look at.
    #[tokio::test]
    async fn the_inventory_names_every_category_even_when_empty() {
        let store = Store::open_memory().await.unwrap();
        let report = local_inventory(&store).await.unwrap();
        assert_eq!(report.len(), CATEGORIES.len());
        for spec in CATEGORIES {
            assert!(
                report.iter().any(|r| r.category == spec.category),
                "{} is missing from the inventory",
                spec.category
            );
        }
        assert!(
            report
                .iter()
                .all(|r| r.class != DurabilityClass::ServerDurable),
            "nothing local is canonically owned by the local store (FR-702)"
        );
    }

    /// A populated store reports a real count against the right class for each
    /// of the three classes local data can be in.
    ///
    /// Written against rows inserted directly rather than through the write
    /// paths, so it asserts the inventory's counting statements and nothing
    /// about the callers that would normally produce the rows.
    #[tokio::test]
    async fn a_populated_store_counts_each_class() {
        let store = Store::open_memory().await.unwrap();
        let now = crate::rows::now_text();
        let project = Uuid::now_v7();
        let session = Uuid::now_v7();
        let owner = Uuid::now_v7();

        sqlx::query(
            "INSERT INTO projects
               (id, name, git_common_dir, repository_remote, linked, server_project_id,
                created_at, updated_at, deleted_at)
             VALUES (?1, 'p', '/tmp/g', NULL, 0, NULL, ?2, ?2, NULL)",
        )
        .bind(project.to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions
               (id, project_id, task_id, user_id, agent, branch, commit_sha, worktree_path,
                agent_session_key, previous_session_id, status, started_at, ended_at,
                last_event_at, last_turn_ended_at, daemon_run_id, end_reason, deleted_at)
             VALUES (?1, ?2, NULL, ?3, 'claude-code', 'main', NULL, '/tmp/wt', 'k', NULL,
                     'active', ?4, NULL, ?4, NULL, ?5, NULL, NULL)",
        )
        .bind(session.to_string())
        .bind(project.to_string())
        .bind(Uuid::now_v7().to_string())
        .bind(&now)
        .bind(Uuid::now_v7().to_string())
        .execute(store.pool())
        .await
        .unwrap();

        // One cached memory and one the user marked local-only. They live in
        // one table and must be counted as two categories in two classes,
        // which is FR-706's exclusion expressed as a row count.
        for (content, local_only) in [("shared", 0), ("mine alone", 1)] {
            sqlx::query(
                "INSERT INTO memories
                   (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
                    origin_session_id, local_only, created_at, updated_at)
                 VALUES (?1, ?2, 'fact', 'project', ?3, ?4, 'active', NULL, ?5, ?6, ?7, ?7)",
            )
            .bind(Uuid::now_v7().to_string())
            .bind(project.to_string())
            .bind(project.to_string())
            .bind(content)
            .bind(session.to_string())
            .bind(local_only)
            .bind(&now)
            .execute(store.pool())
            .await
            .unwrap();
        }

        sqlx::query(
            "INSERT INTO cached_patterns
               (pattern_id, owner_user_id, title, problem, root_cause, approach,
                constraints, applicability, trust, content_key, created_at, updated_at,
                forgotten_at, cached_at)
             VALUES (?1, ?2, 't', 'p', 'r', 'a', '[]', '[]', 'sanitized', 'ck', ?3, ?3,
                     NULL, ?3)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(owner.to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO event_spool
               (event_id, session_id, project_id, account_id, session_seq, kind, payload,
                payload_bytes, boundary_class, state, attempts, created_at)
             VALUES (?1, ?2, ?3, ?4, 1, 'tool_use', '{}', 2, 0, 'pending', 0, ?5)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(session.to_string())
        .bind(project.to_string())
        .bind(owner.to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO observations
               (id, session_id, type, occurred_at, branch, commit_sha, path, command,
                exit_code, outcome, summary, details, payload_bytes, truncated, deleted_at)
             VALUES (?1, ?2, 'command_run', ?3, 'main', NULL, NULL, NULL, NULL, NULL,
                     'ran the probe', NULL, 0, 0, NULL)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(session.to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();

        let report = local_inventory(&store).await.unwrap();
        let find = |name: &str| {
            report
                .iter()
                .find(|r| r.category == name)
                .unwrap_or_else(|| panic!("{name} is missing from the inventory"))
        };

        for (category, class, rows) in [
            ("project memory", DurabilityClass::Cache, 1),
            ("cached patterns", DurabilityClass::Cache, 1),
            ("projects", DurabilityClass::Cache, 1),
            ("sessions", DurabilityClass::Cache, 1),
            ("spooled events", DurabilityClass::QueuedForServer, 1),
            ("observations", DurabilityClass::LocalOnly, 1),
            ("local-only memory", DurabilityClass::LocalOnly, 1),
            // Seeded by migration 7's finish hook, so it is never zero on a
            // real store — the inventory would be lying if it read 0 here.
            ("writer identity", DurabilityClass::LocalOnly, 1),
            // Seeded by migration 8's finish hook, for the same reason.
            (
                "authority and migration state",
                DurabilityClass::LocalOnly,
                1,
            ),
        ] {
            let got = find(category);
            assert_eq!(got.class, class, "{category} is in the wrong class");
            assert_eq!(got.rows, rows, "{category} counted {} rows", got.rows);
        }
    }

    /// FR-710a's central distinction: a cache that has never refilled must not
    /// be reported the same way as one that refilled and found nothing.
    ///
    /// Both hold zero rows. Told `Empty`, a reader concludes the account has no
    /// personal knowledge; told `NeverRefilled`, they conclude this device has
    /// not fetched it yet. Only one of those is true after local loss, and the
    /// row count cannot tell them apart — which is why the state is derived
    /// from `last_success_at` first and the count second.
    #[tokio::test]
    async fn an_unfilled_cache_is_not_reported_as_an_empty_one() {
        let store = Store::open_memory().await.unwrap();
        let instance = Uuid::now_v7();
        let never = Uuid::now_v7();
        let refilled = Uuid::now_v7();

        let never_lane = SyncNamespace::Personal(instance, never);
        let empty_lane = SyncNamespace::Personal(instance, refilled);
        crate::cursor::establish(&store, &never_lane).await.unwrap();
        crate::cursor::establish(&store, &empty_lane).await.unwrap();
        crate::cursor::record_success(&store, &empty_lane)
            .await
            .unwrap();

        let report = cache_status(&store).await.unwrap();
        let of = |lane: &SyncNamespace| {
            let key = lane.key();
            report
                .iter()
                .find(|s| s.namespace == key)
                .unwrap_or_else(|| panic!("{key} is missing from the cache report"))
                .clone()
        };

        let never_status = of(&never_lane);
        assert_eq!(never_status.state, CacheState::NeverRefilled);
        assert_eq!(never_status.rows, 0);
        assert!(
            never_status.last_refilled_at.is_none(),
            "there is no success to report"
        );

        let empty_status = of(&empty_lane);
        assert_eq!(empty_status.state, CacheState::Empty);
        assert_eq!(empty_status.rows, 0);
        assert!(
            empty_status.last_refilled_at.is_some(),
            "an empty cache that refilled can say when"
        );

        assert_ne!(
            never_status.state, empty_status.state,
            "an empty cache must not be presented as an absence of knowledge"
        );
    }

    /// A lane that holds rows is `Fresh` while its last success is recent and
    /// `Stale` once it is older than [`CACHE_STALE_AFTER_HOURS`].
    #[tokio::test]
    async fn a_lane_goes_stale_once_its_last_success_is_old_enough() {
        let store = Store::open_memory().await.unwrap();
        let owner = Uuid::now_v7();
        let lane = SyncNamespace::Patterns(Uuid::now_v7(), owner);
        crate::cursor::establish(&store, &lane).await.unwrap();
        crate::cursor::record_success(&store, &lane).await.unwrap();

        let now = crate::rows::now_text();
        sqlx::query(
            "INSERT INTO cached_patterns
               (pattern_id, owner_user_id, title, problem, root_cause, approach,
                constraints, applicability, trust, content_key, created_at, updated_at,
                forgotten_at, cached_at)
             VALUES (?1, ?2, 't', 'p', 'r', 'a', '[]', '[]', 'sanitized', 'ck', ?3, ?3,
                     NULL, ?3)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(owner.to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();

        let fresh = cache_status(&store).await.unwrap();
        assert_eq!(fresh[0].state, CacheState::Fresh);
        assert_eq!(fresh[0].rows, 1);

        // One hour past the threshold, written directly: the alternative is a
        // test that waits a day.
        let overdue =
            (Utc::now() - chrono::Duration::hours(CACHE_STALE_AFTER_HOURS + 1)).to_rfc3339();
        sqlx::query("UPDATE sync_cursor SET last_success_at = ?1 WHERE namespace = ?2")
            .bind(&overdue)
            .bind(lane.key())
            .execute(store.pool())
            .await
            .unwrap();

        let stale = cache_status(&store).await.unwrap();
        assert_eq!(stale[0].state, CacheState::Stale);
        assert_eq!(
            stale[0].rows, 1,
            "stale describes the clock, not the contents"
        );
    }

    /// One lane's freshness is measured against that lane's own slice.
    ///
    /// Two accounts' personal caches share a table, so a count taken over the
    /// whole table would call an account's empty cache full because another
    /// account's is not — and the account with nothing would be told its
    /// knowledge is here and current.
    #[tokio::test]
    async fn one_owners_rows_do_not_make_another_owners_lane_look_full() {
        let store = Store::open_memory().await.unwrap();
        let instance = Uuid::now_v7();
        let has_rows = Uuid::now_v7();
        let has_none = Uuid::now_v7();

        for owner in [has_rows, has_none] {
            let lane = SyncNamespace::Personal(instance, owner);
            crate::cursor::establish(&store, &lane).await.unwrap();
            crate::cursor::record_success(&store, &lane).await.unwrap();
        }

        sqlx::query(
            "INSERT INTO personal_knowledge
               (id, owner_user_id, knowledge_type, content, writer_id, writer_seq, created_at)
             VALUES (?1, ?2, 'fact', 'a note', ?3, 1, ?4)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(has_rows.to_string())
        .bind(Uuid::now_v7().to_string())
        .bind(crate::rows::now_text())
        .execute(store.pool())
        .await
        .unwrap();

        let report = cache_status(&store).await.unwrap();
        let of = |owner: Uuid| {
            let key = SyncNamespace::Personal(instance, owner).key();
            report.iter().find(|s| s.namespace == key).unwrap().clone()
        };
        assert_eq!(of(has_rows).state, CacheState::Fresh);
        assert_eq!(of(has_none).state, CacheState::Empty);
    }
}

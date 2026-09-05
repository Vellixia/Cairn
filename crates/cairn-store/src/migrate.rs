//! Versioned, forward-only migrations with a schema-version guard.
//!
//! The database records its schema version and refuses to open a newer one, so
//! an older `cairnd` can never write against a schema it does not understand.

// `Result` is deliberately not imported unqualified: this file's own `run`,
// `run_to` and `finish` already use the two-argument `std::result::Result`
// with `MigrateError`, and importing the crate's one-argument alias under the
// same name would shadow that. The new repositories below spell it
// `crate::Result` instead.
use crate::{rows, tx, Store, StoreError};
use cairn_core::domain::{
    KnowledgeDomain, KnowledgeRef, PatternRef, Reference, RelationKind, RelationRef,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{Executor, Row, SqlitePool};
use std::collections::HashMap;
use std::str::FromStr;
use uuid::Uuid;

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
    (
        7,
        "collaborative_global_memory",
        include_str!("../migrations/0007_collaborative_global_memory.sql"),
    ),
    (
        8,
        "safe_events",
        include_str!("../migrations/0008_safe_events.sql"),
    ),
    (
        9,
        "pattern_cache",
        include_str!("../migrations/0009_pattern_cache.sql"),
    ),
    (
        10,
        "spool_server_instance",
        include_str!("../migrations/0010_spool_server_instance.sql"),
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
        7 => seed_writer_identity(tx).await,
        8 => seed_authority_mode(tx).await,
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

/// Migration 7, step 4 — this store's one-time opaque writer identity
/// (data-model.md §2.10, migration.md §Local migration step 4).
///
/// Not expressible in the migration's own SQL script: it needs a fresh UUID,
/// which SQLite cannot generate honestly, and unlike a UUIDv7 identifier this
/// value carries no ordering claim to get wrong — but it must be minted
/// exactly once. Runs inside the same transaction as the rest of migration 7,
/// after its script and before `schema_migrations` is written, so an
/// interruption before this step commits still rolls the whole migration
/// back, leaving no `writer_identity` row and the store on schema 6.
async fn seed_writer_identity(tx: &mut sqlx::SqliteConnection) -> Result<(), MigrateError> {
    sqlx::query("INSERT INTO writer_identity (id, writer_id, created_at) VALUES (1, ?1, ?2)")
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Migration 8 — the single `authority_mode` row.
///
/// Every store starts at `feature_004`, including a brand-new one. A fresh
/// install has no legacy knowledge to migrate, so it will pass through the
/// phases quickly, but it still passes through them: seeding it directly at
/// `server_authoritative` would be claiming the migration ran, and the
/// migration is what establishes that the server actually holds what the
/// device does (migration-cutover.md §6, FR-876).
///
/// In Rust rather than SQL because `changed_at` has to be an RFC 3339 string
/// like every other timestamp in this schema, and SQLite's `datetime('now')`
/// writes a different format — a difference nothing would notice until a
/// comparison against a timestamp written by the daemon quietly stopped
/// ordering correctly.
async fn seed_authority_mode(tx: &mut sqlx::SqliteConnection) -> Result<(), MigrateError> {
    sqlx::query("INSERT INTO authority_mode (id, mode, changed_at) VALUES (1, ?1, ?2)")
        .bind("feature_004")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
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

// =============================================================================
// migration_state — per-phase migration progress (migration-cutover.md §6,
// data-model.md §5).
//
// A row exists once a phase has been begun or finished, and its absence means
// exactly what a `Pending` row would mean: nothing has happened yet. Keeping
// that as "no row" rather than seeding six `Pending` rows up front is what
// lets [`first_unfinished`] treat "never touched" and "explicitly pending" the
// same way, which is the only way they can be told apart.
// =============================================================================

/// One step of the migration (migration-cutover.md §6). Declared in the order
/// the migration actually runs them, because [`Phase::all`] and
/// [`first_unfinished`] both depend on that order to decide what "next" means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    Inspect,
    ClaimPatternOwnership,
    Drain,
    VerifyPossession,
    SwitchAuthority,
    Demote,
}

impl Phase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Phase::Inspect => "inspect",
            Phase::ClaimPatternOwnership => "claim_pattern_ownership",
            Phase::Drain => "drain",
            Phase::VerifyPossession => "verify_possession",
            Phase::SwitchAuthority => "switch_authority",
            Phase::Demote => "demote",
        }
    }

    /// Every phase, in the order the migration runs them. This is the
    /// sequence [`first_unfinished`] walks, so declaring it once here is what
    /// keeps that walk and the migration's actual order from drifting apart.
    pub fn all() -> &'static [Phase] {
        &[
            Phase::Inspect,
            Phase::ClaimPatternOwnership,
            Phase::Drain,
            Phase::VerifyPossession,
            Phase::SwitchAuthority,
            Phase::Demote,
        ]
    }

    // Named `from_str` to match this crate's other repository enums, not the
    // `std::str::FromStr` trait (which returns `Result`, not `Option`) — the
    // suppression below is exactly that naming coincidence, not a real problem.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Phase> {
        match s {
            "inspect" => Some(Phase::Inspect),
            "claim_pattern_ownership" => Some(Phase::ClaimPatternOwnership),
            "drain" => Some(Phase::Drain),
            "verify_possession" => Some(Phase::VerifyPossession),
            "switch_authority" => Some(Phase::SwitchAuthority),
            "demote" => Some(Phase::Demote),
            _ => None,
        }
    }
}

/// Where one phase stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseState {
    Pending,
    Running,
    Done,
    Blocked,
}

impl PhaseState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PhaseState::Pending => "pending",
            PhaseState::Running => "running",
            PhaseState::Done => "done",
            PhaseState::Blocked => "blocked",
        }
    }

    // Named `from_str` to match this crate's other repository enums, not the
    // `std::str::FromStr` trait (which returns `Result`, not `Option`) — the
    // suppression below is exactly that naming coincidence, not a real problem.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<PhaseState> {
        match s {
            "pending" => Some(PhaseState::Pending),
            "running" => Some(PhaseState::Running),
            "done" => Some(PhaseState::Done),
            "blocked" => Some(PhaseState::Blocked),
            _ => None,
        }
    }
}

/// One `migration_state` row, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseRow {
    pub phase: Phase,
    pub state: PhaseState,
    pub detail_count: Option<i64>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

fn phase_row_from(row: &SqliteRow) -> crate::Result<PhaseRow> {
    let phase_text: String = row.try_get("phase")?;
    let phase = Phase::from_str(&phase_text)
        .ok_or_else(|| StoreError::Corrupt(format!("migration_state.phase: {phase_text}")))?;
    let state_text: String = row.try_get("state")?;
    let state = PhaseState::from_str(&state_text)
        .ok_or_else(|| StoreError::Corrupt(format!("migration_state.state: {state_text}")))?;
    Ok(PhaseRow {
        phase,
        state,
        detail_count: row.try_get("detail_count")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
    })
}

/// Record that a phase has started running.
///
/// A fresh `started_at` and a cleared `finished_at`/`detail_count` on every
/// call, deliberately: entering a phase again — because a previous attempt
/// was `Blocked` — is a new attempt at that phase, and leaving its last
/// attempt's `finished_at` in place would claim it finished before it had.
pub async fn phase_begin(store: &Store, p: Phase) -> crate::Result<()> {
    sqlx::query(
        "INSERT INTO migration_state (phase, state, detail_count, started_at, finished_at)
         VALUES (?1, ?2, NULL, ?3, NULL)
         ON CONFLICT (phase) DO UPDATE SET
             state = excluded.state,
             detail_count = NULL,
             started_at = excluded.started_at,
             finished_at = NULL",
    )
    .bind(p.as_str())
    .bind(PhaseState::Running.as_str())
    .bind(rows::now_text())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Record how a phase came out.
///
/// `started_at` is left untouched when a row already carries one — `finish`
/// only ever supplies it as a fallback, for a phase whose `begin` was never
/// recorded, so that the real start time [`phase_begin`] wrote is never
/// overwritten by the later time the phase happened to end.
pub async fn phase_finish(
    store: &Store,
    p: Phase,
    s: PhaseState,
    detail_count: i64,
) -> crate::Result<()> {
    let now = rows::now_text();
    sqlx::query(
        "INSERT INTO migration_state (phase, state, detail_count, started_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT (phase) DO UPDATE SET
             state = excluded.state,
             detail_count = excluded.detail_count,
             finished_at = excluded.finished_at",
    )
    .bind(p.as_str())
    .bind(s.as_str())
    .bind(detail_count)
    .bind(now)
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn phase(store: &Store, p: Phase) -> crate::Result<Option<PhaseRow>> {
    let row = sqlx::query(
        "SELECT phase, state, detail_count, started_at, finished_at
         FROM migration_state WHERE phase = ?1",
    )
    .bind(p.as_str())
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(phase_row_from).transpose()
}

pub async fn phases(store: &Store) -> crate::Result<Vec<PhaseRow>> {
    let rows = sqlx::query(
        "SELECT phase, state, detail_count, started_at, finished_at FROM migration_state",
    )
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(phase_row_from).collect()
}

/// The first phase whose state is not `Done`, in [`Phase::all`] order. `None`
/// when every phase is done. This is what makes a re-run resume rather than
/// restart, and it must never skip a not-done phase to return a later one: a
/// phase with no row at all counts as not done, exactly like one recorded
/// `Pending` or `Blocked`.
pub async fn first_unfinished(store: &Store) -> crate::Result<Option<Phase>> {
    let recorded: HashMap<Phase, PhaseState> = phases(store)
        .await?
        .into_iter()
        .map(|row| (row.phase, row.state))
        .collect();
    for candidate in Phase::all() {
        if recorded.get(candidate) != Some(&PhaseState::Done) {
            return Ok(Some(*candidate));
        }
    }
    Ok(None)
}

// =============================================================================
// retained_local — records the server could not accept, which therefore stay
// local (FR-871, data-model.md §5).
// =============================================================================

/// Why a record stayed local instead of moving to the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedReason {
    LocalOnly,
    ServerRefused,
    PossessionIndeterminate,
    OwnerUnclaimed,
}

impl RetainedReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            RetainedReason::LocalOnly => "local_only",
            RetainedReason::ServerRefused => "server_refused",
            RetainedReason::PossessionIndeterminate => "possession_indeterminate",
            RetainedReason::OwnerUnclaimed => "owner_unclaimed",
        }
    }

    // Named `from_str` to match this crate's other repository enums, not the
    // `std::str::FromStr` trait (which returns `Result`, not `Option`) — the
    // suppression below is exactly that naming coincidence, not a real problem.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<RetainedReason> {
        match s {
            "local_only" => Some(RetainedReason::LocalOnly),
            "server_refused" => Some(RetainedReason::ServerRefused),
            "possession_indeterminate" => Some(RetainedReason::PossessionIndeterminate),
            "owner_unclaimed" => Some(RetainedReason::OwnerUnclaimed),
            _ => None,
        }
    }
}

/// Either kind of `Reference`, or the one shape `Reference` cannot name.
///
/// A relation has no id of its own — it *is* the `(from, to, kind)` triple —
/// so it cannot be folded into `cairn_core::domain::Reference` without giving
/// it an identity it does not have. This type exists to hold that third shape
/// alongside the two `Reference` already knows, never to invent a second
/// spelling of either of those two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedRef {
    Knowledge { domain: KnowledgeDomain, id: Uuid },
    Pattern(Uuid),
    Relation(RelationRef),
}

/// The four columns `retained_local`'s CHECK constraint requires, in the one
/// combination legal for a given reference's shape.
struct RetainedColumns {
    ref_kind: &'static str,
    domain: Option<&'static str>,
    knowledge_id: Option<String>,
    relation_key: Option<String>,
}

impl RetainedRef {
    /// The canonical identity string this record dedupes on.
    ///
    /// Delegates to `Reference::reference_key()` for the two shapes it already
    /// names — `knowledge:<domain>:<uuid>` and `pattern:<uuid>` — and prefixes
    /// `RelationRef::relation_key()` for the one shape `Reference` does not
    /// know: `relation:<from>|<to>|<kind>`. Never a second spelling of either.
    pub fn dedupe_key(&self) -> String {
        match self {
            RetainedRef::Knowledge { domain, id } => {
                Reference::Knowledge(KnowledgeRef::new(*domain, *id)).reference_key()
            }
            RetainedRef::Pattern(id) => Reference::Pattern(PatternRef(*id)).reference_key(),
            RetainedRef::Relation(r) => format!("relation:{}", r.relation_key()),
        }
    }

    fn columns(&self) -> RetainedColumns {
        match self {
            RetainedRef::Knowledge { domain, id } => RetainedColumns {
                ref_kind: "knowledge",
                domain: Some(domain.as_str()),
                knowledge_id: Some(id.to_string()),
                relation_key: None,
            },
            RetainedRef::Pattern(id) => RetainedColumns {
                ref_kind: "pattern",
                domain: None,
                knowledge_id: Some(id.to_string()),
                relation_key: None,
            },
            RetainedRef::Relation(r) => RetainedColumns {
                ref_kind: "relation",
                domain: None,
                knowledge_id: None,
                relation_key: Some(r.relation_key()),
            },
        }
    }
}

/// Rebuild a `RelationRef` from the `from|to|kind` text `relation_key` stores.
///
/// The inverse of `RelationRef::relation_key()`, kept next to it in spirit:
/// this is the only place a stored `retained_local.relation_key` is parsed
/// back, so a change to the triple's text shape has exactly one reader to
/// update.
fn parse_relation_key(key: &str) -> crate::Result<RelationRef> {
    let bad = || StoreError::Corrupt(format!("retained_local.relation_key: {key}"));
    let mut parts = key.splitn(3, '|');
    let from_memory_id = parts.next().ok_or_else(bad)?;
    let to_memory_id = parts.next().ok_or_else(bad)?;
    let kind = parts.next().ok_or_else(bad)?;
    Ok(RelationRef {
        from_memory_id: Uuid::parse_str(from_memory_id).map_err(|_| bad())?,
        to_memory_id: Uuid::parse_str(to_memory_id).map_err(|_| bad())?,
        kind: RelationKind::from_str(kind).map_err(|_| bad())?,
    })
}

/// A `retained_local` row, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retained {
    pub reference: RetainedRef,
    pub reason: RetainedReason,
    pub detected_at: String,
}

fn retained_row_from(row: &SqliteRow) -> crate::Result<Retained> {
    let ref_kind: String = row.try_get("ref_kind")?;
    let reference = match ref_kind.as_str() {
        "knowledge" => {
            let domain_text: String = row.try_get("domain")?;
            let domain = KnowledgeDomain::from_str(&domain_text)
                .map_err(|e| StoreError::Corrupt(format!("retained_local.domain: {e}")))?;
            RetainedRef::Knowledge {
                domain,
                id: rows::uuid(row, "knowledge_id")?,
            }
        }
        "pattern" => RetainedRef::Pattern(rows::uuid(row, "knowledge_id")?),
        "relation" => {
            let key: String = row.try_get("relation_key")?;
            RetainedRef::Relation(parse_relation_key(&key)?)
        }
        other => {
            return Err(StoreError::Corrupt(format!(
                "retained_local.ref_kind: {other}"
            )))
        }
    };
    let reason_text: String = row.try_get("reason")?;
    let reason = RetainedReason::from_str(&reason_text)
        .ok_or_else(|| StoreError::Corrupt(format!("retained_local.reason: {reason_text}")))?;
    Ok(Retained {
        reference,
        reason,
        detected_at: row.try_get("detected_at")?,
    })
}

/// Insert-or-ignore keyed on `dedupe_key`. `Ok(true)` when this call recorded
/// it, `Ok(false)` when it was already recorded. `--retry-retained` runs this
/// repeatedly and must never produce two rows for one record.
pub async fn retain(store: &Store, r: RetainedRef, reason: RetainedReason) -> crate::Result<bool> {
    let cols = r.columns();
    let outcome = sqlx::query(
        "INSERT OR IGNORE INTO retained_local
             (ref_kind, domain, knowledge_id, relation_key, reason, detected_at, dedupe_key)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(cols.ref_kind)
    .bind(cols.domain)
    .bind(cols.knowledge_id)
    .bind(cols.relation_key)
    .bind(reason.as_str())
    .bind(rows::now_text())
    .bind(r.dedupe_key())
    .execute(store.pool())
    .await?;
    Ok(outcome.rows_affected() > 0)
}

pub async fn retained(store: &Store) -> crate::Result<Vec<Retained>> {
    let rows = sqlx::query(
        "SELECT ref_kind, domain, knowledge_id, relation_key, reason, detected_at
         FROM retained_local ORDER BY detected_at",
    )
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(retained_row_from).collect()
}

pub async fn retained_by_reason(
    store: &Store,
    reason: RetainedReason,
) -> crate::Result<Vec<Retained>> {
    let rows = sqlx::query(
        "SELECT ref_kind, domain, knowledge_id, relation_key, reason, detected_at
         FROM retained_local WHERE reason = ?1 ORDER BY detected_at",
    )
    .bind(reason.as_str())
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(retained_row_from).collect()
}

/// Removes it — used when `--retry-retained` finally transfers the record.
pub async fn release_retained(store: &Store, r: &RetainedRef) -> crate::Result<bool> {
    let outcome = sqlx::query("DELETE FROM retained_local WHERE dedupe_key = ?1")
        .bind(r.dedupe_key())
        .execute(store.pool())
        .await?;
    Ok(outcome.rows_affected() > 0)
}

pub async fn is_retained(store: &Store, r: &RetainedRef) -> crate::Result<Option<RetainedReason>> {
    let reason: Option<String> =
        sqlx::query_scalar("SELECT reason FROM retained_local WHERE dedupe_key = ?1")
            .bind(r.dedupe_key())
            .fetch_optional(store.pool())
            .await?;
    reason
        .map(|s| {
            RetainedReason::from_str(&s)
                .ok_or_else(|| StoreError::Corrupt(format!("retained_local.reason: {s}")))
        })
        .transpose()
}

// =============================================================================
// legacy_pattern_claims — one-time establishment of who owns a pattern that
// predates ownership (FR-867b).
// =============================================================================

/// A persisted claim: which account owns a local pattern that predates
/// ownership, and the deterministic server identity it claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternClaim {
    pub local_pattern_id: Uuid,
    pub owner_user_id: Uuid,
    pub content_key: String,
    pub pattern_id: Uuid,
    pub claimed_at: String,
}

fn pattern_claim_from_row(row: &SqliteRow) -> crate::Result<PatternClaim> {
    Ok(PatternClaim {
        local_pattern_id: rows::uuid(row, "local_pattern_id")?,
        owner_user_id: rows::uuid(row, "owner_user_id")?,
        content_key: row.try_get("content_key")?,
        pattern_id: rows::uuid(row, "pattern_id")?,
        claimed_at: row.try_get("claimed_at")?,
    })
}

/// What `claim_pattern` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// This call created the claim.
    Claimed(PatternClaim),
    /// The same owner already claimed it; the PERSISTED identity is returned
    /// unchanged. A repeated claim is a no-op, never a second pattern id.
    AlreadyOwned(PatternClaim),
    /// A different account holds the claim. Nothing changes.
    HeldByAnother { owner_user_id: Uuid },
}

/// `content_key` and `pattern_id` are computed by the CALLER from the local
/// pattern's content and the authenticated owner, and are persisted here
/// BEFORE any delivery. This function never recomputes them and never
/// overwrites an existing claim.
///
/// One transaction. The outcome is decided by reading the existing row first
/// — never by relying on a UNIQUE violation message, which would confuse "no
/// claim yet" with "a genuinely malformed request" and cannot tell a same-
/// owner re-claim from a different owner's collision. There is no UPDATE path
/// here for `owner_user_id` or `pattern_id` at all: a credential switch cannot
/// re-key a claim already recorded, because nothing in this function ever
/// writes to those columns of an existing row.
pub async fn claim_pattern(
    store: &Store,
    local_pattern_id: Uuid,
    owner_user_id: Uuid,
    content_key: &str,
    pattern_id: Uuid,
) -> crate::Result<ClaimOutcome> {
    let mut txn = tx::begin(store, "claim_pattern").await?;

    let existing = sqlx::query(
        "SELECT local_pattern_id, owner_user_id, content_key, pattern_id, claimed_at
         FROM legacy_pattern_claims WHERE local_pattern_id = ?1",
    )
    .bind(local_pattern_id.to_string())
    .fetch_optional(&mut *txn)
    .await?;

    let outcome = match existing {
        Some(row) => {
            let claim = pattern_claim_from_row(&row)?;
            if claim.owner_user_id == owner_user_id {
                ClaimOutcome::AlreadyOwned(claim)
            } else {
                ClaimOutcome::HeldByAnother {
                    owner_user_id: claim.owner_user_id,
                }
            }
        }
        None => {
            let claimed_at = rows::now_text();
            sqlx::query(
                "INSERT INTO legacy_pattern_claims
                     (local_pattern_id, owner_user_id, content_key, pattern_id, claimed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .bind(local_pattern_id.to_string())
            .bind(owner_user_id.to_string())
            .bind(content_key)
            .bind(pattern_id.to_string())
            .bind(&claimed_at)
            .execute(&mut *txn)
            .await?;
            ClaimOutcome::Claimed(PatternClaim {
                local_pattern_id,
                owner_user_id,
                content_key: content_key.to_string(),
                pattern_id,
                claimed_at,
            })
        }
    };

    tx::commit(txn, "claim_pattern").await?;
    Ok(outcome)
}

pub async fn pattern_claim(
    store: &Store,
    local_pattern_id: Uuid,
) -> crate::Result<Option<PatternClaim>> {
    let row = sqlx::query(
        "SELECT local_pattern_id, owner_user_id, content_key, pattern_id, claimed_at
         FROM legacy_pattern_claims WHERE local_pattern_id = ?1",
    )
    .bind(local_pattern_id.to_string())
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(pattern_claim_from_row).transpose()
}

pub async fn pattern_claims(store: &Store) -> crate::Result<Vec<PatternClaim>> {
    let rows = sqlx::query(
        "SELECT local_pattern_id, owner_user_id, content_key, pattern_id, claimed_at
         FROM legacy_pattern_claims ORDER BY claimed_at",
    )
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(pattern_claim_from_row).collect()
}

/// Local patterns with no claim row — `owner_unclaimed` candidates.
pub async fn unclaimed_patterns(store: &Store) -> crate::Result<Vec<Uuid>> {
    let rows = sqlx::query(
        "SELECT id FROM reusable_patterns
         WHERE deleted_at IS NULL
           AND id NOT IN (SELECT local_pattern_id FROM legacy_pattern_claims)
         ORDER BY id",
    )
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(|r| rows::uuid(r, "id")).collect()
}

#[cfg(test)]
mod migration_repositories {
    use super::*;
    use std::collections::HashSet;

    async fn store() -> Store {
        Store::open_memory().await.expect("store")
    }

    /// Inserts the minimum valid `reusable_patterns` row `claim_pattern`'s
    /// `REFERENCES` needs, so a claim test does not depend on the pattern
    /// repository this module is not allowed to touch.
    async fn insert_local_pattern(store: &Store, id: Uuid) {
        sqlx::query(
            "INSERT INTO reusable_patterns
                 (id, title, problem, signals, signal_digest, applicability,
                  root_cause, root_cause_digest, approach, constraints, trust,
                  origin_ref, origin_deleted, source_memory_id,
                  sanitization_report, created_at, updated_at)
             VALUES (?1, 'title', 'problem', '[\"a\",\"b\"]', 'digest', '[]',
                     'root cause', 'rc-digest', 'approach', '[]', 'candidate',
                     'origin-ref', 0, NULL, '{}', ?2, ?2)",
        )
        .bind(id.to_string())
        .bind(rows::now_text())
        .execute(store.pool())
        .await
        .expect("insert local pattern");
    }

    #[test]
    fn the_same_uuid_produces_four_distinct_dedupe_keys() {
        // A project, personal and team knowledge reference and a pattern
        // reference can legitimately share a UUID (they live in different
        // tables), so the dedupe key must tell all four apart.
        let id = Uuid::now_v7();
        let project = RetainedRef::Knowledge {
            domain: KnowledgeDomain::Project,
            id,
        };
        let personal = RetainedRef::Knowledge {
            domain: KnowledgeDomain::Personal,
            id,
        };
        let team = RetainedRef::Knowledge {
            domain: KnowledgeDomain::Team,
            id,
        };
        let pattern = RetainedRef::Pattern(id);

        assert_eq!(project.dedupe_key(), format!("knowledge:project:{id}"));
        assert_eq!(personal.dedupe_key(), format!("knowledge:personal:{id}"));
        assert_eq!(team.dedupe_key(), format!("knowledge:team:{id}"));
        assert_eq!(pattern.dedupe_key(), format!("pattern:{id}"));

        let keys: HashSet<String> = [
            project.dedupe_key(),
            personal.dedupe_key(),
            team.dedupe_key(),
            pattern.dedupe_key(),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            keys.len(),
            4,
            "the same uuid must produce four distinct keys"
        );
    }

    #[test]
    fn a_relation_dedupe_key_is_never_confused_with_a_knowledge_one() {
        let relation = RetainedRef::Relation(RelationRef {
            from_memory_id: Uuid::now_v7(),
            to_memory_id: Uuid::now_v7(),
            kind: RelationKind::Supersedes,
        });
        assert!(relation.dedupe_key().starts_with("relation:"));
        assert_eq!(
            relation.dedupe_key(),
            format!(
                "relation:{}",
                match &relation {
                    RetainedRef::Relation(r) => r.relation_key(),
                    _ => unreachable!(),
                }
            )
        );
    }

    #[tokio::test]
    async fn retaining_the_same_record_twice_leaves_exactly_one_row() {
        let store = store().await;
        let r = RetainedRef::Pattern(Uuid::now_v7());

        assert!(
            retain(&store, r, RetainedReason::LocalOnly).await.unwrap(),
            "the first retain must record it"
        );
        assert!(
            !retain(&store, r, RetainedReason::LocalOnly).await.unwrap(),
            "the second retain must find it already recorded"
        );

        let all = retained(&store).await.unwrap();
        assert_eq!(all.len(), 1, "a repeated retain must not duplicate the row");
        assert_eq!(all[0].reference, r);
        assert_eq!(all[0].reason, RetainedReason::LocalOnly);
    }

    #[tokio::test]
    async fn retained_records_round_trip_through_is_retained_and_release() {
        let store = store().await;
        let knowledge = RetainedRef::Knowledge {
            domain: KnowledgeDomain::Team,
            id: Uuid::now_v7(),
        };
        let relation = RetainedRef::Relation(RelationRef {
            from_memory_id: Uuid::now_v7(),
            to_memory_id: Uuid::now_v7(),
            kind: RelationKind::Duplicates,
        });

        retain(&store, knowledge, RetainedReason::PossessionIndeterminate)
            .await
            .unwrap();
        retain(&store, relation, RetainedReason::ServerRefused)
            .await
            .unwrap();

        assert_eq!(
            is_retained(&store, &knowledge).await.unwrap(),
            Some(RetainedReason::PossessionIndeterminate)
        );
        assert_eq!(
            is_retained(&store, &relation).await.unwrap(),
            Some(RetainedReason::ServerRefused)
        );

        assert!(release_retained(&store, &knowledge).await.unwrap());
        assert_eq!(is_retained(&store, &knowledge).await.unwrap(), None);
        // The relation was untouched by releasing the knowledge record.
        assert!(is_retained(&store, &relation).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_repeated_claim_by_the_same_owner_leaves_the_persisted_identity_unchanged() {
        let store = store().await;
        let local_pattern_id = Uuid::now_v7();
        insert_local_pattern(&store, local_pattern_id).await;
        let owner = Uuid::now_v7();
        let pattern_id = Uuid::now_v7();

        let first = claim_pattern(&store, local_pattern_id, owner, "content-key", pattern_id)
            .await
            .unwrap();
        assert!(matches!(first, ClaimOutcome::Claimed(_)));

        // A second claim by the same owner, carrying a DIFFERENT content_key
        // and pattern_id, must still come back as the FIRST claim's identity —
        // a repeated claim is a no-op, never a second pattern id.
        let different_pattern_id = Uuid::now_v7();
        let second = claim_pattern(
            &store,
            local_pattern_id,
            owner,
            "a-different-content-key",
            different_pattern_id,
        )
        .await
        .unwrap();
        match second {
            ClaimOutcome::AlreadyOwned(claim) => {
                assert_eq!(claim.pattern_id, pattern_id, "the pattern id must not move");
                assert_eq!(claim.content_key, "content-key");
            }
            other => panic!("expected AlreadyOwned, got {other:?}"),
        }

        let stored = pattern_claim(&store, local_pattern_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.pattern_id, pattern_id);
        assert_eq!(stored.content_key, "content-key");
    }

    #[tokio::test]
    async fn a_different_owner_claim_is_refused_and_changes_nothing() {
        let store = store().await;
        let local_pattern_id = Uuid::now_v7();
        insert_local_pattern(&store, local_pattern_id).await;
        let owner = Uuid::now_v7();
        let pattern_id = Uuid::now_v7();
        claim_pattern(&store, local_pattern_id, owner, "content-key", pattern_id)
            .await
            .unwrap();

        let intruder = Uuid::now_v7();
        let outcome = claim_pattern(
            &store,
            local_pattern_id,
            intruder,
            "intruder-content-key",
            Uuid::now_v7(),
        )
        .await
        .unwrap();
        match outcome {
            ClaimOutcome::HeldByAnother { owner_user_id } => {
                assert_eq!(owner_user_id, owner, "the refusal must name the true owner")
            }
            other => panic!("expected HeldByAnother, got {other:?}"),
        }

        let stored = pattern_claim(&store, local_pattern_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.owner_user_id, owner,
            "ownership must not move to the intruder"
        );
        assert_eq!(
            stored.pattern_id, pattern_id,
            "the pattern id must not move"
        );
    }

    #[tokio::test]
    async fn first_unfinished_resumes_at_the_first_not_done_phase() {
        let store = store().await;

        // Nothing recorded yet: the very first phase is next, not `None`.
        assert_eq!(
            first_unfinished(&store).await.unwrap(),
            Some(Phase::Inspect)
        );

        phase_finish(&store, Phase::Inspect, PhaseState::Done, 3)
            .await
            .unwrap();
        phase_finish(&store, Phase::ClaimPatternOwnership, PhaseState::Done, 0)
            .await
            .unwrap();
        phase_begin(&store, Phase::Drain).await.unwrap();

        // A later phase merely being untouched must never look more "next"
        // than an earlier phase that is running but not done.
        assert_eq!(first_unfinished(&store).await.unwrap(), Some(Phase::Drain));

        phase_finish(&store, Phase::Drain, PhaseState::Blocked, 1)
            .await
            .unwrap();
        assert_eq!(
            first_unfinished(&store).await.unwrap(),
            Some(Phase::Drain),
            "blocked is not done"
        );

        phase_finish(&store, Phase::Drain, PhaseState::Done, 1)
            .await
            .unwrap();
        assert_eq!(
            first_unfinished(&store).await.unwrap(),
            Some(Phase::VerifyPossession)
        );

        for remaining in [
            Phase::VerifyPossession,
            Phase::SwitchAuthority,
            Phase::Demote,
        ] {
            phase_finish(&store, remaining, PhaseState::Done, 0)
                .await
                .unwrap();
        }
        assert_eq!(
            first_unfinished(&store).await.unwrap(),
            None,
            "every phase done must resume to nothing left to do"
        );
    }
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

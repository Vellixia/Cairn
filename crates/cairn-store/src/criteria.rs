//! Acceptance criteria, blockers, the local revision counter and the change
//! log (`contracts/task-model.md`).
//!
//! The defect this exists to remove is small and expensive: Feature 001's
//! `repo::update_task` writes `acceptance_criteria` as a JSON array of plain
//! strings with no identity, in one statement. Two sessions editing different
//! criteria lose one another's work — on one machine and across sync
//! (research B3). Criteria here have stable ids, so disjoint edits touch
//! disjoint rows and both land by construction.
//!
//! # Three invariants worth stating before the code
//!
//! **A label is never renumbered.** `ordinal` is allocated from the maximum
//! over *every* row of the task including tombstoned ones, so removing AC-2 and
//! adding a criterion yields AC-3 — never a second AC-2 or a second AC-3.
//! Renumbering would silently change what "AC-2" means in a handoff, a
//! checkpoint or a session's memory (FR-481).
//!
//! **The counter never leaves this machine.** `tasks.local_revision` is absent
//! from the sync payload and from the server schema. That is what makes it
//! sound as a concurrency token: an `expected_revision` a caller holds can only
//! have come from a read against this store (FR-488, FR-490, D80). Cross-device
//! state identity is `task_state_digest`, derived from the records that
//! actually converge — never from either counter.
//!
//! **Nothing changes a task's status.** Progress and readiness are computed on
//! read. Completing a task stays an explicit act (FR-487).

use crate::outbox::SyncPolicy;
use crate::{outbox, rows, tx, Result, Store, StoreError};
use cairn_core::domain::{
    new_id, BlockerState, CompletionReadiness, CriterionState, CriterionVerification,
    OutboxOperation, Task, TaskChangeKind, TaskStatus,
};
use cairn_core::tasks::{
    self, criteria_projection, criterion_label, BlockerFacts, CriterionFacts, Progress,
    TaskStateFacts,
};
use cairn_core::wire::codes;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// One stably identified acceptance criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Criterion {
    pub id: Uuid,
    pub task_id: Uuid,
    pub ordinal: i64,
    /// `AC-<ordinal>` at creation, and never rewritten (FR-481).
    pub label: String,
    pub text: String,
    /// The **work** state a session asserts.
    pub state: CriterionState,
    /// What **evidence** establishes. Independent of `state` (FR-482).
    pub verification: CriterionVerification,
    /// Advanced on every change to this criterion. Compared against a caller's
    /// `expected_revision`; local, like the task counter.
    pub revision: i64,
    pub deleted: bool,
}

fn criterion(row: &sqlx::sqlite::SqliteRow) -> Result<Criterion> {
    Ok(Criterion {
        id: rows::uuid(row, "id")?,
        task_id: rows::uuid(row, "task_id")?,
        ordinal: row.try_get("ordinal")?,
        label: row.try_get("label")?,
        text: row.try_get("text")?,
        state: rows::enum_val(row, "state")?,
        verification: rows::enum_val(row, "verification")?,
        revision: row.try_get("revision")?,
        deleted: rows::opt_ts(row, "deleted_at")?.is_some(),
    })
}

/// One blocker. Append-only, with a single `open → cleared` transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub id: Uuid,
    pub task_id: Uuid,
    pub description: String,
    pub state: BlockerState,
    pub opened_by_session: Uuid,
    pub cleared_by_session: Option<Uuid>,
    pub deleted: bool,
}

fn blocker(row: &sqlx::sqlite::SqliteRow) -> Result<Blocker> {
    Ok(Blocker {
        id: rows::uuid(row, "id")?,
        task_id: rows::uuid(row, "task_id")?,
        description: row.try_get("description")?,
        state: rows::enum_val(row, "state")?,
        opened_by_session: rows::uuid(row, "opened_by_session")?,
        cleared_by_session: rows::opt_uuid(row, "cleared_by_session")?,
        deleted: rows::opt_ts(row, "deleted_at")?.is_some(),
    })
}

/// One entry in the append-only local change history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskChange {
    pub id: Uuid,
    pub task_id: Uuid,
    pub local_revision: i64,
    pub kind: TaskChangeKind,
    pub subject_id: Option<Uuid>,
    pub session_id: Uuid,
    pub prior_value: Option<String>,
    pub new_value: Option<String>,
    /// No `expected_revision` was supplied. The write still applied — this is
    /// what makes the overwrite visible rather than silent (FR-490).
    pub blind_write: bool,
}

fn task_change(row: &sqlx::sqlite::SqliteRow) -> Result<TaskChange> {
    Ok(TaskChange {
        id: rows::uuid(row, "id")?,
        task_id: rows::uuid(row, "task_id")?,
        local_revision: row.try_get("local_revision")?,
        kind: rows::enum_val(row, "kind")?,
        subject_id: rows::opt_uuid(row, "subject_id")?,
        session_id: rows::uuid(row, "session_id")?,
        prior_value: row.try_get("prior_value")?,
        new_value: row.try_get("new_value")?,
        blind_write: rows::boolean(row, "blind_write")?,
    })
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

fn refused(code: &'static str, message: impl Into<String>) -> StoreError {
    StoreError::Refused {
        code,
        message: message.into(),
    }
}

// ---------------------------------------------------------------------------
// The one write path
// ---------------------------------------------------------------------------

async fn current_revision(tx: &mut sqlx::SqliteConnection, task_id: Uuid) -> Result<i64> {
    sqlx::query_scalar("SELECT local_revision FROM tasks WHERE id = ?1")
        .bind(task_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("task {task_id}")))
}

/// What a single change records.
struct Change {
    kind: TaskChangeKind,
    subject_id: Option<Uuid>,
    prior_value: Option<String>,
    new_value: Option<String>,
    blind_write: bool,
}

/// Advance the task counter, log the changes, and rewrite the projection —
/// inside the caller's transaction, always together.
///
/// Every mutation in this module ends here. That is the whole reason
/// `local_revision`, `task_changes` and `tasks.acceptance_criteria` can never
/// disagree with the criteria rows: there is one place that writes them and it
/// writes them in one transaction (FR-488, FR-492, I12).
async fn commit_changes(
    tx: &mut sqlx::SqliteConnection,
    task_id: Uuid,
    session: Uuid,
    changes: &[Change],
) -> Result<i64> {
    // Nothing changed, so nothing is recorded and the counter does not move.
    // `local_revision` answers "has anything changed since I read this, here" —
    // advancing it for an update that set every field to the value it already
    // held would make that answer a false positive, and would refuse a caller's
    // still-valid `expected_revision` (FR-488).
    if changes.is_empty() {
        return current_revision(&mut *tx, task_id).await;
    }

    let revision: i64 = sqlx::query_scalar(
        "UPDATE tasks SET local_revision = local_revision + 1, updated_at = ?2
         WHERE id = ?1 RETURNING local_revision",
    )
    .bind(task_id.to_string())
    .bind(rows::now_text())
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| StoreError::NotFound(format!("task {task_id}")))?;

    let now = rows::now_text();
    for c in changes {
        sqlx::query(
            "INSERT INTO task_changes
                (id, task_id, local_revision, kind, subject_id, session_id,
                 prior_value, new_value, blind_write, changed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(new_id().to_string())
        .bind(task_id.to_string())
        .bind(revision)
        .bind(c.kind.as_str())
        .bind(c.subject_id.map(|id| id.to_string()))
        .bind(session.to_string())
        .bind(c.prior_value.as_deref())
        .bind(c.new_value.as_deref())
        .bind(i64::from(c.blind_write))
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    }

    rewrite_projection(&mut *tx, task_id).await?;
    Ok(revision)
}

/// Rewrite `tasks.acceptance_criteria` as the ordinal-ordered list of `text`
/// values.
///
/// The feature's one retained denormalization (D68, FR-492). Five Feature 001
/// readers consume it and none changes; replacing it with a join would break
/// all five at once for no capability gain.
async fn rewrite_projection(tx: &mut sqlx::SqliteConnection, task_id: Uuid) -> Result<()> {
    let facts = criteria_facts_tx(&mut *tx, task_id).await?;
    let projection = criteria_projection(&facts);
    sqlx::query("UPDATE tasks SET acceptance_criteria = ?2 WHERE id = ?1")
        .bind(task_id.to_string())
        .bind(serde_json::to_string(&projection).unwrap_or_else(|_| "[]".into()))
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Enqueue the task's own sync payload after a criterion or blocker change.
///
/// The criteria and blockers themselves are not sent here — that is Phase 10's
/// work. What must go out now is the retained projection, because a peer's
/// `tasks.acceptance_criteria` would otherwise drift from this machine's.
async fn enqueue_task(
    tx: &mut sqlx::SqliteConnection,
    store: &Store,
    policy: SyncPolicy,
    task_id: Uuid,
) -> Result<()> {
    let t = task_tx(&mut *tx, task_id).await?;
    let _ = store;
    // The criteria and blockers themselves, by stable id, so disjoint edits on
    // two machines both land and converge by identity rather than by whichever
    // arrived last (FR-413).
    //
    // Read through the caller's transaction, never the pool: a pool query while
    // this transaction is open deadlocks whenever the pool has fewer free
    // connections than nested readers, which the in-memory store (one
    // connection) makes certain.
    for c in criteria_tx(&mut *tx, task_id).await? {
        outbox::enqueue(
            &mut *tx,
            policy,
            t.project_id,
            cairn_core::domain::OutboxEntityType::TaskCriterion,
            c.id,
            OutboxOperation::Upsert,
            &outbox::criterion_payload(&c),
        )
        .await?;
    }
    for b in blockers_tx(&mut *tx, task_id).await? {
        outbox::enqueue(
            &mut *tx,
            policy,
            t.project_id,
            cairn_core::domain::OutboxEntityType::TaskBlocker,
            b.id,
            OutboxOperation::Upsert,
            &outbox::blocker_payload(&b),
        )
        .await?;
    }

    // `enqueue` returns whether a row was written — false when the project is
    // not linked, which is the ordinary local-only case and not a failure.
    outbox::enqueue(
        &mut *tx,
        policy,
        t.project_id,
        cairn_core::domain::OutboxEntityType::Task,
        task_id,
        OutboxOperation::Upsert,
        &outbox::task_payload(&t),
    )
    .await?;
    Ok(())
}

async fn task_tx(tx: &mut sqlx::SqliteConnection, id: Uuid) -> Result<Task> {
    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?1 AND deleted_at IS NULL")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("task {id}")))?;
    rows::task(&row)
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// Every criterion of a task, tombstoned ones included, in ordinal order.
///
/// Tombstones are returned rather than filtered because the derivations decide
/// for themselves what a removed criterion means, and the digest must agree
/// with them exactly.
pub async fn criteria(store: &Store, task_id: Uuid) -> Result<Vec<Criterion>> {
    let rs = sqlx::query("SELECT * FROM task_criteria WHERE task_id = ?1 ORDER BY ordinal, id")
        .bind(task_id.to_string())
        .fetch_all(store.pool())
        .await?;
    rs.iter().map(criterion).collect()
}

pub async fn criterion_by_id(store: &Store, id: Uuid) -> Result<Criterion> {
    let row = sqlx::query("SELECT * FROM task_criteria WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| {
            refused(
                codes::CRITERION_NOT_FOUND,
                format!("no criterion {id} in this store"),
            )
        })?;
    criterion(&row)
}

pub async fn blockers(store: &Store, task_id: Uuid) -> Result<Vec<Blocker>> {
    let rs = sqlx::query("SELECT * FROM task_blockers WHERE task_id = ?1 ORDER BY id")
        .bind(task_id.to_string())
        .fetch_all(store.pool())
        .await?;
    rs.iter().map(blocker).collect()
}

/// The change log, newest first.
pub async fn history(store: &Store, task_id: Uuid, limit: i64) -> Result<Vec<TaskChange>> {
    let rs = sqlx::query(
        "SELECT * FROM task_changes WHERE task_id = ?1
         ORDER BY local_revision DESC, changed_at DESC LIMIT ?2",
    )
    .bind(task_id.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(task_change).collect()
}

async fn criteria_facts_tx(
    tx: &mut sqlx::SqliteConnection,
    task_id: Uuid,
) -> Result<Vec<CriterionFacts>> {
    let rs = sqlx::query("SELECT * FROM task_criteria WHERE task_id = ?1 ORDER BY ordinal, id")
        .bind(task_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
    rs.iter().map(|r| criterion(r).map(facts_of)).collect()
}

fn facts_of(c: Criterion) -> CriterionFacts {
    CriterionFacts {
        id: c.id,
        ordinal: c.ordinal,
        text: c.text,
        state: c.state,
        verification: c.verification,
        deleted: c.deleted,
    }
}

fn blocker_facts_of(b: &Blocker) -> BlockerFacts {
    BlockerFacts {
        id: b.id,
        state: b.state,
        deleted: b.deleted,
    }
}

/// Everything the cross-device identity is derived from.
pub async fn task_state_facts(store: &Store, task_id: Uuid) -> Result<TaskStateFacts> {
    let t = crate::repo::task(store, task_id).await?;
    Ok(TaskStateFacts {
        title: t.title,
        goal: t.goal,
        status: t.status,
        criteria: criteria(store, task_id)
            .await?
            .into_iter()
            .map(facts_of)
            .collect(),
        blockers: blockers(store, task_id)
            .await?
            .iter()
            .map(blocker_facts_of)
            .collect(),
    })
}

/// The cross-device state identity (FR-493).
pub async fn state_digest(store: &Store, task_id: Uuid) -> Result<String> {
    Ok(tasks::derive_task_state_digest(
        &task_state_facts(store, task_id).await?,
    ))
}

/// Derived progress and readiness — computed on read, never stored, and never
/// changing `tasks.status` (FR-486, FR-487).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readiness {
    pub progress: Progress,
    pub open_blockers: usize,
    pub completion_readiness: CompletionReadiness,
}

pub async fn readiness(store: &Store, task_id: Uuid) -> Result<Readiness> {
    let crits: Vec<CriterionFacts> = criteria(store, task_id)
        .await?
        .into_iter()
        .map(facts_of)
        .collect();
    let blocks = blockers(store, task_id).await?;
    let facts: Vec<BlockerFacts> = blocks.iter().map(blocker_facts_of).collect();
    Ok(Readiness {
        progress: tasks::progress(&crits),
        open_blockers: facts
            .iter()
            .filter(|b| !b.deleted && b.state == BlockerState::Open)
            .count(),
        completion_readiness: tasks::completion_readiness(&crits, &facts),
    })
}

// ---------------------------------------------------------------------------
// Criteria — writes
// ---------------------------------------------------------------------------

/// The next ordinal for a task.
///
/// The maximum over **every** row including tombstoned ones. The unique index
/// is partial (`WHERE deleted_at IS NULL`), so a maximum over live rows alone
/// would reissue AC-3 after AC-3 was removed and never trip the constraint —
/// silently minting a second AC-3 and breaking FR-481.
async fn next_ordinal(tx: &mut sqlx::SqliteConnection, task_id: Uuid) -> Result<i64> {
    let max: Option<i64> =
        sqlx::query_scalar("SELECT MAX(ordinal) FROM task_criteria WHERE task_id = ?1")
            .bind(task_id.to_string())
            .fetch_one(&mut *tx)
            .await?;
    Ok(max.unwrap_or(0) + 1)
}

/// Add a criterion. Its label is fixed at creation and never rewritten.
pub async fn add_criterion(
    store: &Store,
    task_id: Uuid,
    text: &str,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Criterion> {
    let mut tx = tx::begin(store, "add_criterion").await?;
    let id = add_criterion_tx(&mut tx, task_id, text, session).await?;
    enqueue_task(&mut tx, store, policy, task_id).await?;
    tx::commit(tx, "add_criterion").await?;
    criterion_by_id(store, id).await
}

pub(crate) async fn add_criterion_tx(
    tx: &mut sqlx::SqliteConnection,
    task_id: Uuid,
    text: &str,
    session: Uuid,
) -> Result<Uuid> {
    let ordinal = next_ordinal(&mut *tx, task_id).await?;
    let id = new_id();
    let now = rows::now_text();
    sqlx::query(
        "INSERT INTO task_criteria
            (id, task_id, ordinal, label, text, state, verification, revision,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 'unverified', 1, ?6, ?6)",
    )
    .bind(id.to_string())
    .bind(task_id.to_string())
    .bind(ordinal)
    .bind(criterion_label(ordinal))
    .bind(text)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    commit_changes(
        &mut *tx,
        task_id,
        session,
        &[Change {
            kind: TaskChangeKind::CriterionAdded,
            subject_id: Some(id),
            prior_value: None,
            new_value: Some(text.to_string()),
            blind_write: false,
        }],
    )
    .await?;
    Ok(id)
}

/// Read a criterion inside a transaction, refusing what the contract names.
async fn criterion_for_update(tx: &mut sqlx::SqliteConnection, id: Uuid) -> Result<Criterion> {
    let row = sqlx::query("SELECT * FROM task_criteria WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            refused(
                codes::CRITERION_NOT_FOUND,
                format!("no criterion {id} in this store"),
            )
        })?;
    let c = criterion(&row)?;
    if c.deleted {
        return Err(refused(
            codes::CRITERION_NOT_FOUND,
            format!("criterion {} ({}) was removed", c.label, c.id),
        ));
    }
    // `any → waived` has no arrow back out. Waiving is how work leaves scope
    // without anyone pretending it was done, so it is terminal on both axes.
    if c.state == CriterionState::Waived {
        return Err(refused(
            codes::CRITERION_WAIVED,
            format!("criterion {} is waived; waiving is terminal", c.label),
        ));
    }
    Ok(c)
}

/// Check a caller's `expected_revision`, and report whether the write is blind.
///
/// Supplying it is how a caller is protected. Omitting it applies the write and
/// records `blind_write = true`, which is what makes the overwrite visible
/// rather than silent: the prior value, the author, and the fact that no
/// revision was supplied are all recorded (FR-337, FR-490).
fn check_revision(c: &Criterion, expected: Option<i64>) -> Result<bool> {
    match expected {
        None => Ok(true),
        Some(r) if r == c.revision => Ok(false),
        Some(r) => Err(refused(
            codes::REVISION_CONFLICT,
            format!(
                "criterion {} is at revision {} ({}, {}), not {r}",
                c.label,
                c.revision,
                c.state.as_str(),
                c.verification.as_str()
            ),
        )),
    }
}

/// Set a criterion's **work** state.
pub async fn set_criterion_state(
    store: &Store,
    id: Uuid,
    state: CriterionState,
    expected_revision: Option<i64>,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Criterion> {
    let mut tx = tx::begin(store, "set_criterion_state").await?;
    let current = criterion_for_update(&mut tx, id).await?;
    let blind = check_revision(&current, expected_revision)?;

    bump_criterion(&mut tx, id, "state", state.as_str()).await?;
    commit_changes(
        &mut tx,
        current.task_id,
        session,
        &[Change {
            kind: TaskChangeKind::CriterionState,
            subject_id: Some(id),
            prior_value: Some(current.state.as_str().to_string()),
            new_value: Some(state.as_str().to_string()),
            blind_write: blind,
        }],
    )
    .await?;
    enqueue_task(&mut tx, store, policy, current.task_id).await?;
    tx::commit(tx, "set_criterion_state").await?;
    criterion_by_id(store, id).await
}

/// Set a criterion's text, keeping its id and label.
pub async fn set_criterion_text(
    store: &Store,
    id: Uuid,
    text: &str,
    expected_revision: Option<i64>,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Criterion> {
    let mut tx = tx::begin(store, "set_criterion_text").await?;
    let current = criterion_for_update(&mut tx, id).await?;
    let blind = check_revision(&current, expected_revision)?;

    bump_criterion(&mut tx, id, "text", text).await?;
    commit_changes(
        &mut tx,
        current.task_id,
        session,
        &[Change {
            kind: TaskChangeKind::CriterionText,
            subject_id: Some(id),
            prior_value: Some(current.text.clone()),
            new_value: Some(text.to_string()),
            blind_write: blind,
        }],
    )
    .await?;
    enqueue_task(&mut tx, store, policy, current.task_id).await?;
    tx::commit(tx, "set_criterion_text").await?;
    criterion_by_id(store, id).await
}

/// Set a criterion's **verification** axis.
///
/// Not public by accident: the only caller is the daemon's verification path,
/// which admits a local `cairn`-authority verification and nothing else
/// (FR-484). Nothing in the CLI or the wire reaches this directly, which is
/// what keeps readiness from becoming self-certification.
pub async fn set_criterion_verification(
    store: &Store,
    id: Uuid,
    verification: CriterionVerification,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Criterion> {
    let mut tx = tx::begin(store, "set_criterion_verification").await?;
    let current = criterion_for_update(&mut tx, id).await?;

    bump_criterion(&mut tx, id, "verification", verification.as_str()).await?;
    commit_changes(
        &mut tx,
        current.task_id,
        session,
        &[Change {
            kind: TaskChangeKind::CriterionVerification,
            subject_id: Some(id),
            prior_value: Some(current.verification.as_str().to_string()),
            new_value: Some(verification.as_str().to_string()),
            blind_write: false,
        }],
    )
    .await?;
    enqueue_task(&mut tx, store, policy, current.task_id).await?;
    tx::commit(tx, "set_criterion_verification").await?;
    criterion_by_id(store, id).await
}

/// Tombstone a criterion. Ordinals are **not** renumbered.
pub async fn remove_criterion(
    store: &Store,
    id: Uuid,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<()> {
    let mut tx = tx::begin(store, "remove_criterion").await?;
    let current = criterion_for_update(&mut tx, id).await?;
    remove_criterion_tx(&mut tx, &current, session).await?;
    enqueue_task(&mut tx, store, policy, current.task_id).await?;
    tx::commit(tx, "remove_criterion").await
}

async fn remove_criterion_tx(
    tx: &mut sqlx::SqliteConnection,
    current: &Criterion,
    session: Uuid,
) -> Result<()> {
    let now = rows::now_text();
    sqlx::query(
        "UPDATE task_criteria SET deleted_at = ?2, updated_at = ?2, revision = revision + 1
         WHERE id = ?1",
    )
    .bind(current.id.to_string())
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    commit_changes(
        &mut *tx,
        current.task_id,
        session,
        &[Change {
            kind: TaskChangeKind::CriterionRemoved,
            subject_id: Some(current.id),
            prior_value: Some(current.text.clone()),
            new_value: None,
            blind_write: false,
        }],
    )
    .await?;
    Ok(())
}

async fn bump_criterion(
    tx: &mut sqlx::SqliteConnection,
    id: Uuid,
    column: &str,
    value: &str,
) -> Result<()> {
    // The column name is one of three literals chosen here, never caller input.
    let sql = format!(
        "UPDATE task_criteria SET {column} = ?2, revision = revision + 1, updated_at = ?3
         WHERE id = ?1"
    );
    sqlx::query(&sql)
        .bind(id.to_string())
        .bind(value)
        .bind(rows::now_text())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Blockers
// ---------------------------------------------------------------------------

/// Open a blocker.
pub async fn open_blocker(
    store: &Store,
    task_id: Uuid,
    description: &str,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Blocker> {
    let id = new_id();
    let mut tx = tx::begin(store, "open_blocker").await?;
    sqlx::query(
        "INSERT INTO task_blockers
            (id, task_id, description, state, opened_by_session, opened_at)
         VALUES (?1, ?2, ?3, 'open', ?4, ?5)",
    )
    .bind(id.to_string())
    .bind(task_id.to_string())
    .bind(description)
    .bind(session.to_string())
    .bind(rows::now_text())
    .execute(&mut *tx)
    .await?;

    commit_changes(
        &mut tx,
        task_id,
        session,
        &[Change {
            kind: TaskChangeKind::BlockerOpened,
            subject_id: Some(id),
            prior_value: None,
            new_value: Some(description.to_string()),
            blind_write: false,
        }],
    )
    .await?;
    enqueue_task(&mut tx, store, policy, task_id).await?;
    tx::commit(tx, "open_blocker").await?;
    blocker_by_id(store, id).await
}

pub async fn blocker_by_id(store: &Store, id: Uuid) -> Result<Blocker> {
    let row = sqlx::query("SELECT * FROM task_blockers WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| {
            refused(
                codes::BLOCKER_NOT_FOUND,
                format!("no blocker {id} in this store"),
            )
        })?;
    blocker(&row)
}

/// Clear a blocker. The only transition, and terminal: reopening creates a new
/// blocker, so "who said this was blocked and who said it was not" stays
/// answerable (FR-485).
pub async fn clear_blocker(
    store: &Store,
    id: Uuid,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Blocker> {
    let mut tx = tx::begin(store, "clear_blocker").await?;
    let row = sqlx::query("SELECT * FROM task_blockers WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            refused(
                codes::BLOCKER_NOT_FOUND,
                format!("no blocker {id} in this store"),
            )
        })?;
    let current = blocker(&row)?;
    if current.deleted {
        return Err(refused(
            codes::BLOCKER_NOT_FOUND,
            format!("blocker {id} was removed"),
        ));
    }
    if current.state == BlockerState::Cleared {
        return Err(refused(
            codes::BLOCKER_ALREADY_CLEARED,
            format!("blocker {id} is already cleared; reopening creates a new blocker"),
        ));
    }

    sqlx::query(
        "UPDATE task_blockers SET state = 'cleared', cleared_by_session = ?2, cleared_at = ?3
         WHERE id = ?1",
    )
    .bind(id.to_string())
    .bind(session.to_string())
    .bind(rows::now_text())
    .execute(&mut *tx)
    .await?;

    commit_changes(
        &mut tx,
        current.task_id,
        session,
        &[Change {
            kind: TaskChangeKind::BlockerCleared,
            subject_id: Some(id),
            prior_value: Some(current.description.clone()),
            new_value: None,
            blind_write: false,
        }],
    )
    .await?;
    enqueue_task(&mut tx, store, policy, current.task_id).await?;
    tx::commit(tx, "clear_blocker").await?;
    blocker_by_id(store, id).await
}

// ---------------------------------------------------------------------------
// The Feature 001 whole-list form
// ---------------------------------------------------------------------------

/// Update a task, diffing the whole criteria list by text (FR-492, SC-323).
///
/// This is Feature 001's `cairn task update --acceptance-criteria` and it still
/// works. The list is matched against the existing criteria **by text**: an
/// unchanged entry keeps its id and its label, a new entry is added, and a
/// removed entry is tombstoned. Each is logged as its own change, so a Feature
/// 001 caller loses no work and gains identity for free.
///
/// The whole update — row, counter, change log and projection — is one
/// transaction. The Feature 001 code path did a bare `UPDATE` and then opened a
/// *separate* transaction for the outbox, so a crash between them lost the
/// enqueue; that seam is not preserved here.
#[allow(clippy::too_many_arguments)]
pub async fn update_task(
    store: &Store,
    id: Uuid,
    title: Option<&str>,
    goal: Option<&str>,
    criteria_texts: Option<&[String]>,
    status: Option<TaskStatus>,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Task> {
    let mut tx = tx::begin(store, "update_task").await?;
    let current = task_tx(&mut tx, id).await?;
    let mut changes: Vec<Change> = Vec::new();

    if let Some(t) = title.filter(|t| *t != current.title) {
        changes.push(Change {
            kind: TaskChangeKind::TitleChanged,
            subject_id: None,
            prior_value: Some(current.title.clone()),
            new_value: Some(t.to_string()),
            blind_write: false,
        });
    }
    if let Some(g) = goal.filter(|g| *g != current.goal) {
        changes.push(Change {
            kind: TaskChangeKind::GoalChanged,
            subject_id: None,
            prior_value: Some(current.goal.clone()),
            new_value: Some(g.to_string()),
            blind_write: false,
        });
    }
    if let Some(s) = status.filter(|s| *s != current.status) {
        changes.push(Change {
            kind: TaskChangeKind::StatusChanged,
            subject_id: None,
            prior_value: Some(current.status.as_str().to_string()),
            new_value: Some(s.as_str().to_string()),
            blind_write: false,
        });
    }

    sqlx::query(
        "UPDATE tasks SET title = ?2, goal = ?3, status = ?4, updated_at = ?5 WHERE id = ?1",
    )
    .bind(id.to_string())
    .bind(title.unwrap_or(&current.title))
    .bind(goal.unwrap_or(&current.goal))
    .bind(status.unwrap_or(current.status).as_str())
    .bind(rows::now_text())
    .execute(&mut *tx)
    .await?;

    // The criteria diff, by text. Done before `commit_changes` so the whole
    // update advances the counter once and lands one coherent projection.
    if let Some(wanted) = criteria_texts {
        diff_criteria(&mut tx, id, wanted, session, &mut changes).await?;
    }

    commit_changes(&mut tx, id, session, &changes).await?;
    enqueue_task(&mut tx, store, policy, id).await?;
    tx::commit(tx, "update_task").await?;
    crate::repo::task(store, id).await
}

/// Match `wanted` against the live criteria by text.
///
/// Duplicate texts are matched positionally within their text group, so a list
/// holding the same sentence twice keeps both ids rather than tombstoning one
/// and adding it back with a new label.
async fn diff_criteria(
    tx: &mut sqlx::SqliteConnection,
    task_id: Uuid,
    wanted: &[String],
    session: Uuid,
    changes: &mut Vec<Change>,
) -> Result<()> {
    let rs = sqlx::query(
        "SELECT * FROM task_criteria WHERE task_id = ?1 AND deleted_at IS NULL
         ORDER BY ordinal, id",
    )
    .bind(task_id.to_string())
    .fetch_all(&mut *tx)
    .await?;
    let live: Vec<Criterion> = rs.iter().map(criterion).collect::<Result<_>>()?;

    let mut claimed = vec![false; live.len()];
    let mut unmatched: Vec<&String> = Vec::new();

    for text in wanted {
        match live
            .iter()
            .enumerate()
            .find(|(i, c)| !claimed[*i] && c.text == **text)
        {
            Some((i, _)) => claimed[i] = true,
            None => unmatched.push(text),
        }
    }

    for (i, c) in live.iter().enumerate() {
        if !claimed[i] {
            let now = rows::now_text();
            sqlx::query(
                "UPDATE task_criteria
                 SET deleted_at = ?2, updated_at = ?2, revision = revision + 1
                 WHERE id = ?1",
            )
            .bind(c.id.to_string())
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            changes.push(Change {
                kind: TaskChangeKind::CriterionRemoved,
                subject_id: Some(c.id),
                prior_value: Some(c.text.clone()),
                new_value: None,
                blind_write: false,
            });
        }
    }

    for text in unmatched {
        let ordinal = next_ordinal(&mut *tx, task_id).await?;
        let id = new_id();
        let now = rows::now_text();
        sqlx::query(
            "INSERT INTO task_criteria
                (id, task_id, ordinal, label, text, state, verification, revision,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', 'unverified', 1, ?6, ?6)",
        )
        .bind(id.to_string())
        .bind(task_id.to_string())
        .bind(ordinal)
        .bind(criterion_label(ordinal))
        .bind(text)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        changes.push(Change {
            kind: TaskChangeKind::CriterionAdded,
            subject_id: Some(id),
            prior_value: None,
            new_value: Some(text.clone()),
            blind_write: false,
        });
    }
    let _ = session;
    Ok(())
}

/// Seed criteria rows for a task at creation, inside its transaction.
///
/// A task must have stable criterion identity from the moment it exists. If it
/// did not, the first `criterion add` would rewrite the projection from a set
/// of rows that did not include the criteria the task was created with — and
/// silently drop them.
pub(crate) async fn seed_criteria_tx(
    tx: &mut sqlx::SqliteConnection,
    task_id: Uuid,
    texts: &[String],
    session: Uuid,
) -> Result<()> {
    for text in texts {
        add_criterion_tx(&mut *tx, task_id, text, session).await?;
    }
    Ok(())
}

/// Rebuild `tasks.acceptance_criteria` from the criteria rows.
///
/// The equality `rebuild_criteria_projection` asserts (I11, SC-324): the
/// retained denormalization is a cache of the rows and never an independent
/// truth.
pub async fn rebuild_criteria_projection(store: &Store, task_id: Uuid) -> Result<Vec<String>> {
    let mut tx = tx::begin(store, "rebuild_criteria_projection").await?;
    rewrite_projection(&mut tx, task_id).await?;
    tx::commit(tx, "rebuild_criteria_projection").await?;
    Ok(crate::repo::task(store, task_id).await?.acceptance_criteria)
}

/// The bounded snapshot a session records at bind (FR-489).
pub async fn bind_snapshot(store: &Store, task_id: Uuid) -> Result<String> {
    let facts = task_state_facts(store, task_id).await?;
    serde_json::to_string(&facts).map_err(|e| StoreError::Corrupt(e.to_string()))
}

/// One difference between the state a session bound at and the state now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Divergence {
    /// `AC-2`, or the blocker's description.
    pub subject: String,
    pub what: String,
    /// `this_machine` where **this change** is the one a session on this store
    /// made, and `another_machine` otherwise.
    pub origin: String,
}

/// The latest value this store recorded itself setting, per changed thing.
///
/// Keyed by `(kind, subject_id)` because one criterion has a state, a
/// verification and a text, each changed independently. The last local write
/// wins, which is why the rows are read in `local_revision` order.
type LocalWrites = std::collections::BTreeMap<(TaskChangeKind, Option<Uuid>), Option<String>>;

/// Whether **this** change is one this machine made.
///
/// Attribution compares the value the change arrived at against the last value
/// a local session set for the same thing. Asking only "has this criterion ever
/// been touched here" — which is what this did before — reports a criterion
/// created locally and then changed on another machine as local, which is the
/// one report a divergence must never get wrong: the agent is deciding whether
/// to trust its own understanding of the change.
fn made_here(writes: &LocalWrites, kind: TaskChangeKind, subject: Option<Uuid>, now: &str) -> bool {
    matches!(writes.get(&(kind, subject)), Some(Some(v)) if v == now)
}

/// Whether a one-time event — a creation, a removal, a blocker opening or
/// clearing — happened here.
///
/// These carry no competing value: an id is created once, by one machine, and
/// removal and clearing are terminal. Presence of the local record is the whole
/// answer, and comparing values would wrongly disown a criterion created here
/// whose text another machine later edited.
fn happened_here(writes: &LocalWrites, kind: TaskChangeKind, subject: Option<Uuid>) -> bool {
    writes.contains_key(&(kind, subject))
}

/// Derive what changed since a session bound, by **diffing the snapshot against
/// the current records** — never by reading `task_changes` (D80).
///
/// The change log is local. A log-based report would describe only this
/// machine's edits and would silently omit a criterion another machine changed,
/// even though the criterion row itself had arrived. Diffing converged records
/// reports both origins with no new payload and no log synchronization.
pub async fn divergence(store: &Store, task_id: Uuid, snapshot: &str) -> Result<Vec<Divergence>> {
    let before: TaskStateFacts = match serde_json::from_str(snapshot) {
        Ok(f) => f,
        // A snapshot this store cannot read is not a divergence report; it is
        // an absence of one. Saying "nothing changed" would be a lie, so the
        // caller is told there is no comparison rather than a false empty.
        Err(e) => return Err(StoreError::Corrupt(format!("task snapshot: {e}"))),
    };
    let now = task_state_facts(store, task_id).await?;

    // What a session on this store recorded itself setting, latest write last.
    //
    // The log is read for **attribution only** — never to decide *what*
    // changed, which stays a diff of converged records (D80). A change this
    // machine never made leaves no row here and is reported as another
    // machine's, which is exactly right for an imported one.
    let mut writes = LocalWrites::new();
    let rows = sqlx::query(
        "SELECT kind, subject_id, new_value FROM task_changes
         WHERE task_id = ?1 ORDER BY local_revision ASC",
    )
    .bind(task_id.to_string())
    .fetch_all(store.pool())
    .await?;
    for row in &rows {
        let kind: TaskChangeKind = rows::enum_val(row, "kind")?;
        let subject = rows::opt_uuid(row, "subject_id")?;
        writes.insert((kind, subject), row.try_get("new_value")?);
    }

    let attribute = |made: bool| {
        if made {
            "this_machine".to_string()
        } else {
            "another_machine".to_string()
        }
    };

    let mut out = Vec::new();
    if before.title != now.title {
        out.push(Divergence {
            subject: "title".into(),
            what: format!("{:?} → {:?}", before.title, now.title),
            origin: attribute(made_here(
                &writes,
                TaskChangeKind::TitleChanged,
                None,
                &now.title,
            )),
        });
    }
    if before.goal != now.goal {
        out.push(Divergence {
            subject: "goal".into(),
            what: "the goal changed".into(),
            origin: attribute(made_here(
                &writes,
                TaskChangeKind::GoalChanged,
                None,
                &now.goal,
            )),
        });
    }
    if before.status != now.status {
        out.push(Divergence {
            subject: "status".into(),
            what: format!("{} → {}", before.status.as_str(), now.status.as_str()),
            origin: attribute(made_here(
                &writes,
                TaskChangeKind::StatusChanged,
                None,
                now.status.as_str(),
            )),
        });
    }

    for c in now.criteria.iter().filter(|c| !c.deleted) {
        let label = criterion_label(c.ordinal);
        match before.criteria.iter().find(|b| b.id == c.id) {
            None => out.push(Divergence {
                subject: label.clone(),
                what: format!("criterion added — {:?}", c.text),
                origin: attribute(happened_here(
                    &writes,
                    TaskChangeKind::CriterionAdded,
                    Some(c.id),
                )),
            }),
            Some(b) if b.deleted => out.push(Divergence {
                subject: label.clone(),
                what: format!("criterion added — {:?}", c.text),
                origin: attribute(happened_here(
                    &writes,
                    TaskChangeKind::CriterionAdded,
                    Some(c.id),
                )),
            }),
            Some(b) => {
                if b.state != c.state {
                    out.push(Divergence {
                        subject: label.clone(),
                        what: format!("{} → {}", b.state.as_str(), c.state.as_str()),
                        origin: attribute(made_here(
                            &writes,
                            TaskChangeKind::CriterionState,
                            Some(c.id),
                            c.state.as_str(),
                        )),
                    });
                }
                if b.verification != c.verification {
                    out.push(Divergence {
                        subject: label.clone(),
                        what: format!("{} → {}", b.verification.as_str(), c.verification.as_str()),
                        origin: attribute(made_here(
                            &writes,
                            TaskChangeKind::CriterionVerification,
                            Some(c.id),
                            c.verification.as_str(),
                        )),
                    });
                }
                if b.text != c.text {
                    // A criterion's text can have been set by the write that
                    // created it, so both kinds answer for it.
                    out.push(Divergence {
                        subject: label.clone(),
                        what: "the text changed".into(),
                        origin: attribute(
                            made_here(&writes, TaskChangeKind::CriterionText, Some(c.id), &c.text)
                                || made_here(
                                    &writes,
                                    TaskChangeKind::CriterionAdded,
                                    Some(c.id),
                                    &c.text,
                                ),
                        ),
                    });
                }
            }
        }
    }

    for b in before.criteria.iter().filter(|b| !b.deleted) {
        let gone = now
            .criteria
            .iter()
            .find(|c| c.id == b.id)
            .map(|c| c.deleted)
            .unwrap_or(true);
        if gone {
            out.push(Divergence {
                subject: criterion_label(b.ordinal),
                what: format!("criterion removed — {:?}", b.text),
                origin: attribute(happened_here(
                    &writes,
                    TaskChangeKind::CriterionRemoved,
                    Some(b.id),
                )),
            });
        }
    }

    let blocker_rows = blockers(store, task_id).await?;
    for blk in blocker_rows.iter().filter(|b| !b.deleted) {
        match before.blockers.iter().find(|b| b.id == blk.id) {
            None => out.push(Divergence {
                subject: blk.description.clone(),
                what: format!("blocker {}", blk.state.as_str()),
                origin: attribute(happened_here(
                    &writes,
                    TaskChangeKind::BlockerOpened,
                    Some(blk.id),
                )),
            }),
            Some(b) if b.state != blk.state => out.push(Divergence {
                subject: blk.description.clone(),
                what: format!("blocker {} → {}", b.state.as_str(), blk.state.as_str()),
                origin: attribute(happened_here(
                    &writes,
                    // The only state change a blocker has.
                    TaskChangeKind::BlockerCleared,
                    Some(blk.id),
                )),
            }),
            Some(_) => {}
        }
    }

    Ok(out)
}

/// Criteria currently `verified`, oldest task first, capped.
///
/// What the bounded verification pass re-checks. A criterion that nothing
/// re-examines would stay `verified` — and its task `ready` — indefinitely after
/// the evidence it rests on moved, which is the one thing readiness must never
/// do (`contracts/task-model.md` §Completion readiness).
pub async fn verified_criteria_for_project(
    store: &Store,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<Uuid>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT c.id FROM task_criteria c
           JOIN tasks t ON t.id = c.task_id
          WHERE t.project_id = ?1
            AND c.deleted_at IS NULL
            AND t.deleted_at IS NULL
            AND c.verification = 'verified'
          ORDER BY c.updated_at ASC
          LIMIT ?2",
    )
    .bind(project_id.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await?;
    Ok(rows.iter().filter_map(|s| Uuid::from_str(s).ok()).collect())
}

/// Upsert a criterion that arrived from a peer, by its stable id.
///
/// Two machines that changed *different* criteria offline both land, because
/// different criteria are different rows and cannot collide. The local
/// `revision` is **not** taken from the payload: it is a local concurrency
/// token, and an arriving row must not be able to move it (D80).
///
/// The projection and the task's own counter are rebuilt afterwards from the
/// converged rows, so `tasks.acceptance_criteria` and the derived digest agree
/// with what actually arrived.
#[allow(clippy::too_many_arguments)]
pub async fn import_criterion(
    store: &Store,
    id: Uuid,
    task_id: Uuid,
    ordinal: i64,
    label: &str,
    text: &str,
    state: CriterionState,
    verification: CriterionVerification,
    deleted: bool,
) -> Result<()> {
    let mut tx = tx::begin(store, "import_criterion").await?;
    let now = rows::now_text();
    sqlx::query(
        "INSERT INTO task_criteria
            (id, task_id, ordinal, label, text, state, verification, revision,
             created_at, updated_at, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8, ?9)
         ON CONFLICT (id) DO UPDATE SET
             ordinal = excluded.ordinal, label = excluded.label, text = excluded.text,
             state = excluded.state, verification = excluded.verification,
             updated_at = excluded.updated_at, deleted_at = excluded.deleted_at",
    )
    .bind(id.to_string())
    .bind(task_id.to_string())
    .bind(ordinal)
    .bind(label)
    .bind(text)
    .bind(state.as_str())
    .bind(verification.as_str())
    .bind(&now)
    .bind(deleted.then(|| now.clone()))
    .execute(&mut *tx)
    .await?;

    rewrite_projection(&mut tx, task_id).await?;
    tx::commit(tx, "import_criterion").await
}

/// Upsert a blocker that arrived from a peer.
#[allow(clippy::too_many_arguments)]
pub async fn import_blocker(
    store: &Store,
    id: Uuid,
    task_id: Uuid,
    description: &str,
    state: BlockerState,
    opened_by_session: Uuid,
    cleared_by_session: Option<Uuid>,
    deleted: bool,
) -> Result<()> {
    let mut tx = tx::begin(store, "import_blocker").await?;
    let now = rows::now_text();
    sqlx::query(
        "INSERT INTO task_blockers
            (id, task_id, description, state, opened_by_session, opened_at,
             cleared_by_session, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (id) DO UPDATE SET
             state = excluded.state,
             cleared_by_session = excluded.cleared_by_session,
             deleted_at = excluded.deleted_at",
    )
    .bind(id.to_string())
    .bind(task_id.to_string())
    .bind(description)
    .bind(state.as_str())
    .bind(opened_by_session.to_string())
    .bind(&now)
    .bind(cleared_by_session.map(|s| s.to_string()))
    .bind(deleted.then(|| now.clone()))
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "import_blocker").await
}

/// `criteria`, read through a caller's transaction.
async fn criteria_tx(tx: &mut sqlx::SqliteConnection, task_id: Uuid) -> Result<Vec<Criterion>> {
    let rs = sqlx::query("SELECT * FROM task_criteria WHERE task_id = ?1 ORDER BY ordinal, id")
        .bind(task_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
    rs.iter().map(criterion).collect()
}

/// `blockers`, read through a caller's transaction.
async fn blockers_tx(tx: &mut sqlx::SqliteConnection, task_id: Uuid) -> Result<Vec<Blocker>> {
    let rs = sqlx::query("SELECT * FROM task_blockers WHERE task_id = ?1 ORDER BY id")
        .bind(task_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
    rs.iter().map(blocker).collect()
}

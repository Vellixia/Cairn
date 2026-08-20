//! Continuity checkpoints (`contracts/continuity-context.md` Part 1).
//!
//! A checkpoint is **derived work state plus the assumptions it was taken
//! under**. It is not a summary of conversation, and it does not depend on any
//! provider's compression quality (FR-421). Nothing is carried in the
//! conversation, so nothing degrades with each compaction pass — which is the
//! whole point.
//!
//! It anchors to the handoff Cairn already derives at that boundary (D55,
//! FR-423) and adds only what the handoff cannot: the assumption set that makes
//! staleness detectable, the bounded per-path fingerprints that make a change
//! detectable *whoever made it*, and the restore counters.
//!
//! Checkpoints are **local** and append-only. They never synchronize (FR-503),
//! and each `context_compacting` writes a new one rather than rewriting the
//! last, so ten cycles leave ten records and the tenth restoration reads the
//! tenth.

use crate::{rows, tx, Result, Store, StoreError};
use cairn_core::continuity::{Assumptions, PathFingerprint};
use cairn_core::domain::{new_id, CheckpointTrigger};
use cairn_core::tasks::{BlockerFacts, CriterionFacts};
use sqlx::Row;
use uuid::Uuid;

/// One recorded checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub id: Uuid,
    pub session_id: Uuid,
    pub project_id: Uuid,
    /// Everything the handoff already derives, by reference.
    pub handoff_id: Uuid,
    pub trigger: CheckpointTrigger,
    pub assumed: Assumptions,
    pub criteria_snapshot: Vec<CriterionFacts>,
    pub open_blockers: Vec<BlockerFacts>,
    pub pinned_constraints: Vec<Uuid>,
    pub next_action: String,
    pub restore_count: i64,
}

/// What a new checkpoint records.
pub struct NewCheckpoint<'a> {
    pub session_id: Uuid,
    pub project_id: Uuid,
    pub handoff_id: Uuid,
    pub trigger: CheckpointTrigger,
    pub assumed: &'a Assumptions,
    pub criteria_snapshot: &'a [CriterionFacts],
    pub open_blockers: &'a [BlockerFacts],
    pub pinned_constraints: &'a [Uuid],
    pub next_action: &'a str,
}

fn checkpoint(row: &sqlx::sqlite::SqliteRow) -> Result<Checkpoint> {
    let paths: Vec<PathFingerprint> = rows::json_field(row, "path_fingerprints")?;
    Ok(Checkpoint {
        id: rows::uuid(row, "id")?,
        session_id: rows::uuid(row, "session_id")?,
        project_id: rows::uuid(row, "project_id")?,
        handoff_id: rows::uuid(row, "handoff_id")?,
        trigger: rows::enum_val(row, "trigger")?,
        assumed: Assumptions {
            branch: row.try_get("assumed_branch")?,
            commit: row.try_get("assumed_commit")?,
            task_id: rows::opt_uuid(row, "assumed_task_id")?,
            task_state_digest: row.try_get("assumed_task_state_digest")?,
            path_fingerprints: paths,
        },
        criteria_snapshot: rows::json_field(row, "criteria_snapshot")?,
        open_blockers: rows::json_field(row, "open_blockers")?,
        pinned_constraints: rows::json_field(row, "pinned_constraints")?,
        next_action: row.try_get("next_action")?,
        restore_count: row.try_get("restore_count")?,
    })
}

/// The cap on relevant paths (FR-424).
///
/// Thirty-two bounded reads on a *restoration* is a different order of work from
/// the repository scan FR-471 forbids, and it does not run on an ordinary
/// session open.
pub const RELEVANT_PATHS_MAX: usize = 32;

/// Write a checkpoint.
///
/// Append-only: a new boundary writes a new record. Nothing is copied forward
/// from the previous checkpoint, because every field is derived from the store
/// again — which is what makes the tenth cycle as complete as the first
/// (FR-428, SC-310).
pub async fn record(store: &Store, c: NewCheckpoint<'_>) -> Result<Checkpoint> {
    let id = new_id();
    let mut paths = c.assumed.path_fingerprints.clone();
    paths.truncate(RELEVANT_PATHS_MAX);
    let relevant: Vec<&String> = paths.iter().map(|p| &p.path).collect();

    let mut t = tx::begin(store, "record_checkpoint").await?;
    sqlx::query(
        "INSERT INTO continuity_checkpoints
            (id, session_id, project_id, handoff_id, trigger, assumed_branch,
             assumed_commit, assumed_task_id, assumed_task_state_digest,
             relevant_paths, path_fingerprints, criteria_snapshot, open_blockers,
             pinned_constraints, next_action, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )
    .bind(id.to_string())
    .bind(c.session_id.to_string())
    .bind(c.project_id.to_string())
    .bind(c.handoff_id.to_string())
    .bind(c.trigger.as_str())
    .bind(&c.assumed.branch)
    .bind(c.assumed.commit.as_deref())
    .bind(c.assumed.task_id.map(|t| t.to_string()))
    .bind(c.assumed.task_state_digest.as_deref())
    .bind(json(&relevant))
    .bind(json(&paths))
    .bind(json(&c.criteria_snapshot))
    .bind(json(&c.open_blockers))
    .bind(json(&c.pinned_constraints))
    .bind(c.next_action)
    .bind(rows::now_text())
    .execute(&mut *t)
    .await?;
    tx::commit(t, "record_checkpoint").await?;
    by_id(store, id).await
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

pub async fn by_id(store: &Store, id: Uuid) -> Result<Checkpoint> {
    let row = sqlx::query("SELECT * FROM continuity_checkpoints WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::Refused {
            code: cairn_core::wire::codes::CHECKPOINT_NOT_FOUND,
            message: format!("no checkpoint {id}"),
        })?;
    checkpoint(&row)
}

/// The newest checkpoint for a session.
pub async fn latest(store: &Store, session_id: Uuid) -> Result<Option<Checkpoint>> {
    let row = sqlx::query(
        "SELECT * FROM continuity_checkpoints
          WHERE session_id = ?1 AND deleted_at IS NULL
          ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(session_id.to_string())
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(checkpoint).transpose()
}

/// The newest checkpoint on a branch, for a session resuming with no checkpoint
/// of its own.
pub async fn latest_on_branch(
    store: &Store,
    project_id: Uuid,
    branch: &str,
) -> Result<Option<Checkpoint>> {
    let row = sqlx::query(
        "SELECT c.* FROM continuity_checkpoints c
           JOIN sessions s ON s.id = c.session_id
          WHERE c.project_id = ?1 AND c.assumed_branch = ?2 AND c.deleted_at IS NULL
          ORDER BY c.created_at DESC, c.id DESC LIMIT 1",
    )
    .bind(project_id.to_string())
    .bind(branch)
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(checkpoint).transpose()
}

/// Count a restoration.
///
/// Evidence for the ten-compaction test: each cycle increments, so the count is
/// a fact about what happened rather than an assertion about what should have.
pub async fn mark_restored(store: &Store, id: Uuid) -> Result<i64> {
    let mut t = tx::begin(store, "mark_restored").await?;
    let count: i64 = sqlx::query_scalar(
        "UPDATE continuity_checkpoints
            SET restore_count = restore_count + 1, restored_at = ?2
          WHERE id = ?1 RETURNING restore_count",
    )
    .bind(id.to_string())
    .bind(rows::now_text())
    .fetch_optional(&mut *t)
    .await?
    .ok_or_else(|| StoreError::Refused {
        code: cairn_core::wire::codes::CHECKPOINT_NOT_FOUND,
        message: format!("no checkpoint {id}"),
    })?;
    tx::commit(t, "mark_restored").await?;
    Ok(count)
}

/// How many checkpoints a session has. Append-only, so this only grows.
pub async fn count_for_session(store: &Store, session_id: Uuid) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(*) FROM continuity_checkpoints WHERE session_id = ?1 AND deleted_at IS NULL",
    )
    .bind(session_id.to_string())
    .fetch_one(store.pool())
    .await?)
}

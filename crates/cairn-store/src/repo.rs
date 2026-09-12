//! Repositories over the local database.
//!
//! Deletion is scoped, never cascading into durable knowledge: removing a
//! session clears the session and its observations but leaves the memories and
//! handoffs it produced readable, with their origin marked deleted (FR-052).

use crate::outbox::{self, SyncPolicy};
use crate::rows;
use crate::tx;
use crate::{Result, Store, StoreError};
use cairn_core::domain::*;
use cairn_core::knowledge as knowledge_core;
use cairn_core::knowledge::ProposalOutcome;
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// The single implicit local user, created on first use.
///
/// Reconciled to the authenticated server user when a project is linked.
pub async fn ensure_local_user(store: &Store) -> Result<Uuid> {
    if let Some(row) = sqlx::query("SELECT id FROM users ORDER BY created_at LIMIT 1")
        .fetch_optional(store.pool())
        .await?
    {
        return rows::uuid(&row, "id");
    }
    let id = new_id();
    let name = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "local".into());
    sqlx::query(
        "INSERT INTO users (id, email, display_name, created_at) VALUES (?1, NULL, ?2, ?3)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(rows::now_text())
    .execute(store.pool())
    .await?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// Projects
// ---------------------------------------------------------------------------

/// Register the local project for a repository instance, or return the
/// existing one. Idempotent (FR-002).
pub async fn ensure_project(
    store: &Store,
    git_common_dir: &str,
    name: &str,
    remote: Option<&str>,
) -> Result<Project> {
    if let Some(p) = project_by_common_dir(store, git_common_dir).await? {
        // Keep the remote hint fresh; it is only ever a discovery aid.
        if p.repository_remote.as_deref() != remote {
            sqlx::query(
                "UPDATE projects SET repository_remote = ?1, updated_at = ?2 WHERE id = ?3",
            )
            .bind(remote)
            .bind(rows::now_text())
            .bind(p.id.to_string())
            .execute(store.pool())
            .await?;
            return project(store, p.id).await;
        }
        return Ok(p);
    }
    // Insert-or-ignore, then read back. Check-then-insert races when two hooks
    // touch a repository for the first time at the same moment, and SQLite
    // answers that race with a UNIQUE violation rather than a retry (H1).
    let now = rows::now_text();
    let id = new_id();
    sqlx::query(
        "INSERT INTO projects
            (id, name, git_common_dir, repository_remote, linked, server_project_id,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, NULL, ?5, ?5)
         ON CONFLICT (git_common_dir) DO NOTHING",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(git_common_dir)
    .bind(remote)
    .bind(&now)
    .execute(store.pool())
    .await?;

    // Whoever won the race owns the row; both callers get the same project.
    project_by_common_dir(store, git_common_dir)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("project for {git_common_dir}")))
}

pub async fn project(store: &Store, id: Uuid) -> Result<Project> {
    let row = sqlx::query("SELECT * FROM projects WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("project {id}")))?;
    rows::project(&row)
}

pub async fn project_by_common_dir(store: &Store, dir: &str) -> Result<Option<Project>> {
    let row =
        sqlx::query("SELECT * FROM projects WHERE git_common_dir = ?1 AND deleted_at IS NULL")
            .bind(dir)
            .fetch_optional(store.pool())
            .await?;
    row.as_ref().map(rows::project).transpose()
}

pub async fn list_projects(store: &Store) -> Result<Vec<Project>> {
    let rs = sqlx::query("SELECT * FROM projects WHERE deleted_at IS NULL ORDER BY created_at")
        .fetch_all(store.pool())
        .await?;
    rs.iter().map(rows::project).collect()
}

/// Opt a project into server sync (FR-053, FR-064).
pub async fn link_project(store: &Store, id: Uuid, server_project_id: Uuid) -> Result<Project> {
    sqlx::query(
        "UPDATE projects SET linked = 1, server_project_id = ?1, updated_at = ?2 WHERE id = ?3",
    )
    .bind(server_project_id.to_string())
    .bind(rows::now_text())
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    sqlx::query("INSERT OR IGNORE INTO sync_meta (project_id) VALUES (?1)")
        .bind(id.to_string())
        .execute(store.pool())
        .await?;
    project(store, id).await
}

/// Opt back out. Queued items for the project are dropped: an unlinked project
/// sends nothing (FR-053).
pub async fn unlink_project(store: &Store, id: Uuid) -> Result<Project> {
    let mut tx = tx::begin(store, "unlink_project").await?;
    sqlx::query("UPDATE projects SET linked = 0, updated_at = ?1 WHERE id = ?2")
        .bind(rows::now_text())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM outbox WHERE project_id = ?1 AND state IN ('pending', 'failed')")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    tx::commit(tx, "unlink_project").await?;
    project(store, id).await
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/// Create a task, with a criterion row for each acceptance criterion.
///
/// `session` attributes the seeded criteria in the change log. Seeding happens
/// in this transaction rather than afterwards because a task whose projection
/// held criteria that no row backed would lose them the first time anything
/// rewrote the projection (FR-481, FR-492).
pub async fn create_task(
    store: &Store,
    project_id: Uuid,
    title: &str,
    goal: &str,
    criteria: &[String],
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Task> {
    let id = new_id();
    let now = rows::now_text();
    let mut tx = tx::begin(store, "create_task").await?;
    sqlx::query(
        "INSERT INTO tasks (id, project_id, title, goal, acceptance_criteria, status,
                            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'todo', ?6, ?6)",
    )
    .bind(id.to_string())
    .bind(project_id.to_string())
    .bind(title)
    .bind(goal)
    .bind(serde_json::to_string(criteria).unwrap_or_else(|_| "[]".into()))
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    crate::criteria::seed_criteria_tx(&mut tx, id, criteria, session).await?;

    // Same transaction as the change it describes (D9) — and the criteria as
    // well as the task.
    //
    // Enqueuing only the task left every criterion given at creation time
    // unqueued and therefore unshared, so a task created with `--criterion`
    // arrived on a peer as a shell with none of them: zero criteria, and a
    // completion readiness of `ready` because nothing was outstanding. Only
    // criteria added *after* creation ever crossed. `enqueue_task` queues the
    // criteria, the blockers and the task together, which is the same set every
    // later criterion change already queues.
    crate::criteria::enqueue_task(&mut tx, store, policy, id).await?;
    tx::commit(tx, "create_task").await?;
    task(store, id).await
}

pub async fn task(store: &Store, id: Uuid) -> Result<Task> {
    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?1 AND deleted_at IS NULL")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("task {id}")))?;
    rows::task(&row)
}

pub async fn list_tasks(
    store: &Store,
    project_id: Uuid,
    status: Option<TaskStatus>,
) -> Result<Vec<Task>> {
    let rs =
        match status {
            Some(s) => sqlx::query(
                "SELECT * FROM tasks WHERE project_id = ?1 AND status = ?2 AND deleted_at IS NULL
                 ORDER BY created_at DESC",
            )
            .bind(project_id.to_string())
            .bind(s.as_str())
            .fetch_all(store.pool())
            .await?,
            None => {
                sqlx::query(
                    "SELECT * FROM tasks WHERE project_id = ?1 AND deleted_at IS NULL
                 ORDER BY created_at DESC",
                )
                .bind(project_id.to_string())
                .fetch_all(store.pool())
                .await?
            }
        };
    rs.iter().map(rows::task).collect()
}

/// Update whichever fields were supplied. Status transitions are unrestricted
/// and simply recorded (FR-037).
///
/// Feature 003 moved the body to [`crate::criteria::update_task`], which does
/// the same job plus the criteria diff, the local counter and the change log —
/// all in one transaction. This stays as the name Feature 001's callers use.
/// There is deliberately no second write path: a task edit that bypassed the
/// counter would leave `expected_revision` unsound.
#[allow(clippy::too_many_arguments)]
pub async fn update_task(
    store: &Store,
    id: Uuid,
    title: Option<&str>,
    goal: Option<&str>,
    criteria: Option<&[String]>,
    status: Option<TaskStatus>,
    session: Uuid,
    policy: SyncPolicy,
) -> Result<Task> {
    crate::criteria::update_task(store, id, title, goal, criteria, status, session, policy).await
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

pub struct StartSession<'a> {
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub agent: &'a str,
    pub agent_session_key: &'a str,
    pub branch: &'a str,
    pub commit_sha: Option<&'a str>,
    pub worktree_path: &'a str,
    pub task_id: Option<Uuid>,
    pub daemon_run_id: Uuid,
    pub policy: SyncPolicy,
}

/// Start a session, or return the existing one for this agent session.
///
/// Idempotency is per `agent_session_key`, never per worktree — two agents in
/// one checkout get two distinct sessions (FR-010). A session previously
/// reconciled to `interrupted` resumes here (FR-009, D16).
pub async fn start_session(store: &Store, input: StartSession<'_>) -> Result<Session> {
    if let Some(existing) = session_by_key(store, input.project_id, input.agent_session_key).await?
    {
        if existing.status != SessionStatus::Active {
            return resume_session(store, existing.id, input.daemon_run_id).await;
        }
        return Ok(existing);
    }

    let previous =
        previous_session_for(store, input.project_id, input.task_id, input.branch).await?;
    let id = new_id();
    let now = rows::now_text();
    let mut tx = tx::begin(store, "start_session").await?;
    sqlx::query(
        "INSERT INTO sessions
            (id, project_id, task_id, user_id, agent, branch, commit_sha, worktree_path,
             agent_session_key, previous_session_id, status, started_at, ended_at,
             last_event_at, last_turn_ended_at, daemon_run_id, end_reason)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, NULL, ?11, NULL, ?12, NULL)
         ON CONFLICT DO NOTHING",
    )
    .bind(id.to_string())
    .bind(input.project_id.to_string())
    .bind(input.task_id.map(|t| t.to_string()))
    .bind(input.user_id.to_string())
    .bind(input.agent)
    .bind(input.branch)
    .bind(input.commit_sha)
    .bind(input.worktree_path)
    .bind(input.agent_session_key)
    .bind(previous.map(|p| p.to_string()))
    .bind(&now)
    .bind(input.daemon_run_id.to_string())
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "start_session").await?;

    // The read above and this insert are check-then-act, and `sessions` has a
    // unique index on `(project_id, agent_session_key)`. Two callers starting
    // the same session at once both see nothing and both insert; one wins.
    //
    // Starting a session that already exists is not an error — it is the
    // idempotency this function's key contract promises — so the loser reads
    // the winner's session rather than failing the caller's write.
    if let Some(existing) = session_by_key(store, input.project_id, input.agent_session_key).await?
    {
        if existing.id != id {
            return Ok(existing);
        }
    }

    // A session that starts already bound to a task records the state it bound
    // at, exactly as `bind_task` does — otherwise a session started with
    // `--task` could never be told the task advanced under it (FR-489).
    if let Some(task_id) = input.task_id {
        if let Ok(snapshot) = crate::criteria::bind_snapshot(store, task_id).await {
            sqlx::query("UPDATE sessions SET task_snapshot_at_bind = ?2 WHERE id = ?1")
                .bind(id.to_string())
                .bind(snapshot)
                .execute(store.pool())
                .await?;
        }
    }

    let created = session(store, id).await?;
    enqueue_session(store, input.policy, &created).await?;
    Ok(created)
}

/// Queue a session's minimal provenance for a linked project (FR-055).
async fn enqueue_session(store: &Store, policy: SyncPolicy, s: &Session) -> Result<()> {
    let mut tx = tx::begin(store, "enqueue_session").await?;
    outbox::enqueue(
        &mut *tx,
        policy,
        s.project_id,
        OutboxEntityType::Session,
        s.id,
        OutboxOperation::Upsert,
        &outbox::session_payload(s),
    )
    .await?;
    tx::commit(tx, "enqueue_session").await?;
    Ok(())
}

/// The most recently *ended* qualifying session: same task, else same branch,
/// with `id` breaking ties. A single link, not a graph (FR-008).
async fn previous_session_for(
    store: &Store,
    project_id: Uuid,
    task_id: Option<Uuid>,
    branch: &str,
) -> Result<Option<Uuid>> {
    if let Some(task_id) = task_id {
        let row = sqlx::query(
            "SELECT id FROM sessions
             WHERE project_id = ?1 AND task_id = ?2 AND status != 'active' AND deleted_at IS NULL
             ORDER BY ended_at DESC, id DESC LIMIT 1",
        )
        .bind(project_id.to_string())
        .bind(task_id.to_string())
        .fetch_optional(store.pool())
        .await?;
        if let Some(r) = row {
            return rows::uuid(&r, "id").map(Some);
        }
    }
    let row = sqlx::query(
        "SELECT id FROM sessions
         WHERE project_id = ?1 AND branch = ?2 AND status != 'active' AND deleted_at IS NULL
         ORDER BY ended_at DESC, id DESC LIMIT 1",
    )
    .bind(project_id.to_string())
    .bind(branch)
    .fetch_optional(store.pool())
    .await?;
    row.map(|r| rows::uuid(&r, "id")).transpose()
}

pub async fn session(store: &Store, id: Uuid) -> Result<Session> {
    let row = sqlx::query("SELECT * FROM sessions WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("session {id}")))?;
    rows::session(&row)
}

pub async fn session_by_key(store: &Store, project_id: Uuid, key: &str) -> Result<Option<Session>> {
    let row = sqlx::query(
        "SELECT * FROM sessions
         WHERE project_id = ?1 AND agent_session_key = ?2 AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .bind(key)
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(rows::session).transpose()
}

pub async fn list_sessions(store: &Store, project_id: Uuid) -> Result<Vec<Session>> {
    let rs = sqlx::query(
        "SELECT * FROM sessions WHERE project_id = ?1 AND deleted_at IS NULL
         ORDER BY started_at DESC",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::session).collect()
}

/// Active sessions in one worktree. There may legitimately be more than one.
pub async fn active_sessions_in_worktree(
    store: &Store,
    project_id: Uuid,
    worktree: &str,
) -> Result<Vec<Session>> {
    let rs = sqlx::query(
        "SELECT * FROM sessions
         WHERE project_id = ?1 AND worktree_path = ?2 AND status = 'active'
           AND deleted_at IS NULL
         ORDER BY started_at DESC",
    )
    .bind(project_id.to_string())
    .bind(worktree)
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::session).collect()
}

/// Record that something happened, without changing status.
pub async fn touch_session(store: &Store, id: Uuid) -> Result<()> {
    tx::retry("touch_session", || {
        Box::pin(async {
            sqlx::query("UPDATE sessions SET last_event_at = ?1 WHERE id = ?2")
                .bind(rows::now_text())
                .bind(id.to_string())
                .execute(store.pool())
                .await?;
            Ok(())
        })
    })
    .await?;
    Ok(())
}

/// `Stop`: the agent finished a turn. The session stays `active` (FR-032, D16).
pub async fn turn_checkpoint(store: &Store, id: Uuid) -> Result<Session> {
    let now = rows::now_text();
    sqlx::query("UPDATE sessions SET last_turn_ended_at = ?1, last_event_at = ?1 WHERE id = ?2")
        .bind(&now)
        .bind(id.to_string())
        .execute(store.pool())
        .await?;
    session(store, id).await
}

/// Bind a session to a task, recording the task state it bound at.
///
/// `task_snapshot_at_bind` is what makes a divergence report possible without
/// synchronizing the local change log: on refresh the snapshot is diffed
/// against the current records, so a criterion another machine changed shows up
/// as readily as one this machine changed (FR-489, D80).
pub async fn bind_task(store: &Store, id: Uuid, task_id: Uuid) -> Result<Session> {
    let snapshot = crate::criteria::bind_snapshot(store, task_id).await?;
    sqlx::query(
        "UPDATE sessions SET task_id = ?1, task_snapshot_at_bind = ?4, last_event_at = ?2
         WHERE id = ?3",
    )
    .bind(task_id.to_string())
    .bind(rows::now_text())
    .bind(id.to_string())
    .bind(snapshot)
    .execute(store.pool())
    .await?;
    session(store, id).await
}

pub async fn end_session(
    store: &Store,
    id: Uuid,
    status: SessionStatus,
    reason: Option<&str>,
    policy: SyncPolicy,
) -> Result<Session> {
    let now = rows::now_text();
    sqlx::query(
        "UPDATE sessions SET status = ?1, ended_at = ?2, last_event_at = ?2, end_reason = ?3
         WHERE id = ?4",
    )
    .bind(status.as_str())
    .bind(&now)
    .bind(reason)
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    let ended = session(store, id).await?;
    enqueue_session(store, policy, &ended).await?;
    Ok(ended)
}

/// A later event arrived for a session reconciled at daemon start: resume it
/// under the current run and clear `ended_at` (D16).
pub async fn resume_session(store: &Store, id: Uuid, daemon_run_id: Uuid) -> Result<Session> {
    sqlx::query(
        "UPDATE sessions
         SET status = 'active', ended_at = NULL, end_reason = NULL,
             daemon_run_id = ?1, last_event_at = ?2
         WHERE id = ?3",
    )
    .bind(daemon_run_id.to_string())
    .bind(rows::now_text())
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    session(store, id).await
}

/// Sessions still `active` from a previous daemon run.
///
/// Cairn has no liveness signal, so daemon start — not a heartbeat — is the
/// deterministic boundary that reconciles them (FR-009, D16).
/// Active sessions whose last event predates `cutoff`.
///
/// A session only ends when something tells it to. When that signal is lost —
/// an agent killed, a `SessionEnd` hook arriving for a key the daemon never
/// saw — the row stays `active` forever and makes every later session
/// ambiguous. Going quiet is the evidence that nobody is driving it.
pub async fn sessions_idle_since(
    store: &Store,
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<Session>> {
    let rs = sqlx::query(
        "SELECT * FROM sessions
         WHERE status = 'active' AND deleted_at IS NULL AND last_event_at < ?1",
    )
    .bind(cutoff.to_rfc3339())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::session).collect()
}

/// Active sessions a newer active session in the same worktree has overtaken,
/// silent since `cutoff`.
///
/// An agent that is restarted rather than exited leaves its session `active`:
/// no `SessionEnd` arrives, and Cairn has no liveness signal to notice. Every
/// such session makes the worktree ambiguous, which is what blocks the briefing
/// an agent asks for before it knows its own session key — the very thing the
/// idle reaper exists to prevent.
///
/// The generous idle timeout is right for a session on its own: a developer
/// reading and thinking must never be mistaken for one who left. It is far too
/// generous once a *newer* session is running in the same worktree, because
/// then the older one is not thinking — something replaced it. That is the
/// evidence this query looks for, and it is why the caller may apply a much
/// shorter silence to these than to a session working alone.
///
/// Only sessions with a newer sibling are returned, so the newest in a worktree
/// is never reaped by this rule and a worktree can never be emptied by it. Ties
/// on `started_at` break on id, so two sessions stamped the same instant still
/// leave exactly one survivor.
pub async fn superseded_sessions_idle_since(
    store: &Store,
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<Session>> {
    let rs = sqlx::query(
        "SELECT s.* FROM sessions s
          WHERE s.status = 'active' AND s.deleted_at IS NULL AND s.last_event_at < ?1
            AND EXISTS (
                SELECT 1 FROM sessions n
                 WHERE n.project_id = s.project_id
                   AND n.worktree_path = s.worktree_path
                   AND n.status = 'active' AND n.deleted_at IS NULL
                   AND (n.started_at > s.started_at
                        OR (n.started_at = s.started_at AND n.id > s.id))
            )
          ORDER BY s.started_at ASC",
    )
    .bind(cutoff.to_rfc3339())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::session).collect()
}

pub async fn sessions_from_previous_runs(store: &Store, current_run: Uuid) -> Result<Vec<Session>> {
    let rs = sqlx::query(
        "SELECT * FROM sessions
         WHERE status = 'active' AND daemon_run_id != ?1 AND deleted_at IS NULL",
    )
    .bind(current_run.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::session).collect()
}

// ---------------------------------------------------------------------------
// Observations
// ---------------------------------------------------------------------------

pub struct NewObservation<'a> {
    pub session_id: Uuid,
    pub kind: ObservationType,
    pub branch: &'a str,
    pub commit_sha: Option<&'a str>,
    pub path: Option<&'a str>,
    pub command: Option<&'a str>,
    pub exit_code: Option<i64>,
    pub outcome: Option<&'a str>,
    pub summary: &'a str,
    pub details: Option<&'a serde_json::Value>,
    pub payload_bytes: i64,
    pub truncated: bool,
}

pub async fn insert_observation(store: &Store, o: NewObservation<'_>) -> Result<Observation> {
    let id = new_id();
    tx::retry("insert_observation", || {
        Box::pin(async {
            sqlx::query(
                "INSERT INTO observations
                    (id, session_id, type, occurred_at, branch, commit_sha, path, command, exit_code,
                     outcome, summary, details, payload_bytes, truncated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )
            .bind(id.to_string())
            .bind(o.session_id.to_string())
            .bind(o.kind.as_str())
            .bind(rows::now_text())
            .bind(o.branch)
            .bind(o.commit_sha)
            .bind(o.path)
            .bind(o.command)
            .bind(o.exit_code)
            .bind(o.outcome)
            .bind(o.summary)
            .bind(o.details.map(|d| d.to_string()))
            .bind(o.payload_bytes)
            .bind(o.truncated as i64)
            .execute(store.pool())
            .await?;
            Ok(())
        })
    })
    .await?;
    observation(store, id).await
}

pub async fn observation(store: &Store, id: Uuid) -> Result<Observation> {
    let row = sqlx::query("SELECT * FROM observations WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("observation {id}")))?;
    rows::observation(&row)
}

pub async fn observations_for_session(store: &Store, session_id: Uuid) -> Result<Vec<Observation>> {
    let rs = sqlx::query(
        "SELECT * FROM observations WHERE session_id = ?1 AND deleted_at IS NULL
         ORDER BY occurred_at, id",
    )
    .bind(session_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::observation).collect()
}

pub async fn count_observations(store: &Store, project_id: Uuid) -> Result<i64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM observations o
         JOIN sessions s ON s.id = o.session_id
         WHERE s.project_id = ?1 AND o.deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_one(store.pool())
    .await?;
    Ok(n)
}

// ---------------------------------------------------------------------------
// Memories
// ---------------------------------------------------------------------------

pub struct NewMemory<'a> {
    pub project_id: Uuid,
    pub kind: MemoryType,
    pub scope: MemoryScope,
    pub scope_key: &'a str,
    pub content: &'a str,
    pub origin_session_id: Uuid,
    pub local_only: bool,
    /// Zero or more. Never fabricated to satisfy the schema (FR-019).
    pub evidence: &'a [Uuid],
    /// The subject this proposal concerns, as the caller proposed it. Optional:
    /// a free-form memory is fully valid, searchable, briefable and syncable,
    /// and behaves exactly as it does in Feature 001 (FR-313).
    ///
    /// Normalized here rather than by the caller, so every writer gets the same
    /// treatment. An unrepresentable key does **not** reject the memory: it is
    /// stored free-form and the reason is reported (FR-312).
    pub topic_key: Option<&'a str>,
    /// The comparable value it asserts. Accepted only alongside a topic key.
    pub value_key: Option<&'a str>,
    /// A within-bucket ranking hint, and nothing more (FR-308).
    pub importance: Importance,
}

impl<'a> NewMemory<'a> {
    /// A proposal with no subject identity — Feature 001's shape.
    #[allow(clippy::too_many_arguments)]
    pub fn free_form(
        project_id: Uuid,
        kind: MemoryType,
        scope: MemoryScope,
        scope_key: &'a str,
        content: &'a str,
        origin_session_id: Uuid,
        local_only: bool,
        evidence: &'a [Uuid],
    ) -> Self {
        Self {
            project_id,
            kind,
            scope,
            scope_key,
            content,
            origin_session_id,
            local_only,
            evidence,
            topic_key: None,
            value_key: None,
            importance: Importance::Normal,
        }
    }
}

/// What creating a proposal turned out to mean for its subject.
///
/// Returned alongside the memory so the writer learns what Cairn decided — and,
/// where Cairn deliberately did **not** decide, which member it matched, so the
/// party that can read both statements can settle it explicitly (FR-327).
#[derive(Debug, Clone)]
pub struct CreateOutcome {
    pub memory: Memory,
    pub reconciliation: ProposalOutcome,
    /// The relation this write actually recorded, carried out of the
    /// transaction that recorded it rather than re-derived from the outcome.
    /// A report built from a lookup table can drift from the database; this
    /// cannot (`contracts/mcp-tools.md` §`reconciliation`).
    pub relation_recorded: Option<cairn_core::RelationKind>,
    /// The matched member's value key, taken from the members the classifier
    /// already held, so the report costs no extra query.
    pub matched_value_key: Option<String>,
    /// The subject this proposal joined, **after** normalization — so the
    /// report names the key the store actually indexed, not the raw one the
    /// caller typed. `None` for a free-form memory, and for a key that failed
    /// normalization (FR-312).
    pub subject: Option<String>,
    /// Notes for the `ok: true` envelope: `invalid_topic_key`,
    /// `corroborating_member`, `reconciliation_deferred` (FR-312, FR-474).
    pub notes: Vec<&'static str>,
}

impl CreateOutcome {
    /// The wire form of what reconciliation decided.
    pub fn report(&self) -> cairn_core::wire::ReconciliationReport {
        cairn_core::wire::ReconciliationReport::build(
            &self.reconciliation,
            self.subject.as_deref(),
            self.relation_recorded,
            self.matched_value_key.clone(),
        )
    }
}

pub async fn create_memory(store: &Store, m: NewMemory<'_>, policy: SyncPolicy) -> Result<Memory> {
    Ok(
        create_memory_reconciled(store, m, policy, DEFAULT_RECONCILE_MEMBERS_MAX)
            .await?
            .memory,
    )
}

/// The per-write bound, when the caller has no configuration to hand.
///
/// Mirrors `CairnConfig::reconcile_members_max`; `bounds.rs` (T140) asserts the
/// two agree.
pub const DEFAULT_RECONCILE_MEMBERS_MAX: usize = 64;

/// Re-queue a memory whose syncable fields changed after it was first sent.
///
/// The outbox holds a **snapshot**, taken when the row was queued. A memory
/// verified after it synced would otherwise keep its peers on the payload from
/// before the check forever — and `remote_cairn` and `remote_attested`, the
/// whole point of transmitting an authority, would be unreachable in practice
/// (FR-368, SC-329).
///
/// The idempotency key covers the payload, so re-queuing an unchanged memory is
/// a no-op rather than a duplicate delivery.
pub async fn enqueue_memory_upsert(store: &Store, memory_id: Uuid) -> Result<bool> {
    let m = memory(store, memory_id).await?;
    // The boundary is the memory's own flag, checked wherever a payload is
    // built (FR-051).
    if m.local_only {
        return Ok(false);
    }
    let mut tx = tx::begin(store, "enqueue_memory_upsert").await?;
    let policy = outbox::policy_for_project_tx(&mut tx, m.project_id).await?;
    let payload = outbox::memory_payload_for(&mut tx, &m).await?;
    let queued = outbox::enqueue(
        &mut *tx,
        policy,
        m.project_id,
        OutboxEntityType::Memory,
        m.id,
        OutboxOperation::Upsert,
        &payload,
    )
    .await?;
    tx::commit(tx, "enqueue_memory_upsert").await?;
    Ok(queued)
}

/// A proposal that arrived from another machine (FR-411).
#[derive(Debug, Clone)]
pub struct ImportedMemory<'a> {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: MemoryType,
    pub scope: MemoryScope,
    pub scope_key: &'a str,
    pub content: &'a str,
    pub origin_session_id: Uuid,
    /// As the sender proposed it. Normalized here, not trusted.
    pub topic_key: Option<&'a str>,
    pub value_key: Option<&'a str>,
    pub importance: Importance,
    pub effective_from: Option<&'a str>,
}

/// Store a memory another machine produced, letting a correction to a record
/// this store already holds actually land (T086, FR-712a).
///
/// **This was `INSERT OR IGNORE`, and the change is deliberate.** The old rule
/// read: two machines that wrote different things wrote *different rows*, the
/// id is the same only when it is the same record, so a second arrival can only
/// be a redelivery of what is already here. That was true while the local store
/// was an authority over its own memories. Under server authority the row is a
/// **cache** of a record the server owns, and the same statement makes a
/// server-side content correction unapplicable: the device would go on recalling
/// the uncorrected text for as long as the id survives. FR-701's declaration
/// that the server is correct does not execute itself — it needs a merge rule
/// able to accept a correction, which is what the `ON CONFLICT (id) DO UPDATE`
/// below is.
///
/// The reasoning extends [`crate::global::merge_synced_personal`]'s to project
/// memory, and it is the same reasoning the caller in `cairnd::sync` already
/// gives for not returning early on a memory it has seen: a peer re-sends a
/// memory precisely when something shareable about it changed. That was true of
/// verification before it was true of content; this makes the store side agree
/// with it rather than quietly discarding the change the caller went to the
/// trouble of delivering.
///
/// Nothing here consults a clock, so which copy arrives first still cannot
/// change the result by itself (FR-411, SC-304). What decides is arrival order,
/// and ordering pulled pages is the sync cursor's job rather than a per-row
/// comparison.
///
/// **Two rows are never refreshed**, and both exclusions are enforced in the
/// `DO UPDATE`'s own `WHERE`, not by a check the caller could skip:
///
/// * a `local_only` row. Knowledge marked local-only never went to the server
///   and is excluded from the durability guarantee by FR-706, so nothing can
///   legitimately come back bearing its id — but the guard is written anyway,
///   because the cost of being wrong is a record the user deliberately kept off
///   the network being overwritten by content that reached the network;
/// * a deleted row. `delete_memory` clears content and sets `deleted_at`
///   (FR-052). An arriving copy that rewrote content into a tombstone would
///   resurrect exactly what a deletion is supposed to have ended.
///
/// What is still deliberately not taken from the sender — these are derived
/// locally, and the `DO UPDATE` names none of them:
///
/// * `reinforcement_count` and `distinct_origin_count` — derived from the
///   records this store holds, and rebuilt by the caller after the arriving
///   decisions land;
/// * `superseded_at` and `state` — a view of the `supersedes` relations, which
///   `rebuild_supersession` recomputes when the decision itself arrives (D67);
/// * `verification` and `verification_authority` — rebuilt from the runs this
///   store has, by `import_verification`'s own path;
/// * `stale_at` — drift is what *this* machine observed about its own worktree;
/// * `pinned` — an attention decision governed by this project's local pin
///   budget (D75), which adopting a peer's pins would silently exceed;
/// * `created_at`, `origin_session_id` and `local_only` — established when the
///   row first appeared here and not restated by a refresh.
///
/// Returns whether the incoming proposal landed. That is a change of meaning
/// from "a row was written": under the old rule the two were the same question,
/// and under this one they are not. `false` now means a guard above suppressed
/// the write, which is the answer a caller actually needs — "you already had
/// this" stopped being interesting the moment a second arrival could carry a
/// correction.
pub async fn import_memory(store: &Store, m: ImportedMemory<'_>) -> Result<bool> {
    // A project-scoped memory is scoped to *the project*, and each machine
    // names that project with its own local id. Storing the sender's id would
    // file the arriving proposal under a scope key this store's own reads never
    // look at: present, searchable by text, and invisible to every subject
    // read — so two machines could never converge on a project-scoped subject
    // at all, which is most of them.
    //
    // Only `project` needs the mapping. A branch key is a branch name, a task
    // key is a task id that travels with the task, and a session key belongs to
    // the machine that opened the session.
    let scope_key = match m.scope {
        MemoryScope::Project => m.project_id.to_string(),
        _ => m.scope_key.to_string(),
    };

    // Normalized again rather than trusted: the sender's normalizer is the
    // sender's, and a key this store cannot represent must be dropped rather
    // than stored in a shape its own reads would miss (FR-312).
    let topic_key = m
        .topic_key
        .and_then(cairn_core::knowledge::normalize_topic_key);
    // A value key means nothing without a topic key to compare within, so it
    // is dropped along with one this store could not represent (FR-311).
    let value_key = topic_key
        .as_ref()
        .and(m.value_key)
        .and_then(cairn_core::knowledge::normalize_value_key);

    let now = rows::now_text();
    // `excluded.*` names the row this statement tried to insert, so the update
    // restates the incoming values without binding any parameter twice.
    //
    // The `WHERE` on the `DO UPDATE` is the whole of the two exclusions: a
    // `local_only` or already-deleted row matches the conflict, fails the
    // predicate, and is left exactly as it is while the statement reports zero
    // rows affected. Written here rather than as a `SELECT` beforehand because
    // this runs outside a transaction — a read-then-write would leave a window
    // in which the flag changed between the two.
    let wrote = sqlx::query(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
             origin_session_id, local_only, created_at, updated_at,
             topic_key, value_key, content_norm_digest, importance, effective_from)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', NULL, ?7, 0, ?8, ?8,
                 ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT (id) DO UPDATE SET
             type                = excluded.type,
             scope               = excluded.scope,
             scope_key           = excluded.scope_key,
             content             = excluded.content,
             content_norm_digest = excluded.content_norm_digest,
             topic_key           = excluded.topic_key,
             value_key           = excluded.value_key,
             importance          = excluded.importance,
             effective_from      = excluded.effective_from,
             updated_at          = excluded.updated_at
           WHERE memories.local_only = 0 AND memories.deleted_at IS NULL",
    )
    .bind(m.id.to_string())
    .bind(m.project_id.to_string())
    .bind(m.kind.as_str())
    .bind(m.scope.as_str())
    .bind(&scope_key)
    .bind(m.content)
    .bind(m.origin_session_id.to_string())
    .bind(&now)
    .bind(topic_key.as_deref())
    .bind(value_key.as_deref())
    // Derived here, never sent: it is a local index, and FR-506 forbids it on
    // the wire.
    .bind(cairn_core::knowledge::content_norm_digest(m.content))
    .bind(m.importance.as_str())
    .bind(m.effective_from.unwrap_or(&now))
    .execute(store.pool())
    .await?
    .rows_affected()
        > 0;

    Ok(wrote)
}

/// Create a proposal and run bounded reconciliation in the same transaction.
///
/// The proposal and any relation it implies commit together
/// (`contracts/records-and-rebuild.md` §Aggregate ownership), so a reader never
/// sees a member without the decision that placed it.
///
/// Exactly one merging case exists — content identical after normalization —
/// and it is the only one Cairn can decide without inference (D46). A shared
/// value key with differing content records **nothing**: the value is agreed
/// and the statements are several.
pub async fn create_memory_reconciled(
    store: &Store,
    m: NewMemory<'_>,
    policy: SyncPolicy,
    reconcile_members_max: usize,
) -> Result<CreateOutcome> {
    let id = new_id();
    let now = rows::now_text();
    let mut notes: Vec<&'static str> = Vec::new();

    // An unrepresentable key never rejects the memory (FR-312).
    let topic_key = match m.topic_key {
        Some(raw) => {
            let normalized = knowledge_core::normalize_topic_key(raw);
            if normalized.is_none() {
                notes.push(cairn_core::wire::codes::INVALID_TOPIC_KEY);
            }
            normalized
        }
        None => None,
    };
    let value_key = match (m.value_key, topic_key.as_ref()) {
        (Some(raw), Some(_)) => knowledge_core::normalize_value_key(raw),
        // A value key with no topic key has no subject to be a value of. The
        // memory is still stored; the key is dropped and the reason reported.
        (Some(_), None) => {
            notes.push(cairn_core::wire::codes::VALUE_WITHOUT_TOPIC);
            None
        }
        (None, _) => None,
    };
    let content_norm_digest = knowledge_core::content_norm_digest(m.content);

    let mut tx = tx::begin(store, "create_memory").await?;
    sqlx::query(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
             origin_session_id, local_only, created_at, updated_at,
             topic_key, value_key, content_norm_digest, importance, effective_from)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', NULL, ?7, ?8, ?9, ?9,
                 ?10, ?11, ?12, ?13, ?9)",
    )
    .bind(id.to_string())
    .bind(m.project_id.to_string())
    .bind(m.kind.as_str())
    .bind(m.scope.as_str())
    .bind(m.scope_key)
    .bind(m.content)
    .bind(m.origin_session_id.to_string())
    .bind(m.local_only as i64)
    .bind(&now)
    .bind(topic_key.as_deref())
    .bind(value_key.as_deref())
    .bind(&content_norm_digest)
    .bind(m.importance.as_str())
    .execute(&mut *tx)
    .await?;

    for obs_id in m.evidence {
        // The digest lets provenance stay meaningful after the observation is
        // deleted, without retaining what the user removed.
        let digest = match sqlx::query("SELECT summary FROM observations WHERE id = ?1")
            .bind(obs_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
        {
            Some(r) => {
                let summary: String = r.try_get("summary")?;
                cairn_core::digest(&summary)
            }
            None => cairn_core::digest(&obs_id.to_string()),
        };
        sqlx::query(
            "INSERT OR IGNORE INTO memory_evidence (memory_id, observation_id, content_digest)
             VALUES (?1, ?2, ?3)",
        )
        .bind(id.to_string())
        .bind(obs_id.to_string())
        .bind(digest)
        .execute(&mut *tx)
        .await?;
    }

    // A `local_only` memory is never transmitted (FR-051), so it never even
    // produces a row to transmit.
    if !m.local_only {
        let evidence: Vec<EvidenceRef> = m
            .evidence
            .iter()
            .map(|o| EvidenceRef {
                observation_id: *o,
                content_digest: String::new(),
                deleted: false,
            })
            .collect();
        let staged = Memory {
            id,
            project_id: m.project_id,
            kind: m.kind,
            scope: m.scope,
            scope_key: m.scope_key.to_string(),
            content: m.content.to_string(),
            state: MemoryState::Active,
            superseded_by_id: None,
            origin_session_id: m.origin_session_id,
            local_only: false,
            evidence,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        // Built before the enqueue, not inside its argument list: both need the
        // transaction, and only one may borrow it at a time.
        let payload = outbox::memory_payload_for(&mut tx, &staged).await?;
        outbox::enqueue(
            &mut *tx,
            policy,
            m.project_id,
            OutboxEntityType::Memory,
            id,
            OutboxOperation::Upsert,
            &payload,
        )
        .await?;
    }

    // Bounded reconciliation, in this same transaction (FR-474).
    let mut reconciliation = ProposalOutcome::Created;
    let mut relation_recorded: Option<cairn_core::RelationKind> = None;
    let mut matched_value_key: Option<String> = None;
    if let Some(topic) = topic_key.as_deref() {
        let (members, over_bound) = crate::knowledge::subject_members_tx(
            &mut tx,
            m.project_id,
            m.scope,
            m.scope_key,
            topic,
            reconcile_members_max,
        )
        .await?;

        let proposal = knowledge_core::MemoryFacts {
            id,
            state: MemoryState::Active,
            scope: m.scope,
            scope_key: m.scope_key.to_string(),
            topic_key: topic_key.clone(),
            value_key: value_key.clone(),
            content_norm_digest: Some(content_norm_digest.clone()),
            verification: cairn_core::VerificationState::Unverified,
            verification_authority: None,
            evidence_fact_count: 0,
            pinned: false,
            importance: m.importance,
            origin_session_id: m.origin_session_id,
        };

        if over_bound {
            reconciliation = ProposalOutcome::Deferred;
            notes.push(cairn_core::wire::codes::RECONCILIATION_DEFERRED);
        } else {
            let (outcome, relations) =
                knowledge_core::classify_proposal(&proposal, &members, reconcile_members_max);
            let mut kinds: Vec<cairn_core::RelationKind> = Vec::new();
            for r in relations {
                kinds.push(r.kind);
                crate::knowledge::record_relation_tx(
                    &mut tx,
                    crate::knowledge::NewRelation {
                        project_id: m.project_id,
                        from: r.from,
                        to: r.to,
                        kind: r.kind,
                        decided_by_session: m.origin_session_id,
                        basis: r.basis,
                        basis_evidence_id: None,
                        rationale: None,
                    },
                )
                .await?;
            }
            if matches!(outcome, ProposalOutcome::Corroborating { .. }) {
                notes.push(cairn_core::wire::codes::CORROBORATING_MEMBER);
            }
            // What the write recorded, not what an outcome implies. Every
            // relation a single classification returns shares one kind.
            relation_recorded = kinds.first().copied();
            // The matched member is already in `members`; reading its value key
            // here is what keeps the report free of an extra query.
            matched_value_key = match &outcome {
                ProposalOutcome::Duplicate { of } => members
                    .iter()
                    .find(|m| m.id == *of)
                    .and_then(|m| m.value_key.clone()),
                ProposalOutcome::Corroborating { member } => members
                    .iter()
                    .find(|m| m.id == *member)
                    .and_then(|m| m.value_key.clone()),
                _ => None,
            };
            reconciliation = outcome;
        }
    }

    tx::commit(tx, "create_memory").await?;
    let memory = memory(store, id).await?;
    Ok(CreateOutcome {
        memory,
        reconciliation,
        relation_recorded,
        matched_value_key,
        subject: topic_key,
        notes,
    })
}

pub async fn memory(store: &Store, id: Uuid) -> Result<Memory> {
    let row = sqlx::query("SELECT * FROM memories WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("memory {id}")))?;
    let mut m = rows::memory_bare(&row)?;
    m.evidence = evidence_for(store, id).await?;
    Ok(m)
}

/// Evidence references, with deleted observations reported rather than hidden.
pub async fn evidence_for(store: &Store, memory_id: Uuid) -> Result<Vec<EvidenceRef>> {
    let rs = sqlx::query(
        "SELECT e.observation_id, e.content_digest, o.id IS NULL OR o.deleted_at IS NOT NULL AS gone
         FROM memory_evidence e
         LEFT JOIN observations o ON o.id = e.observation_id
         WHERE e.memory_id = ?1
         ORDER BY e.observation_id",
    )
    .bind(memory_id.to_string())
    .fetch_all(store.pool())
    .await?;

    rs.iter()
        .map(|r| {
            Ok(EvidenceRef {
                observation_id: rows::uuid(r, "observation_id")?,
                content_digest: r.try_get("content_digest")?,
                deleted: rows::boolean(r, "gone")?,
            })
        })
        .collect()
}

/// Replace a memory, retaining the original and the link (FR-020).
pub async fn supersede_memory(
    store: &Store,
    original_id: Uuid,
    replacement: NewMemory<'_>,
    policy: SyncPolicy,
) -> Result<(Memory, Memory)> {
    let original = memory(store, original_id).await?;
    let session = replacement.origin_session_id;
    let new = create_memory(store, replacement, policy).await?;
    let now = rows::now_text();

    // The relation, the lifecycle columns and the pin move together (FR-323,
    // FR-341, FR-456). Feature 001's `state` and `superseded_by_id` become a
    // *view* of the relation, which is what makes FR-324 true and what lets a
    // remotely decided supersession land on import without a row being
    // overwritten.
    crate::constraints::check_supersession(MemoryState::Superseded.as_str(), Some(&now))?;
    let mut tx = tx::begin(store, "supersede_memory").await?;

    crate::knowledge::record_relation_tx(
        &mut tx,
        crate::knowledge::NewRelation {
            project_id: original.project_id,
            from: new.id,
            to: original.id,
            kind: RelationKind::Supersedes,
            decided_by_session: session,
            // Supersession is never automatic (FR-325): reaching this function
            // is itself the explicit act.
            basis: RelationBasis::ExplicitAgent,
            basis_evidence_id: None,
            rationale: None,
        },
    )
    .await?;

    sqlx::query(
        "UPDATE memories SET state = 'superseded', superseded_by_id = ?1, updated_at = ?2,
                             superseded_at = ?2,
                             pinned = 0, pinned_at = NULL, pinned_by_session = NULL,
                             pin_reason = NULL
         WHERE id = ?3",
    )
    .bind(new.id.to_string())
    .bind(&now)
    .bind(original.id.to_string())
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "supersede_memory").await?;

    let updated = memory(store, original_id).await?;
    if !updated.local_only {
        let mut tx = tx::begin(store, "supersede_memory").await?;
        let payload = outbox::memory_payload_for(&mut tx, &updated).await?;
        outbox::enqueue(
            &mut *tx,
            policy,
            updated.project_id,
            OutboxEntityType::Memory,
            updated.id,
            OutboxOperation::Upsert,
            &payload,
        )
        .await?;
        tx::commit(tx, "supersede_memory").await?;
    }
    Ok((updated, new))
}

/// Mark memories whose scope key no longer resolves as `stale`, never delete
/// them (US3 scenario 5).
pub async fn mark_stale_scopes(
    store: &Store,
    project_id: Uuid,
    live_branches: &[String],
) -> Result<u64> {
    let mut marked = 0u64;
    let rs = sqlx::query(
        "SELECT id, scope, scope_key FROM memories
         WHERE project_id = ?1 AND state = 'active' AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;

    for r in &rs {
        let scope: MemoryScope = rows::enum_val(r, "scope")?;
        let key: String = r.try_get("scope_key")?;
        let gone = match scope {
            MemoryScope::Branch => !live_branches.iter().any(|b| b == &key),
            MemoryScope::Task => {
                let n: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM tasks WHERE id = ?1 AND deleted_at IS NULL",
                )
                .bind(&key)
                .fetch_one(store.pool())
                .await?;
                n == 0
            }
            _ => false,
        };
        if gone {
            // `stale_at` records the instant Cairn itself performed the
            // transition, going forward only (FR-341, D82). A memory that went
            // stale before this feature existed keeps NULL, which means
            // **unknown** — never "not stale" — and a historical answer says so
            // rather than presenting an unbounded interval as fact. Inferring
            // one from `updated_at` would be a second approximation on top of
            // the one the migration already documents, and several paths touch
            // `updated_at`, so it is a worse source here than for supersession.
            sqlx::query(
                "UPDATE memories SET state = 'stale', updated_at = ?1, stale_at = ?1
                 WHERE id = ?2",
            )
            .bind(rows::now_text())
            .bind(rows::uuid(r, "id")?.to_string())
            .execute(store.pool())
            .await?;
            marked += 1;
        }
    }
    Ok(marked)
}

pub async fn count_memories(store: &Store, project_id: Uuid) -> Result<i64> {
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memories WHERE project_id = ?1 AND deleted_at IS NULL",
    )
    .bind(project_id.to_string())
    .fetch_one(store.pool())
    .await?;
    Ok(n)
}

/// Decision-typed memories produced by one session, for handoff synthesis.
pub async fn decision_memories_for_session(store: &Store, session_id: Uuid) -> Result<Vec<Memory>> {
    let rs = sqlx::query(
        "SELECT * FROM memories
         WHERE origin_session_id = ?1 AND type = 'decision' AND deleted_at IS NULL
         ORDER BY created_at",
    )
    .bind(session_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::memory_bare).collect()
}

// ---------------------------------------------------------------------------
// Handoffs
// ---------------------------------------------------------------------------

pub async fn insert_handoff(
    store: &Store,
    h: &Handoff,
    project_id: Uuid,
    policy: SyncPolicy,
) -> Result<Handoff> {
    let mut tx = tx::begin(store, "insert_handoff").await?;
    sqlx::query(
        "INSERT INTO handoffs
            (id, session_id, trigger, goal, progress, completed_work, remaining_work,
             changed_files, decisions, failures, tests_executed, repository_state,
             next_step, agent_note, evidence, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )
    .bind(h.id.to_string())
    .bind(h.session_id.to_string())
    .bind(h.trigger.as_str())
    .bind(&h.goal)
    .bind(&h.progress)
    .bind(json(&h.completed_work))
    .bind(json(&h.remaining_work))
    .bind(json(&h.changed_files))
    .bind(json(&h.decisions))
    .bind(json(&h.failures))
    .bind(json(&h.tests_executed))
    .bind(json(&h.repository_state))
    .bind(&h.next_step)
    .bind(h.agent_note.as_deref())
    .bind(json(&h.evidence))
    .bind(rows::ts_text(h.created_at))
    .execute(&mut *tx)
    .await?;
    outbox::enqueue(
        &mut *tx,
        policy,
        project_id,
        OutboxEntityType::Handoff,
        h.id,
        OutboxOperation::Upsert,
        &outbox::handoff_payload(h),
    )
    .await?;
    tx::commit(tx, "insert_handoff").await?;
    handoff(store, h.id).await
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

pub async fn handoff(store: &Store, id: Uuid) -> Result<Handoff> {
    let row = sqlx::query("SELECT * FROM handoffs WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("handoff {id}")))?;
    rows::handoff(&row)
}

pub async fn latest_handoff(store: &Store, session_id: Uuid) -> Result<Option<Handoff>> {
    let row = sqlx::query(
        "SELECT * FROM handoffs WHERE session_id = ?1 AND deleted_at IS NULL
         ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .bind(session_id.to_string())
    .fetch_optional(store.pool())
    .await?;
    row.as_ref().map(rows::handoff).transpose()
}

pub async fn handoffs_for_session(store: &Store, session_id: Uuid) -> Result<Vec<Handoff>> {
    let rs = sqlx::query(
        "SELECT * FROM handoffs WHERE session_id = ?1 AND deleted_at IS NULL
         ORDER BY created_at DESC",
    )
    .bind(session_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::handoff).collect()
}

/// A bounded, attributed note beside the derived record — never in place of it
/// (FR-034).
pub async fn annotate_handoff(store: &Store, id: Uuid, note: &str) -> Result<Handoff> {
    sqlx::query("UPDATE handoffs SET agent_note = ?1 WHERE id = ?2")
        .bind(note)
        .bind(id.to_string())
        .execute(store.pool())
        .await?;
    handoff(store, id).await
}

/// The most recent handoff of the session that preceded `session_id`.
pub async fn previous_handoff_for(store: &Store, session_id: Uuid) -> Result<Option<Handoff>> {
    let s = session(store, session_id).await?;
    let Some(prev) = s.previous_session_id else {
        return Ok(None);
    };
    latest_handoff(store, prev).await
}

// ---------------------------------------------------------------------------
// Deletion (FR-052)
// ---------------------------------------------------------------------------

fn tombstone_now() -> String {
    rows::now_text()
}

/// Clear an observation's content, keeping identity so provenance resolves.
pub async fn delete_observation(store: &Store, id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE observations
         SET deleted_at = ?1, summary = '', path = NULL, command = NULL, details = NULL,
             outcome = NULL, exit_code = NULL, payload_bytes = 0
         WHERE id = ?2",
    )
    .bind(tombstone_now())
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    store.checkpoint().await?;
    Ok(())
}

/// Delete one memory and nothing else. Its evidence, origin session and every
/// other memory are untouched.
pub async fn delete_memory(store: &Store, id: Uuid, policy: SyncPolicy) -> Result<()> {
    let m = memory(store, id).await?;
    let mut tx = tx::begin(store, "delete_memory").await?;
    sqlx::query("UPDATE memories SET deleted_at = ?1, content = '' WHERE id = ?2")
        .bind(tombstone_now())
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    if !m.local_only {
        outbox::enqueue(
            &mut *tx,
            policy,
            m.project_id,
            OutboxEntityType::Memory,
            id,
            OutboxOperation::Delete,
            &outbox::delete_payload(id),
        )
        .await?;
    }
    tx::commit(tx, "delete_memory").await?;
    store.checkpoint().await?;
    Ok(())
}

/// Delete one handoff and nothing else.
pub async fn delete_handoff(
    store: &Store,
    id: Uuid,
    project_id: Uuid,
    policy: SyncPolicy,
) -> Result<()> {
    let mut tx = tx::begin(store, "delete_handoff").await?;
    sqlx::query(
        "UPDATE handoffs
         SET deleted_at = ?1, goal = '', progress = '', next_step = '', agent_note = NULL,
             completed_work = '[]', remaining_work = '[]', changed_files = '[]',
             decisions = '[]', failures = '[]', tests_executed = '[]', repository_state = '{}'
         WHERE id = ?2",
    )
    .bind(tombstone_now())
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;
    outbox::enqueue(
        &mut *tx,
        policy,
        project_id,
        OutboxEntityType::Handoff,
        id,
        OutboxOperation::Delete,
        &outbox::delete_payload(id),
    )
    .await?;
    tx::commit(tx, "delete_handoff").await?;
    store.checkpoint().await?;
    Ok(())
}

/// Delete a session: the session and its observations are cleared, and the
/// memories and handoffs it produced **survive** with origin marked deleted.
///
/// `with_memories` is the only way to take those memories too — never the
/// default, because a cascade here would destroy the durable knowledge the
/// product exists to keep (FR-052).
pub async fn delete_session(
    store: &Store,
    id: Uuid,
    with_memories: bool,
    policy: SyncPolicy,
) -> Result<()> {
    let existing = session(store, id).await?;
    let doomed_memories: Vec<Uuid> = if with_memories {
        sqlx::query(
            "SELECT id FROM memories WHERE origin_session_id = ?1 AND deleted_at IS NULL
               AND local_only = 0",
        )
        .bind(id.to_string())
        .fetch_all(store.pool())
        .await?
        .iter()
        .map(|r| rows::uuid(r, "id"))
        .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    let now = tombstone_now();
    let mut tx = tx::begin(store, "delete_session").await?;

    sqlx::query(
        "UPDATE observations
         SET deleted_at = ?1, summary = '', path = NULL, command = NULL, details = NULL,
             outcome = NULL, exit_code = NULL, payload_bytes = 0
         WHERE session_id = ?2 AND deleted_at IS NULL",
    )
    .bind(&now)
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;

    if with_memories {
        sqlx::query(
            "UPDATE memories SET deleted_at = ?1, content = ''
             WHERE origin_session_id = ?2 AND deleted_at IS NULL",
        )
        .bind(&now)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE sessions SET deleted_at = ?1, end_reason = NULL, agent_session_key = ?2
         WHERE id = ?3",
    )
    .bind(&now)
    // Free the unique key so a new session may reuse it.
    .bind(format!("deleted:{id}"))
    .bind(id.to_string())
    .execute(&mut *tx)
    .await?;

    outbox::enqueue(
        &mut *tx,
        policy,
        existing.project_id,
        OutboxEntityType::Session,
        id,
        OutboxOperation::Delete,
        &outbox::delete_payload(id),
    )
    .await?;
    for m in doomed_memories {
        outbox::enqueue(
            &mut *tx,
            policy,
            existing.project_id,
            OutboxEntityType::Memory,
            m,
            OutboxOperation::Delete,
            &outbox::delete_payload(m),
        )
        .await?;
    }

    tx::commit(tx, "delete_session").await?;
    store.checkpoint().await?;
    Ok(())
}

/// True when the session row exists at all, deleted or not — provenance must
/// still resolve after a delete.
pub async fn session_exists(store: &Store, id: Uuid) -> Result<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE id = ?1")
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await?;
    Ok(n > 0)
}

pub async fn session_is_deleted(store: &Store, id: Uuid) -> Result<bool> {
    let row = sqlx::query("SELECT deleted_at FROM sessions WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?;
    match row {
        None => Ok(true),
        Some(r) => Ok(rows::opt_ts(&r, "deleted_at")?.is_some()),
    }
}

// ---------------------------------------------------------------------------
// Writer identity (D407, FR-490)
// ---------------------------------------------------------------------------

/// This store's opaque writer identity.
///
/// Seeded exactly once, by migration 7's finish hook (`migrate::finish`,
/// `seed_writer_identity`) — this is only a reader, never a generator. The
/// singleton row it reads is guaranteed to exist by the migration that created
/// it, in the same transaction, the same way `writer_identity`'s own `CHECK (id
/// = 1)` guarantees there is never more than one (`0007_collaborative_global_
/// memory.sql`). A store that predates migration 7 cannot reach this function
/// with a linked personal or team namespace, because nothing routes to one
/// before the migration that creates both the table and the row has run.
pub async fn writer_identity(store: &Store) -> Result<Uuid> {
    let row = sqlx::query("SELECT writer_id FROM writer_identity WHERE id = 1")
        .fetch_one(store.pool())
        .await?;
    rows::uuid(&row, "writer_id")
}

/// The next sequence number in this writer's `personal_knowledge` stream
/// (D408, FR-445, FR-492).
///
/// Derived from what is already stored rather than tracked by a separate
/// counter row: `MAX(writer_seq) + 1` over this store's own `writer_id` is
/// exactly as monotonic as a dedicated counter would be, because this store is
/// the only writer that can ever produce a row bearing its own `writer_id` —
/// every row bearing a *different* `writer_id` reached this table by import,
/// never by a local write here. Call inside the same transaction that inserts
/// the record: SQLite serializes writers, so the daemon and the CLI can never
/// observe (and then both consume) the same next value.
///
/// `writer_seq` is diagnostic only (§9 of the contract) — this function hands
/// out the next number for a record to carry, and nothing here or downstream
/// ever compares one writer's sequence against another's to decide anything.
pub async fn next_personal_writer_seq(
    tx: &mut sqlx::SqliteConnection,
    writer_id: Uuid,
) -> Result<i64> {
    let next: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(writer_seq), 0) + 1 FROM personal_knowledge WHERE writer_id = ?1",
    )
    .bind(writer_id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    Ok(next)
}

/// The next sequence number in this writer's `team_knowledge` stream. Same
/// construction as [`next_personal_writer_seq`], over the separate `team_
/// knowledge` sequence space `team_knowledge_writer_seq`'s unique index keeps
/// (`0007_collaborative_global_memory.sql`) — personal and team are different
/// streams for the same writer, not one shared counter.
pub async fn next_team_writer_seq(tx: &mut sqlx::SqliteConnection, writer_id: Uuid) -> Result<i64> {
    let next: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(writer_seq), 0) + 1 FROM team_knowledge WHERE writer_id = ?1",
    )
    .bind(writer_id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    Ok(next)
}

// ---------------------------------------------------------------------------
// Sync metadata
// ---------------------------------------------------------------------------

pub async fn last_sync_success(store: &Store, project_id: Uuid) -> Result<Option<DateTime<Utc>>> {
    let row = sqlx::query("SELECT last_success_at FROM sync_meta WHERE project_id = ?1")
        .bind(project_id.to_string())
        .fetch_optional(store.pool())
        .await?;
    match row {
        None => Ok(None),
        Some(r) => rows::opt_ts(&r, "last_success_at"),
    }
}

pub async fn record_sync_success(store: &Store, project_id: Uuid) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_meta (project_id, last_success_at) VALUES (?1, ?2)
         ON CONFLICT(project_id) DO UPDATE SET last_success_at = ?2",
    )
    .bind(project_id.to_string())
    .bind(rows::now_text())
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn pull_cursor(store: &Store, project_id: Uuid) -> Result<Option<String>> {
    let row = sqlx::query("SELECT pull_cursor FROM sync_meta WHERE project_id = ?1")
        .bind(project_id.to_string())
        .fetch_optional(store.pool())
        .await?;
    Ok(row.and_then(|r| r.try_get::<Option<String>, _>("pull_cursor").ok().flatten()))
}

pub async fn set_pull_cursor(store: &Store, project_id: Uuid, cursor: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_meta (project_id, pull_cursor) VALUES (?1, ?2)
         ON CONFLICT(project_id) DO UPDATE SET pull_cursor = ?2",
    )
    .bind(project_id.to_string())
    .bind(cursor)
    .execute(store.pool())
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Deferred pull records (#44)
// ---------------------------------------------------------------------------

/// A pulled record held back because the parent it names had not arrived.
#[derive(Debug, Clone)]
pub struct DeferredRecord {
    pub kind: String,
    pub record_key: String,
    pub payload: String,
    pub waiting_on: String,
    pub attempts: i64,
    pub first_seen_at: String,
}

/// Hold a pulled record until the parent it names arrives.
///
/// Keyed by the record's own identity, so a record the server sends again
/// replaces the held copy instead of adding another. `first_seen_at` survives
/// the replacement: how long a record has been waiting is the diagnostic, and
/// re-sending it does not make the wait shorter.
pub async fn defer_pulled_record(
    store: &Store,
    project_id: Uuid,
    kind: &str,
    record_key: &str,
    payload: &str,
    waiting_on: &str,
) -> Result<()> {
    let now = rows::now_text();
    sqlx::query(
        "INSERT INTO sync_deferred
            (project_id, kind, record_key, payload, waiting_on, attempts,
             first_seen_at, last_attempt_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)
         ON CONFLICT(project_id, kind, record_key) DO UPDATE SET
            payload = ?4, waiting_on = ?5, last_attempt_at = ?6",
    )
    .bind(project_id.to_string())
    .bind(kind)
    .bind(record_key)
    .bind(payload)
    .bind(waiting_on)
    .bind(&now)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Records waiting on a parent, oldest wait first.
///
/// Bounded: a backlog must not turn one pull into an unbounded amount of work.
/// Whatever does not fit is offered again on the next pull.
pub async fn deferred_records(
    store: &Store,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<DeferredRecord>> {
    let rs = sqlx::query(
        "SELECT kind, record_key, payload, waiting_on, attempts, first_seen_at
           FROM sync_deferred
          WHERE project_id = ?1
          ORDER BY first_seen_at ASC
          LIMIT ?2",
    )
    .bind(project_id.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await?;
    Ok(rs
        .iter()
        .map(|r| DeferredRecord {
            kind: r.get::<String, _>("kind"),
            record_key: r.get::<String, _>("record_key"),
            payload: r.get::<String, _>("payload"),
            waiting_on: r.get::<String, _>("waiting_on"),
            attempts: r.get::<i64, _>("attempts"),
            first_seen_at: r.get::<String, _>("first_seen_at"),
        })
        .collect())
}

/// The record landed, or can never land: stop holding it.
pub async fn clear_deferred_record(
    store: &Store,
    project_id: Uuid,
    kind: &str,
    record_key: &str,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM sync_deferred
          WHERE project_id = ?1 AND kind = ?2 AND record_key = ?3",
    )
    .bind(project_id.to_string())
    .bind(kind)
    .bind(record_key)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// The parent still has not arrived. Recorded so a record waiting on one that
/// never comes is visible rather than merely retried in silence.
pub async fn note_deferred_attempt(
    store: &Store,
    project_id: Uuid,
    kind: &str,
    record_key: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE sync_deferred
            SET attempts = attempts + 1, last_attempt_at = ?4
          WHERE project_id = ?1 AND kind = ?2 AND record_key = ?3",
    )
    .bind(project_id.to_string())
    .bind(kind)
    .bind(record_key)
    .bind(rows::now_text())
    .execute(store.pool())
    .await?;
    Ok(())
}

/// How many records this project is holding, for `cairn status` and `doctor`.
pub async fn deferred_count(store: &Store, project_id: Uuid) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sync_deferred WHERE project_id = ?1")
            .bind(project_id.to_string())
            .fetch_one(store.pool())
            .await?,
    )
}

/// What the server last said it could hold, verbatim.
///
/// Cached so the probe runs at most once per drain cycle rather than once per
/// item, and compared as an opaque string: a change of any kind is a reason to
/// look again at what is blocked, and interpreting the value is the caller's
/// job (FR-418).
pub async fn server_capability(store: &Store, project_id: Uuid) -> Result<Option<String>> {
    let row = sqlx::query("SELECT server_capability FROM sync_meta WHERE project_id = ?1")
        .bind(project_id.to_string())
        .fetch_optional(store.pool())
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<String>, _>("server_capability")
            .ok()
            .flatten()
    }))
}

pub async fn set_server_capability(
    store: &Store,
    project_id: Uuid,
    capability: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_meta (project_id, server_capability) VALUES (?1, ?2)
         ON CONFLICT(project_id) DO UPDATE SET server_capability = ?2",
    )
    .bind(project_id.to_string())
    .bind(capability)
    .execute(store.pool())
    .await?;
    Ok(())
}

/// Seal a session's termination, durably, before anything is acknowledged
/// (FR-240 clause 1, D22).
///
/// One transaction sets the terminal status, the end reason, `ended_at` and
/// `handoff_pending`. No Git, no capture quiesce, no synthesis — this is the
/// whole of what happens before the daemon answers, which is what makes a
/// one-second vendor handler budget survivable without giving up the
/// completion guarantee.
pub async fn seal_session(
    store: &Store,
    id: Uuid,
    status: SessionStatus,
    reason: Option<&str>,
    policy: SyncPolicy,
) -> Result<Session> {
    let now = rows::now_text();
    sqlx::query(
        "UPDATE sessions
         SET status = ?1, ended_at = ?2, last_event_at = ?2, end_reason = ?3,
             handoff_pending = 1, handoff_attempts = 0, handoff_error = NULL
         WHERE id = ?4",
    )
    .bind(status.as_str())
    .bind(&now)
    .bind(reason)
    .bind(id.to_string())
    .execute(store.pool())
    .await?;
    let sealed = session(store, id).await?;
    enqueue_session(store, policy, &sealed).await?;
    Ok(sealed)
}

/// The handoff landed: the boundary is complete (FR-240 clause 2).
pub async fn clear_handoff_pending(store: &Store, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE sessions SET handoff_pending = 0, handoff_error = NULL WHERE id = ?1")
        .bind(id.to_string())
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Synthesis failed again. The reason is redacted and never carries file or
/// conversation content (FR-240 clause 3).
pub async fn record_handoff_failure(store: &Store, id: Uuid, reason: &str) -> Result<i64> {
    let redacted = cairn_core::redact::redact(reason);
    let bounded = cairn_core::bound::bound_text(&redacted, 500).text;
    sqlx::query(
        "UPDATE sessions
         SET handoff_attempts = handoff_attempts + 1, handoff_error = ?2
         WHERE id = ?1",
    )
    .bind(id.to_string())
    .bind(&bounded)
    .execute(store.pool())
    .await?;
    let row = sqlx::query("SELECT handoff_attempts FROM sessions WHERE id = ?1")
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await?;
    Ok(row.try_get::<i64, _>("handoff_attempts")?)
}

/// Sessions that acknowledged a boundary but have not produced its handoff.
///
/// Ordered oldest first, so the sweep clears the longest-owed first. A
/// terminal session never sits silently owing a handoff (FR-240).
pub async fn sessions_awaiting_handoff(
    store: &Store,
    sealed_before: DateTime<Utc>,
) -> Result<Vec<Session>> {
    let rows = sqlx::query(
        "SELECT * FROM sessions
         WHERE handoff_pending = 1 AND deleted_at IS NULL AND ended_at <= ?1
         ORDER BY ended_at",
    )
    .bind(rows::ts_text(sealed_before))
    .fetch_all(store.pool())
    .await?;
    rows.iter().map(rows::session).collect()
}

/// How many boundaries are currently owed a handoff, and which have stopped
/// being retried quickly. Reported in `cairn status` and doctor's core section.
pub async fn handoff_debt(store: &Store) -> Result<(i64, Vec<(Uuid, String)>)> {
    let owed: i64 = sqlx::query("SELECT COUNT(*) AS n FROM sessions WHERE handoff_pending = 1")
        .fetch_one(store.pool())
        .await?
        .try_get("n")?;
    let rows = sqlx::query(
        "SELECT id, handoff_error FROM sessions
         WHERE handoff_pending = 1 AND handoff_error IS NOT NULL
         ORDER BY ended_at",
    )
    .fetch_all(store.pool())
    .await?;
    let mut failures = Vec::new();
    for r in &rows {
        failures.push((
            rows::uuid(r, "id")?,
            r.try_get::<String, _>("handoff_error")?,
        ));
    }
    Ok((owed, failures))
}

#[cfg(test)]
mod idle_tests {
    use super::*;
    use crate::Store;

    /// A project for the sessions to hang off.
    async fn seed_project(store: &Store) -> Uuid {
        let id = Uuid::now_v7();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO projects
               (id, name, git_common_dir, repository_remote, linked,
                server_project_id, created_at, updated_at, deleted_at)
             VALUES (?1, 'test', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
        )
        .bind(id.to_string())
        .bind(format!("/tmp/git-{id}"))
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();
        id
    }

    /// Insert a session directly so its clock can be set to the past.
    async fn seed(store: &Store, project: Uuid, id: Uuid, status: &str, last_event: &str) {
        sqlx::query(
            "INSERT INTO sessions
               (id, project_id, task_id, user_id, agent, branch, commit_sha,
                worktree_path, agent_session_key, previous_session_id, status,
                started_at, ended_at, last_event_at, last_turn_ended_at,
                daemon_run_id, end_reason, deleted_at)
             VALUES (?1, ?2, NULL, ?3, 'claude-code', 'main', NULL,
                     '/tmp/wt', ?4, NULL, ?5,
                     ?6, NULL, ?6, NULL, ?7, NULL, NULL)",
        )
        .bind(id.to_string())
        .bind(project.to_string())
        .bind(Uuid::now_v7().to_string())
        .bind(format!("key-{id}"))
        .bind(status)
        .bind(last_event)
        .bind(Uuid::now_v7().to_string())
        .execute(store.pool())
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn only_quiet_active_sessions_are_offered_for_reaping() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .unwrap();

        let quiet = Uuid::now_v7();
        let busy = Uuid::now_v7();
        let already_done = Uuid::now_v7();

        let project = seed_project(&store).await;
        seed(
            &store,
            project,
            quiet,
            "active",
            "2020-01-01T00:00:00+00:00",
        )
        .await;
        seed(
            &store,
            project,
            busy,
            "active",
            &chrono::Utc::now().to_rfc3339(),
        )
        .await;
        seed(
            &store,
            project,
            already_done,
            "completed",
            "2020-01-01T00:00:00+00:00",
        )
        .await;

        let cutoff = chrono::Utc::now() - chrono::Duration::hours(2);
        let found = sessions_idle_since(&store, cutoff).await.unwrap();
        let ids: Vec<Uuid> = found.iter().map(|s| s.id).collect();

        assert!(
            ids.contains(&quiet),
            "a session that went quiet must be offered"
        );
        assert!(
            !ids.contains(&busy),
            "a session still receiving events must be left alone"
        );
        assert!(
            !ids.contains(&already_done),
            "a session that already ended must not be reaped twice"
        );
    }
}

// ---------------------------------------------------------------------------
// Feature 003 — the repository boundary for the columns SQLite cannot CHECK
// ---------------------------------------------------------------------------

/// Write the Feature 003 columns of a memory, refusing anything a `CHECK` would
/// have refused (data-model.md §2.1).
///
/// This is the single boundary those predicates are enforced at, so a later
/// writer cannot reach the columns without passing them. Only the fields a
/// caller supplies are written; `None` leaves the stored value alone, which is
/// what lets one function serve reconciliation, verification, pinning and the
/// temporal fields without any of them clobbering another's work.
pub async fn set_memory_intelligence(
    store: &Store,
    id: Uuid,
    columns: crate::constraints::MemoryColumns<'_>,
) -> Result<()> {
    crate::constraints::check_memory_columns(columns)?;

    let mut tx = tx::begin(store, "set_memory_intelligence").await?;
    sqlx::query(
        "UPDATE memories SET
             topic_key              = COALESCE(?2, topic_key),
             value_key              = COALESCE(?3, value_key),
             importance             = COALESCE(?4, importance),
             verification           = COALESCE(?5, verification),
             verification_authority = CASE WHEN ?5 IS NOT NULL THEN ?6
                                           ELSE COALESCE(?6, verification_authority) END,
             pinned                 = COALESCE(?7, pinned),
             pinned_at              = CASE WHEN ?7 IS NOT NULL THEN ?8
                                           ELSE COALESCE(?8, pinned_at) END,
             pinned_by_session      = CASE WHEN ?7 IS NOT NULL THEN ?9
                                           ELSE COALESCE(?9, pinned_by_session) END,
             pin_reason             = CASE WHEN ?7 IS NOT NULL THEN ?10
                                           ELSE COALESCE(?10, pin_reason) END
         WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(id.to_string())
    .bind(columns.topic_key)
    .bind(columns.value_key)
    .bind(columns.importance)
    .bind(columns.verification)
    .bind(columns.verification_authority)
    .bind(columns.pinned)
    .bind(columns.pinned_at)
    .bind(columns.pinned_by_session)
    .bind(columns.pin_reason)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod intelligence_constraint_tests {
    use super::*;
    use crate::constraints::MemoryColumns;
    use crate::Store;

    async fn store_with_memory() -> (tempfile::TempDir, Store, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .unwrap();

        let project = Uuid::now_v7();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                                   server_project_id, created_at, updated_at, deleted_at)
             VALUES (?1, 'test', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
        )
        .bind(project.to_string())
        .bind(format!("/tmp/git-{project}"))
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();

        let memory = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO memories (id, project_id, type, scope, scope_key, content, state,
                                   superseded_by_id, origin_session_id, local_only,
                                   created_at, updated_at)
             VALUES (?1, ?2, 'fact', 'project', ?3, 'a claim', 'active', NULL, ?4, 0, ?5, ?5)",
        )
        .bind(memory.to_string())
        .bind(project.to_string())
        .bind(project.to_string())
        .bind(Uuid::now_v7().to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();

        (dir, store, memory)
    }

    /// Each predicate a `CHECK` would have expressed is refused at the
    /// repository boundary, against a real store rather than in the abstract.
    #[tokio::test]
    async fn the_boundary_refuses_what_a_check_would_have() {
        let (_dir, store, id) = store_with_memory().await;

        let cases: Vec<(&str, MemoryColumns<'_>)> = vec![
            (
                "value_key IS NULL OR topic_key IS NOT NULL",
                MemoryColumns {
                    value_key: Some("postgresql"),
                    ..Default::default()
                },
            ),
            (
                "importance",
                MemoryColumns {
                    importance: Some("critical"),
                    ..Default::default()
                },
            ),
            (
                "verification",
                MemoryColumns {
                    verification: Some("probably"),
                    ..Default::default()
                },
            ),
            (
                "implies verification_authority IS NULL",
                MemoryColumns {
                    verification: Some("drifted"),
                    verification_authority: Some("cairn"),
                    ..Default::default()
                },
            ),
            (
                "pinned = 0 implies",
                MemoryColumns {
                    pinned: Some(0),
                    pin_reason: Some("a reason with no pin"),
                    ..Default::default()
                },
            ),
            (
                "pinned = 1 requires",
                MemoryColumns {
                    pinned: Some(1),
                    ..Default::default()
                },
            ),
        ];

        for (predicate, columns) in cases {
            let err = set_memory_intelligence(&store, id, columns)
                .await
                .expect_err(&format!("{predicate} was accepted"))
                .to_string();
            assert!(err.contains(predicate), "expected {predicate}, got {err}");
        }

        // And nothing was written by any of the refusals.
        let row: (Option<String>, String, Option<String>) = sqlx::query_as(
            "SELECT value_key, importance, verification_authority FROM memories WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(row, (None, "normal".to_string(), None));
    }

    #[tokio::test]
    async fn a_valid_write_lands() {
        let (_dir, store, id) = store_with_memory().await;

        set_memory_intelligence(
            &store,
            id,
            MemoryColumns {
                topic_key: Some("infra.production_database"),
                value_key: Some("postgresql"),
                importance: Some("high"),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let row: (Option<String>, Option<String>, String) =
            sqlx::query_as("SELECT topic_key, value_key, importance FROM memories WHERE id = ?1")
                .bind(id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(
            row,
            (
                Some("infra.production_database".into()),
                Some("postgresql".into()),
                "high".into()
            )
        );
    }

    /// Unpinning clears the pin's metadata rather than leaving it behind.
    #[tokio::test]
    async fn unpinning_clears_the_metadata_it_required() {
        let (_dir, store, id) = store_with_memory().await;

        set_memory_intelligence(
            &store,
            id,
            MemoryColumns {
                pinned: Some(1),
                pinned_at: Some("2026-01-01T00:00:00Z"),
                pinned_by_session: Some("s1"),
                pin_reason: Some("never move a published ref"),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        set_memory_intelligence(
            &store,
            id,
            MemoryColumns {
                pinned: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let row: (i64, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT pinned, pinned_at, pinned_by_session, pin_reason FROM memories WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(row, (0, None, None, None));
    }
}

// ---------------------------------------------------------------------------
// Pinned invariants (`contracts/continuity-context.md` Part 3)
// ---------------------------------------------------------------------------

/// Pin a memory, or unpin it.
///
/// Refused with `pin_budget_exhausted` when the project or scope budget is full,
/// **listing the current pins and unpinning nothing** (FR-454). Automatically
/// evicting someone else's constraint to make room for a new one would be the
/// opposite of what a pin is for.
pub async fn set_pinned(
    store: &Store,
    memory_id: Uuid,
    pinned: bool,
    reason: Option<&str>,
    session: Uuid,
    project_pin_budget: usize,
    scope_pin_budget: usize,
) -> Result<()> {
    let mut tx = tx::begin(store, "set_pinned").await?;
    let row =
        sqlx::query("SELECT project_id, scope, scope_key, pinned FROM memories WHERE id = ?1")
            .bind(memory_id.to_string())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| StoreError::NotFound(format!("memory {memory_id}")))?;
    let project_id: String = row.try_get("project_id")?;
    let scope: String = row.try_get("scope")?;
    let scope_key: String = row.try_get("scope_key")?;
    let already: i64 = row.try_get("pinned")?;

    if !pinned {
        sqlx::query(
            "UPDATE memories SET pinned = 0, pinned_at = NULL, pinned_by_session = NULL,
                                 pin_reason = NULL
             WHERE id = ?1",
        )
        .bind(memory_id.to_string())
        .execute(&mut *tx)
        .await?;
        return tx::commit(tx, "set_pinned").await;
    }
    if already == 1 {
        return tx::commit(tx, "set_pinned").await;
    }

    let in_project: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM memories WHERE project_id = ?1 AND pinned = 1")
            .bind(&project_id)
            .fetch_one(&mut *tx)
            .await?;
    let in_scope: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memories
          WHERE project_id = ?1 AND scope = ?2 AND scope_key = ?3 AND pinned = 1",
    )
    .bind(&project_id)
    .bind(&scope)
    .bind(&scope_key)
    .fetch_one(&mut *tx)
    .await?;

    if in_project as usize >= project_pin_budget || in_scope as usize >= scope_pin_budget {
        let current: Vec<String> =
            sqlx::query_scalar("SELECT id FROM memories WHERE project_id = ?1 AND pinned = 1")
                .bind(&project_id)
                .fetch_all(&mut *tx)
                .await?;
        return Err(StoreError::Refused {
            code: cairn_core::wire::codes::PIN_BUDGET_EXHAUSTED,
            message: format!(
                "the pin budget is full ({in_project} of {project_pin_budget} in this project, \
                 {in_scope} of {scope_pin_budget} in this scope); nothing was unpinned. \
                 Current pins: {}",
                current.join(", ")
            ),
        });
    }

    let bounded =
        reason.map(|r| cairn_core::bound::bound_text(&cairn_core::redact::redact(r), 200).text);
    sqlx::query(
        "UPDATE memories SET pinned = 1, pinned_at = ?2, pinned_by_session = ?3, pin_reason = ?4
         WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .bind(rows::now_text())
    .bind(session.to_string())
    .bind(bounded)
    .execute(&mut *tx)
    .await?;
    tx::commit(tx, "set_pinned").await
}

/// Clear a superseded memory's pin, in the caller's transaction.
///
/// The successor is pinned only explicitly: inheriting a pin would carry an
/// invariant onto a claim nobody chose to make invariant (FR-456).
pub async fn clear_pin_tx(tx: &mut sqlx::SqliteConnection, memory_id: Uuid) -> Result<()> {
    sqlx::query(
        "UPDATE memories SET pinned = 0, pinned_at = NULL, pinned_by_session = NULL,
                             pin_reason = NULL
         WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The pins in force here, ordered by scope precedence then importance.
///
/// A pin never widens scope: a pinned `branch:feature/x` memory is in force only
/// on that branch (FR-453). A pin whose claim drifted is **kept** and carries its
/// warning — a constraint that stopped being true is exactly what must be said
/// (FR-456).
pub async fn applicable_pins(
    store: &Store,
    project_id: Uuid,
    branch: &str,
    task_id: Option<Uuid>,
) -> Result<Vec<cairn_core::wire::PinnedConstraint>> {
    let rows = sqlx::query(
        "SELECT id, content, scope, scope_key, verification FROM memories
          WHERE project_id = ?1 AND pinned = 1 AND deleted_at IS NULL
            AND state != 'superseded'
            AND ( scope = 'project'
               OR (scope = 'branch' AND scope_key = ?2)
               OR (scope = 'task'   AND scope_key = ?3) )
          ORDER BY CASE scope WHEN 'task' THEN 0 WHEN 'branch' THEN 1 ELSE 2 END,
                   importance DESC, id",
    )
    .bind(project_id.to_string())
    .bind(branch)
    .bind(task_id.map(|t| t.to_string()).unwrap_or_default())
    .fetch_all(store.pool())
    .await?;

    rows.iter()
        .map(|r| {
            let verification: Option<String> = r.try_get("verification").ok();
            Ok(cairn_core::wire::PinnedConstraint {
                id: rows::uuid(r, "id")?,
                text: r.try_get("content")?,
                drifted: verification.as_deref() == Some("drifted"),
            })
        })
        .collect()
}

/// The summaries of a session's `error` observations, most recent first.
///
/// One of the two sources of the signals a pattern is matched against. Read
/// from what Cairn already recorded rather than asked of the agent: an agent
/// reporting its own symptoms would be reporting them after having read
/// whatever was suggested last time (FR-398).
///
/// The previous session is included, because a symptom is often recorded in the
/// session that hit it and worked on in the next one.
pub async fn recent_error_summaries(
    store: &Store,
    session_id: Uuid,
    limit: i64,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT summary FROM observations
          WHERE type = 'error'
            AND session_id IN (
                ?1,
                (SELECT previous_session_id FROM sessions WHERE id = ?1)
            )
          ORDER BY occurred_at DESC
          LIMIT ?2",
    )
    .bind(session_id.to_string())
    .bind(limit)
    .fetch_all(store.pool())
    .await?)
}

/// The text of `failure`-type memories in the applicable scopes.
///
/// The other signal source. Project- and branch-scoped only: a task-scoped
/// failure belongs to work that may have nothing to do with what is happening
/// now, and widening the read would widen what a pattern matches on.
pub async fn failure_memory_text(
    store: &Store,
    project_id: Uuid,
    branch: &str,
    limit: i64,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT content FROM memories
          WHERE project_id = ?1 AND type = 'failure' AND state = 'active'
            AND deleted_at IS NULL
            AND (scope = 'project' OR (scope = 'branch' AND scope_key = ?2))
          ORDER BY updated_at DESC
          LIMIT ?3",
    )
    .bind(project_id.to_string())
    .bind(branch)
    .bind(limit)
    .fetch_all(store.pool())
    .await?)
}

/// How many of a project's most recent sessions supply signals.
///
/// The contract scopes signals to "the current and previous session". The
/// session-independent read keeps that bound rather than reading the project's
/// whole history: a failure from months ago is not what this project is showing
/// now, and suggesting a pattern for it would be worse than suggesting nothing
/// (`contracts/patterns.md` §Suggestion).
pub const SIGNAL_SESSIONS: i64 = 2;

/// A project's recent `error` observations, most recent first.
///
/// The session-independent form of [`recent_error_summaries`]. `cairn memory
/// search --include-patterns` is a developer command and must not require an
/// open agent session to answer — but the signals it matches on are still what
/// Cairn recorded, never what the caller asserts, and still only from what this
/// project is currently working through.
pub async fn recent_project_errors(
    store: &Store,
    project_id: Uuid,
    limit: i64,
) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT o.summary FROM observations o
          WHERE o.type = 'error'
            AND o.session_id IN (
                SELECT id FROM sessions
                 WHERE project_id = ?1 AND deleted_at IS NULL
                 ORDER BY started_at DESC
                 LIMIT ?3
            )
          ORDER BY o.occurred_at DESC
          LIMIT ?2",
    )
    .bind(project_id.to_string())
    .bind(limit)
    .bind(SIGNAL_SESSIONS)
    .fetch_all(store.pool())
    .await?)
}

/// How far the subject mechanism actually reaches in this project, and what it
/// is currently reporting (FR-499).
///
/// Observable in **every** project without anyone running an evaluation: an
/// adoption rate nobody can see is one nobody acts on, and the whole design
/// rests on agents choosing to give facts a subject. A low share is a product
/// finding about the usage contract and the tool descriptions — never a licence
/// to start inferring subjects from content, which D46 rejects on correctness
/// grounds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubjectAdoption {
    /// Project-scoped, non-deleted memories.
    pub project_memories: i64,
    /// How many of them carry a subject identity.
    pub with_subject: i64,
    pub conflicted_subjects: i64,
    pub needs_recheck: i64,
    pub drifted: i64,
}

impl SubjectAdoption {
    /// Whole percent, or `None` when there is nothing to divide by.
    ///
    /// `None` rather than zero: a project with no project-scoped memories has
    /// no adoption rate, and reporting 0% would read as a failure where there
    /// is nothing to have adopted.
    pub fn percent(&self) -> Option<i64> {
        (self.project_memories > 0).then(|| self.with_subject * 100 / self.project_memories)
    }
}

pub async fn subject_adoption(store: &Store, project_id: Uuid) -> Result<SubjectAdoption> {
    let one = |sql: &'static str| {
        let id = project_id.to_string();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(id)
                .fetch_one(store.pool())
                .await
        }
    };

    Ok(SubjectAdoption {
        project_memories: one("SELECT COUNT(*) FROM memories
              WHERE project_id = ?1 AND scope = 'project' AND deleted_at IS NULL")
        .await?,
        with_subject: one("SELECT COUNT(*) FROM memories
              WHERE project_id = ?1 AND scope = 'project' AND deleted_at IS NULL
                AND topic_key IS NOT NULL")
        .await?,
        // Counted by subject, not by memory: one disagreement between four
        // proposals is one thing to resolve, not four.
        conflicted_subjects: one("SELECT COUNT(*) FROM (
                 SELECT scope, scope_key, topic_key FROM memories
                  WHERE project_id = ?1 AND deleted_at IS NULL AND topic_key IS NOT NULL
                    AND state = 'active'
                  GROUP BY scope, scope_key, topic_key
                 HAVING COUNT(DISTINCT value_key) > 1
             ) conflicted")
        .await?,
        needs_recheck: one("SELECT COUNT(*) FROM memories
              WHERE project_id = ?1 AND deleted_at IS NULL AND verification = 'needs_recheck'")
        .await?,
        drifted: one("SELECT COUNT(*) FROM memories
              WHERE project_id = ?1 AND deleted_at IS NULL AND verification = 'drifted'")
        .await?,
    })
}

#[cfg(test)]
mod writer_identity_tests {
    use super::*;
    use crate::Store;

    /// Migration 7's finish hook seeds the singleton row; this is only
    /// asserting the reader sees it, not creating it.
    #[tokio::test]
    async fn writer_identity_is_present_and_stable_after_migration() {
        let store = Store::open_memory().await.unwrap();
        let first = writer_identity(&store).await.unwrap();
        let second = writer_identity(&store).await.unwrap();
        assert_eq!(
            first, second,
            "the same store must always read the same writer_id"
        );
    }

    /// A fresh writer's first personal record is sequence 1, and it climbs by
    /// one per record — never compared to any other writer's stream.
    #[tokio::test]
    async fn personal_writer_seq_climbs_from_one_and_never_repeats() {
        let store = Store::open_memory().await.unwrap();
        let writer = writer_identity(&store).await.unwrap();

        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        let first = next_personal_writer_seq(&mut tx, writer).await.unwrap();
        assert_eq!(first, 1);
        insert_personal_row(&mut tx, writer, first).await;
        tx.commit().await.unwrap();

        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        let second = next_personal_writer_seq(&mut tx, writer).await.unwrap();
        assert_eq!(second, 2);
        tx.commit().await.unwrap();
    }

    /// Personal and team are separate sequence spaces for the same writer
    /// (`personal_knowledge_writer_seq` and `team_knowledge_writer_seq` are two
    /// distinct unique indexes, over two distinct tables) — a personal write
    /// must not advance the team counter or vice versa.
    #[tokio::test]
    async fn personal_and_team_sequences_are_independent_for_the_same_writer() {
        let store = Store::open_memory().await.unwrap();
        let writer = writer_identity(&store).await.unwrap();

        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        let personal_first = next_personal_writer_seq(&mut tx, writer).await.unwrap();
        insert_personal_row(&mut tx, writer, personal_first).await;
        tx.commit().await.unwrap();

        // The team stream has had no writes at all yet, so it still starts at 1
        // regardless of how far the personal stream has climbed.
        let mut tx = crate::tx::begin(&store, "test").await.unwrap();
        let team_first = next_team_writer_seq(&mut tx, writer).await.unwrap();
        assert_eq!(
            team_first, 1,
            "an unrelated domain's counter must not inherit another's progress"
        );
        tx.commit().await.unwrap();
    }

    async fn insert_personal_row(tx: &mut sqlx::SqliteConnection, writer: Uuid, seq: i64) {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO personal_knowledge
                (id, owner_user_id, knowledge_type, content, writer_id, writer_seq, created_at)
             VALUES (?1, ?2, 'fact', 'a personal note', ?3, ?4, ?5)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(Uuid::now_v7().to_string())
        .bind(writer.to_string())
        .bind(seq)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// Import: the server-wins refill (T086)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod import_tests {
    use super::*;
    use crate::Store;

    async fn seed_project(store: &Store) -> Uuid {
        let id = Uuid::now_v7();
        let now = rows::now_text();
        sqlx::query(
            "INSERT INTO projects
               (id, name, git_common_dir, repository_remote, linked,
                server_project_id, created_at, updated_at, deleted_at)
             VALUES (?1, 'test', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
        )
        .bind(id.to_string())
        .bind(format!("/tmp/git-{id}"))
        .bind(&now)
        .execute(store.pool())
        .await
        .unwrap();
        id
    }

    fn arriving(id: Uuid, project: Uuid, content: &str) -> ImportedMemory<'_> {
        ImportedMemory {
            id,
            project_id: project,
            kind: MemoryType::Fact,
            scope: MemoryScope::Project,
            scope_key: "",
            content,
            origin_session_id: Uuid::now_v7(),
            topic_key: None,
            value_key: None,
            importance: Importance::Normal,
            effective_from: None,
        }
    }

    async fn content_of(store: &Store, id: Uuid) -> String {
        sqlx::query_scalar("SELECT content FROM memories WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(store.pool())
            .await
            .unwrap()
    }

    /// FR-712a, the whole point of the change: a second arrival carrying
    /// corrected content replaces what is stored.
    ///
    /// Under the `INSERT OR IGNORE` this replaced, the second call was a no-op
    /// and this store recalled the uncorrected text forever.
    #[tokio::test]
    async fn a_second_arrival_replaces_the_cached_content() {
        let store = Store::open_memory().await.unwrap();
        let project = seed_project(&store).await;
        let id = Uuid::now_v7();

        assert!(
            import_memory(&store, arriving(id, project, "the timeout is 30s"))
                .await
                .unwrap()
        );
        assert!(
            import_memory(&store, arriving(id, project, "the timeout is 5s"))
                .await
                .unwrap()
        );

        assert_eq!(
            content_of(&store, id).await,
            "the timeout is 5s",
            "a correction the cache cannot accept never reaches the reader"
        );
        let digest: Option<String> =
            sqlx::query_scalar("SELECT content_norm_digest FROM memories WHERE id = ?1")
                .bind(id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(
            digest.as_deref(),
            Some(cairn_core::knowledge::content_norm_digest("the timeout is 5s").as_str()),
            "the digest follows the content it indexes"
        );
    }

    /// FR-706. A local-only memory never went to the server, so nothing can
    /// legitimately come back bearing its id — and if something does, the row
    /// the user deliberately kept off the network is left exactly as it is.
    #[tokio::test]
    async fn a_local_only_row_is_never_refreshed() {
        let store = Store::open_memory().await.unwrap();
        let project = seed_project(&store).await;
        let id = Uuid::now_v7();

        import_memory(&store, arriving(id, project, "kept on this machine"))
            .await
            .unwrap();
        sqlx::query("UPDATE memories SET local_only = 1 WHERE id = ?1")
            .bind(id.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let landed = import_memory(
            &store,
            arriving(id, project, "overwritten from the network"),
        )
        .await
        .unwrap();
        assert!(!landed, "the guard must report that nothing was applied");
        assert_eq!(
            content_of(&store, id).await,
            "kept on this machine",
            "a local-only row must not be overwritten by content that crossed the network"
        );
    }

    /// FR-052. A deletion cleared the content and set `deleted_at`; an arriving
    /// copy must not write the content back into the tombstone.
    #[tokio::test]
    async fn a_deleted_row_is_never_resurrected_by_an_import() {
        let store = Store::open_memory().await.unwrap();
        let project = seed_project(&store).await;
        let id = Uuid::now_v7();

        import_memory(&store, arriving(id, project, "since deleted"))
            .await
            .unwrap();
        sqlx::query("UPDATE memories SET content = '', deleted_at = ?2 WHERE id = ?1")
            .bind(id.to_string())
            .bind(rows::now_text())
            .execute(store.pool())
            .await
            .unwrap();

        let landed = import_memory(&store, arriving(id, project, "since deleted"))
            .await
            .unwrap();
        assert!(!landed, "the guard must report that nothing was applied");
        assert_eq!(
            content_of(&store, id).await,
            "",
            "a tombstone must stay a tombstone"
        );
    }

    /// The derived columns are rebuilt from this store's own relations and
    /// runs, so a refresh must leave every one of them alone. This is the
    /// assertion that would catch a future `DO UPDATE` growing a column.
    #[tokio::test]
    async fn a_refresh_leaves_the_locally_derived_columns_alone() {
        let store = Store::open_memory().await.unwrap();
        let project = seed_project(&store).await;
        let id = Uuid::now_v7();

        import_memory(&store, arriving(id, project, "first"))
            .await
            .unwrap();
        let created_at: String =
            sqlx::query_scalar("SELECT created_at FROM memories WHERE id = ?1")
                .bind(id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        sqlx::query(
            "UPDATE memories
                SET state = 'superseded', reinforcement_count = 4,
                    distinct_origin_count = 3, verification = 'remote_attested',
                    pinned = 1
              WHERE id = ?1",
        )
        .bind(id.to_string())
        .execute(store.pool())
        .await
        .unwrap();

        import_memory(&store, arriving(id, project, "second"))
            .await
            .unwrap();

        let after: (String, i64, i64, String, i64, String) = sqlx::query_as(
            "SELECT state, reinforcement_count, distinct_origin_count, verification,
                    pinned, created_at
               FROM memories WHERE id = ?1",
        )
        .bind(id.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(after.0, "superseded", "state is a view of the relations");
        assert_eq!(after.1, 4, "reinforcement is counted locally");
        assert_eq!(after.2, 3, "distinct origins are counted locally");
        assert_eq!(
            after.3, "remote_attested",
            "verification is rebuilt from the runs this store has"
        );
        assert_eq!(after.4, 1, "a pin is this project's own attention decision");
        assert_eq!(
            after.5, created_at,
            "the row is as old as its first arrival here"
        );
        assert_eq!(content_of(&store, id).await, "second");
    }
}

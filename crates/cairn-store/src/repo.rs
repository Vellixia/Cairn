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

pub async fn create_task(
    store: &Store,
    project_id: Uuid,
    title: &str,
    goal: &str,
    criteria: &[String],
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

    let created = Task {
        id,
        project_id,
        title: title.to_string(),
        goal: goal.to_string(),
        acceptance_criteria: criteria.to_vec(),
        status: TaskStatus::Todo,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };
    // Same transaction as the change it describes (D9).
    outbox::enqueue(
        &mut *tx,
        policy,
        project_id,
        OutboxEntityType::Task,
        id,
        OutboxOperation::Upsert,
        &outbox::task_payload(&created),
    )
    .await?;
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
/// and simply recorded (FR-037); there is no revision history (FR-039).
pub async fn update_task(
    store: &Store,
    id: Uuid,
    title: Option<&str>,
    goal: Option<&str>,
    criteria: Option<&[String]>,
    status: Option<TaskStatus>,
    policy: SyncPolicy,
) -> Result<Task> {
    let current = task(store, id).await?;
    sqlx::query(
        "UPDATE tasks SET title = ?1, goal = ?2, acceptance_criteria = ?3, status = ?4,
                          updated_at = ?5
         WHERE id = ?6",
    )
    .bind(title.unwrap_or(&current.title))
    .bind(goal.unwrap_or(&current.goal))
    .bind(
        serde_json::to_string(criteria.unwrap_or(&current.acceptance_criteria))
            .unwrap_or_else(|_| "[]".into()),
    )
    .bind(status.unwrap_or(current.status).as_str())
    .bind(rows::now_text())
    .bind(id.to_string())
    .execute(store.pool())
    .await?;

    let updated = task(store, id).await?;
    let mut tx = tx::begin(store, "update_task").await?;
    outbox::enqueue(
        &mut *tx,
        policy,
        updated.project_id,
        OutboxEntityType::Task,
        id,
        OutboxOperation::Upsert,
        &outbox::task_payload(&updated),
    )
    .await?;
    tx::commit(tx, "update_task").await?;
    Ok(updated)
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
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, NULL, ?11, NULL, ?12, NULL)",
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

pub async fn bind_task(store: &Store, id: Uuid, task_id: Uuid) -> Result<Session> {
    sqlx::query("UPDATE sessions SET task_id = ?1, last_event_at = ?2 WHERE id = ?3")
        .bind(task_id.to_string())
        .bind(rows::now_text())
        .bind(id.to_string())
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
}

pub async fn create_memory(store: &Store, m: NewMemory<'_>, policy: SyncPolicy) -> Result<Memory> {
    let id = new_id();
    let now = rows::now_text();
    let mut tx = tx::begin(store, "create_memory").await?;
    sqlx::query(
        "INSERT INTO memories
            (id, project_id, type, scope, scope_key, content, state, superseded_by_id,
             origin_session_id, local_only, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', NULL, ?7, ?8, ?9, ?9)",
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
        outbox::enqueue(
            &mut *tx,
            policy,
            m.project_id,
            OutboxEntityType::Memory,
            id,
            OutboxOperation::Upsert,
            &outbox::memory_payload(&staged),
        )
        .await?;
    }

    tx::commit(tx, "create_memory").await?;
    memory(store, id).await
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
    let new = create_memory(store, replacement, policy).await?;
    sqlx::query(
        "UPDATE memories SET state = 'superseded', superseded_by_id = ?1, updated_at = ?2
         WHERE id = ?3",
    )
    .bind(new.id.to_string())
    .bind(rows::now_text())
    .bind(original.id.to_string())
    .execute(store.pool())
    .await?;

    let updated = memory(store, original_id).await?;
    if !updated.local_only {
        let mut tx = tx::begin(store, "supersede_memory").await?;
        outbox::enqueue(
            &mut *tx,
            policy,
            updated.project_id,
            OutboxEntityType::Memory,
            updated.id,
            OutboxOperation::Upsert,
            &outbox::memory_payload(&updated),
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
            sqlx::query("UPDATE memories SET state = 'stale', updated_at = ?1 WHERE id = ?2")
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

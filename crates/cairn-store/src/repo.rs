//! Edge binding and hook/session correlation. Canonical knowledge lives server-side.

use crate::{rows, tx, Result, Store, StoreError};
use cairn_core::domain::{new_id, Project, Session, SessionStatus};
use uuid::Uuid;

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

pub async fn ensure_project(
    store: &Store,
    git_common_dir: &str,
    name: &str,
    remote: Option<&str>,
) -> Result<Project> {
    if let Some(project) = project_by_common_dir(store, git_common_dir).await? {
        if project.repository_remote.as_deref() == remote {
            return Ok(project);
        }
        sqlx::query("UPDATE projects SET repository_remote = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(remote)
            .bind(rows::now_text())
            .bind(project.id.to_string())
            .execute(store.pool())
            .await?;
        return project_by_common_dir(store, git_common_dir)
            .await?
            .ok_or_else(|| StoreError::NotFound(format!("project for {git_common_dir}")));
    }
    let now = rows::now_text();
    sqlx::query("INSERT INTO projects (id, name, git_common_dir, repository_remote, linked, server_project_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, NULL, ?5, ?5) ON CONFLICT (git_common_dir) DO NOTHING")
        .bind(new_id().to_string()).bind(name).bind(git_common_dir).bind(remote).bind(now)
        .execute(store.pool()).await?;
    project_by_common_dir(store, git_common_dir)
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("project for {git_common_dir}")))
}

async fn project_by_common_dir(store: &Store, dir: &str) -> Result<Option<Project>> {
    let row =
        sqlx::query("SELECT * FROM projects WHERE git_common_dir = ?1 AND deleted_at IS NULL")
            .bind(dir)
            .fetch_optional(store.pool())
            .await?;
    row.as_ref().map(rows::project).transpose()
}

pub async fn project(store: &Store, id: Uuid) -> Result<Project> {
    let row = sqlx::query("SELECT * FROM projects WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(store.pool())
        .await?
        .ok_or_else(|| StoreError::NotFound(format!("project {id}")))?;
    rows::project(&row)
}

pub async fn has_active_sessions(store: &Store) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE status = 'active' AND deleted_at IS NULL)",
    )
    .fetch_one(store.pool())
    .await?
        != 0)
}

pub struct StartSession<'a> {
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub agent: &'a str,
    pub agent_session_key: &'a str,
    pub branch: &'a str,
    pub commit_sha: Option<&'a str>,
    pub worktree_path: &'a str,
    pub daemon_run_id: Uuid,
}

pub async fn start_session(store: &Store, input: StartSession<'_>) -> Result<Session> {
    if let Some(existing) = session_by_key(store, input.project_id, input.agent_session_key).await?
    {
        return if existing.status == SessionStatus::Active {
            Ok(existing)
        } else {
            resume_session(store, existing.id, input.daemon_run_id).await
        };
    }
    let previous: Option<String> = sqlx::query_scalar("SELECT id FROM sessions WHERE project_id = ?1 AND branch = ?2 AND status != 'active' AND deleted_at IS NULL ORDER BY ended_at DESC, id DESC LIMIT 1")
        .bind(input.project_id.to_string()).bind(input.branch).fetch_optional(store.pool()).await?;
    let id = new_id();
    let now = rows::now_text();
    let mut transaction = tx::begin(store, "start_session").await?;
    sqlx::query("INSERT INTO sessions (id, project_id, user_id, agent, branch, commit_sha, worktree_path, agent_session_key, previous_session_id, status, started_at, ended_at, last_event_at, last_turn_ended_at, daemon_run_id, end_reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, NULL, ?10, NULL, ?11, NULL) ON CONFLICT DO NOTHING")
        .bind(id.to_string()).bind(input.project_id.to_string()).bind(input.user_id.to_string())
        .bind(input.agent).bind(input.branch).bind(input.commit_sha).bind(input.worktree_path)
        .bind(input.agent_session_key).bind(previous).bind(&now).bind(input.daemon_run_id.to_string())
        .execute(&mut *transaction).await?;
    tx::commit(transaction, "start_session").await?;
    session_by_key(store, input.project_id, input.agent_session_key)
        .await?
        .ok_or_else(|| StoreError::NotFound("started session".into()))
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
    let row = sqlx::query("SELECT * FROM sessions WHERE project_id = ?1 AND agent_session_key = ?2 AND deleted_at IS NULL")
        .bind(project_id.to_string()).bind(key).fetch_optional(store.pool()).await?;
    row.as_ref().map(rows::session).transpose()
}

pub async fn list_sessions(store: &Store, project_id: Uuid) -> Result<Vec<Session>> {
    let rows = sqlx::query("SELECT * FROM sessions WHERE project_id = ?1 AND deleted_at IS NULL ORDER BY started_at DESC")
        .bind(project_id.to_string()).fetch_all(store.pool()).await?;
    rows.iter().map(rows::session).collect()
}

pub async fn active_sessions_in_worktree(
    store: &Store,
    project_id: Uuid,
    worktree: &str,
) -> Result<Vec<Session>> {
    let rows = sqlx::query("SELECT * FROM sessions WHERE project_id = ?1 AND worktree_path = ?2 AND status = 'active' AND deleted_at IS NULL ORDER BY started_at DESC")
        .bind(project_id.to_string()).bind(worktree).fetch_all(store.pool()).await?;
    rows.iter().map(rows::session).collect()
}

pub async fn turn_checkpoint(store: &Store, id: Uuid) -> Result<Session> {
    let now = rows::now_text();
    sqlx::query("UPDATE sessions SET last_turn_ended_at = ?1, last_event_at = ?1 WHERE id = ?2")
        .bind(&now)
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
) -> Result<Session> {
    let now = rows::now_text();
    sqlx::query("UPDATE sessions SET status = ?1, ended_at = ?2, last_event_at = ?2, end_reason = ?3 WHERE id = ?4")
        .bind(status.as_str()).bind(&now).bind(reason).bind(id.to_string()).execute(store.pool()).await?;
    session(store, id).await
}

pub async fn resume_session(store: &Store, id: Uuid, daemon_run_id: Uuid) -> Result<Session> {
    sqlx::query("UPDATE sessions SET status = 'active', ended_at = NULL, end_reason = NULL, daemon_run_id = ?1, last_event_at = ?2 WHERE id = ?3")
        .bind(daemon_run_id.to_string()).bind(rows::now_text()).bind(id.to_string())
        .execute(store.pool()).await?;
    session(store, id).await
}

pub async fn seal_session(
    store: &Store,
    id: Uuid,
    status: SessionStatus,
    reason: Option<&str>,
) -> Result<Session> {
    let now = rows::now_text();
    sqlx::query("UPDATE sessions SET status = ?1, ended_at = ?2, last_event_at = ?2, end_reason = ?3, handoff_pending = 1, handoff_attempts = 0, handoff_error = NULL WHERE id = ?4")
        .bind(status.as_str()).bind(&now).bind(reason).bind(id.to_string()).execute(store.pool()).await?;
    session(store, id).await
}

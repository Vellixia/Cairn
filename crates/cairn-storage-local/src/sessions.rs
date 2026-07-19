//! T013: sessions DAO. State changes only through cairn-domain's legal
//! transition set; `recovering_since` is written once and never overwritten.

use cairn_domain::{transition, SessionState, TransitionReason};
use sqlx::{Executor, Sqlite, SqlitePool};

use crate::db::StorageError;
use crate::records::SessionRow;

pub async fn get_by_id<'e, E>(exec: E, id: &str) -> Result<Option<SessionRow>, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    Ok(
        sqlx::query_as::<_, SessionRow>("SELECT * FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_optional(exec)
            .await?,
    )
}

/// The live (active|recovering) session for one agent instance in one repo.
pub async fn get_live_for_instance<'e, E>(
    exec: E,
    repository_id: &str,
    agent_instance_id: &str,
) -> Result<Option<SessionRow>, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    Ok(sqlx::query_as::<_, SessionRow>(
        "SELECT * FROM sessions WHERE repository_id = ? AND agent_instance_id = ? \
         AND state IN ('active','recovering')",
    )
    .bind(repository_id)
    .bind(agent_instance_id)
    .fetch_optional(exec)
    .await?)
}

pub async fn list(
    pool: &SqlitePool,
    repository_id: Option<&str>,
    states: Option<&[SessionState]>,
) -> Result<Vec<SessionRow>, StorageError> {
    let mut sql = String::from("SELECT * FROM sessions WHERE 1=1");
    if repository_id.is_some() {
        sql.push_str(" AND repository_id = ?");
    }
    if let Some(states) = states {
        let placeholders = vec!["?"; states.len()].join(",");
        sql.push_str(&format!(" AND state IN ({placeholders})"));
    }
    sql.push_str(" ORDER BY started_at DESC");

    let mut q = sqlx::query_as::<_, SessionRow>(&sql);
    if let Some(r) = repository_id {
        q = q.bind(r.to_string());
    }
    if let Some(states) = states {
        for s in states {
            q = q.bind(s.as_str());
        }
    }
    Ok(q.fetch_all(pool).await?)
}

pub async fn list_by_state(
    pool: &SqlitePool,
    state: SessionState,
) -> Result<Vec<SessionRow>, StorageError> {
    Ok(
        sqlx::query_as::<_, SessionRow>("SELECT * FROM sessions WHERE state = ?")
            .bind(state.as_str())
            .fetch_all(pool)
            .await?,
    )
}

pub async fn count_by_state(pool: &SqlitePool, state: SessionState) -> Result<u64, StorageError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions WHERE state = ?")
        .bind(state.as_str())
        .fetch_one(pool)
        .await?;
    Ok(n as u64)
}

pub async fn insert<'e, E>(exec: E, row: &SessionRow) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query(
        "INSERT INTO sessions (id, repository_id, worktree_id, local_user, agent_type, \
         agent_instance_id, agent_pid, resume_token_hash, lease_expires_at, state, \
         start_snapshot_id, current_snapshot_id, started_at, ended_at, last_heartbeat_at, \
         recovering_since) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.id)
    .bind(&row.repository_id)
    .bind(&row.worktree_id)
    .bind(&row.local_user)
    .bind(&row.agent_type)
    .bind(&row.agent_instance_id)
    .bind(row.agent_pid)
    .bind(&row.resume_token_hash)
    .bind(&row.lease_expires_at)
    .bind(&row.state)
    .bind(&row.start_snapshot_id)
    .bind(&row.current_snapshot_id)
    .bind(&row.started_at)
    .bind(&row.ended_at)
    .bind(&row.last_heartbeat_at)
    .bind(&row.recovering_since)
    .execute(exec)
    .await?;
    Ok(())
}

/// Apply a legal state transition. `recovering_since` is set only when
/// entering recovering AND currently NULL (analysis A2); cleared on reattach.
#[allow(clippy::too_many_arguments)]
pub async fn apply_transition<'e, E>(
    exec: E,
    session_id: &str,
    from: SessionState,
    to: SessionState,
    reason: TransitionReason,
    now_rfc3339: &str,
) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    transition(from, to, reason).map_err(|e| StorageError::IllegalTransition(e.to_string()))?;

    let ended = matches!(to, SessionState::Stopped | SessionState::Interrupted);
    let sql = match to {
        SessionState::Recovering => {
            "UPDATE sessions SET state = ?, \
             recovering_since = COALESCE(recovering_since, ?) \
             WHERE id = ? AND state = ?"
        }
        SessionState::Active => {
            "UPDATE sessions SET state = ?, recovering_since = NULL, last_heartbeat_at = ? \
             WHERE id = ? AND state = ?"
        }
        _ if ended => "UPDATE sessions SET state = ?, ended_at = ? WHERE id = ? AND state = ?",
        _ => "UPDATE sessions SET state = ?, last_heartbeat_at = ? WHERE id = ? AND state = ?",
    };
    let res = sqlx::query(sql)
        .bind(to.as_str())
        .bind(now_rfc3339)
        .bind(session_id)
        .bind(from.as_str())
        .execute(exec)
        .await?;
    if res.rows_affected() == 0 {
        return Err(StorageError::IllegalTransition(format!(
            "session {session_id} not in expected state {}",
            from.as_str()
        )));
    }
    Ok(())
}

pub async fn update_heartbeat<'e, E>(
    exec: E,
    session_id: &str,
    heartbeat_at: &str,
    lease_expires_at: &str,
) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("UPDATE sessions SET last_heartbeat_at = ?, lease_expires_at = ? WHERE id = ?")
        .bind(heartbeat_at)
        .bind(lease_expires_at)
        .bind(session_id)
        .execute(exec)
        .await?;
    Ok(())
}

pub async fn update_current_snapshot<'e, E>(
    exec: E,
    session_id: &str,
    snapshot_id: &str,
) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("UPDATE sessions SET current_snapshot_id = ? WHERE id = ?")
        .bind(snapshot_id)
        .bind(session_id)
        .execute(exec)
        .await?;
    Ok(())
}

pub async fn update_resume_token_hash<'e, E>(
    exec: E,
    session_id: &str,
    resume_token_hash: &str,
    lease_expires_at: &str,
) -> Result<(), StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("UPDATE sessions SET resume_token_hash = ?, lease_expires_at = ? WHERE id = ?")
        .bind(resume_token_hash)
        .bind(lease_expires_at)
        .bind(session_id)
        .execute(exec)
        .await?;
    Ok(())
}

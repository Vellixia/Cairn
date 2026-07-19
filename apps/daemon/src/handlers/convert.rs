//! Row → DTO conversion.

use std::str::FromStr;

use cairn_domain::{AgentInstanceId, SessionId, SessionState, SnapshotId, Timestamp};
use cairn_protocol::{
    RemoteDto, RepositoryDto, SessionDto, SessionSummaryDto, SnapshotDto, WorktreeDto,
};
use cairn_storage_local::{RepositoryRow, SessionRow, SnapshotRow, WorktreeRow};

use super::{HandlerError, HandlerResult};
use cairn_protocol::ErrorCode;

fn ts(s: &str) -> HandlerResult<Timestamp> {
    Timestamp::parse(s)
        .map_err(|e| HandlerError::new(ErrorCode::Internal, format!("bad timestamp: {e}")))
}

pub fn repository_dto(row: &RepositoryRow) -> HandlerResult<RepositoryDto> {
    Ok(RepositoryDto {
        repository_id: row.id.clone(),
        repo_uuid: cairn_domain::RepoUuid::from_str(&row.repo_uuid)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        canonical_path: row.canonical_path.clone(),
        default_remote: match (&row.default_remote_name, &row.default_remote_url) {
            (Some(n), Some(u)) => Some(RemoteDto {
                name: n.clone(),
                url: u.clone(),
            }),
            _ => None,
        },
        copied_from_repository_id: row.copied_from_repository_id.clone(),
        registered_at: ts(&row.registered_at)?,
    })
}

pub fn worktree_dto(row: &WorktreeRow) -> HandlerResult<WorktreeDto> {
    Ok(WorktreeDto {
        worktree_id: row.id.clone(),
        worktree_uuid: cairn_domain::WorktreeUuid::from_str(&row.worktree_uuid)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        path: row.path.clone(),
        is_main: row.is_main != 0,
    })
}

pub fn snapshot_dto(row: &SnapshotRow) -> HandlerResult<SnapshotDto> {
    Ok(SnapshotDto {
        snapshot_id: SnapshotId::from_str(&row.id)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        branch: row.branch.clone(),
        detached: row.branch.is_none(),
        head_commit: row.head_commit.clone(),
        staged_fp: row.staged_fp.clone(),
        unstaged_fp: row.unstaged_fp.clone(),
        untracked_fp: row.untracked_fp.clone(),
        snapshot_fp: row.snapshot_fp.clone(),
        created_at: ts(&row.created_at)?,
    })
}

pub fn session_dto(
    row: &SessionRow,
    start_snapshot: &SnapshotRow,
    current_snapshot: &SnapshotRow,
) -> HandlerResult<SessionDto> {
    Ok(SessionDto {
        session_id: SessionId::from_str(&row.id)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        repository_id: row.repository_id.clone(),
        worktree_id: row.worktree_id.clone(),
        local_user: row.local_user.clone(),
        agent_type: row.agent_type.clone(),
        agent_instance_id: AgentInstanceId::from_str(&row.agent_instance_id)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        state: SessionState::parse(&row.state)
            .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "bad state"))?,
        start_snapshot: snapshot_dto(start_snapshot)?,
        current_snapshot: snapshot_dto(current_snapshot)?,
        started_at: ts(&row.started_at)?,
        ended_at: row.ended_at.as_deref().map(ts).transpose()?,
        last_heartbeat_at: ts(&row.last_heartbeat_at)?,
        lease_expires_at: ts(&row.lease_expires_at)?,
        recovering_since: row.recovering_since.as_deref().map(ts).transpose()?,
    })
}

pub fn session_summary_dto(row: &SessionRow) -> HandlerResult<SessionSummaryDto> {
    Ok(SessionSummaryDto {
        session_id: SessionId::from_str(&row.id)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        agent_type: row.agent_type.clone(),
        agent_instance_id: AgentInstanceId::from_str(&row.agent_instance_id)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        state: SessionState::parse(&row.state)
            .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "bad state"))?,
        started_at: ts(&row.started_at)?,
    })
}

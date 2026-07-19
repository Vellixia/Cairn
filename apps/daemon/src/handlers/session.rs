//! T036/T049: session IPC handlers wiring the lifecycle service; watcher
//! start/stop rides on session lifecycle (US3).

use cairn_protocol::*;
use cairn_session::{SessionError, StartOutcome as SvcOutcome};
use cairn_storage_local::{sessions as sdao, snapshots, worktrees, SessionRow};

use super::{convert, HandlerError, HandlerResult};
use crate::state::AppState;
use crate::watch::WatchCommand;

async fn dto_for(state: &AppState, row: &SessionRow) -> HandlerResult<SessionDto> {
    let start = snapshots::get_by_id(state.pool(), &row.start_snapshot_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "missing start snapshot"))?;
    let current = snapshots::get_by_id(state.pool(), &row.current_snapshot_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "missing current snapshot"))?;
    convert::session_dto(row, &start, &current)
}

async fn watch_ready(
    state: &AppState,
    repository_id: &str,
    worktree_id: &str,
    root: &str,
) -> Result<(), crate::watch::WatchStartFailure> {
    crate::watch::request_ready(
        state,
        repository_id.to_string(),
        worktree_id.to_string(),
        root.into(),
    )
    .await
}

async fn maybe_unwatch(state: &AppState, worktree_id: &str) {
    let live = [
        cairn_domain::SessionState::Active,
        cairn_domain::SessionState::Recovering,
    ];
    let rows = sdao::list(state.pool(), None, Some(&live))
        .await
        .unwrap_or_default();
    if !rows.iter().any(|r| r.worktree_id == worktree_id) {
        let _ = state.inner.watch_tx.send(WatchCommand::Unwatch {
            worktree_id: worktree_id.to_string(),
        });
    }
}

pub async fn start(
    state: &AppState,
    params: SessionStartParams,
) -> HandlerResult<SessionStartResult> {
    let (repo, worktree, layout) = super::repository::resolve_registered(
        state,
        params.path.as_deref(),
        params.repository_id.as_deref(),
    )
    .await?;
    let (snapshot, _) =
        super::snapshot::ensure_snapshot(state, &repo.id, &worktree.id, &layout.worktree_root)
            .await?;

    let local_user = whoami::username();
    let result = state
        .inner
        .sessions
        .start(
            &repo.id,
            &worktree.id,
            &local_user,
            &params.agent_type,
            &params.agent_instance_id.to_string(),
            params.agent_pid.map(i64::from),
            &snapshot,
        )
        .await?;

    if let Err(failure) = watch_ready(state, &repo.id, &worktree.id, &worktree.path).await {
        state
            .inner
            .sessions
            .interrupt_watcher_start(&result.session, failure.stage)
            .await?;
        maybe_unwatch(state, &worktree.id).await;
        return Err(HandlerError::new(
            ErrorCode::WatcherStartFailed,
            "session watcher readiness failed",
        )
        .with_data(ErrorData::watcher_start_failure(failure.stage)));
    }

    let ready_session = sdao::get_by_id(state.pool(), &result.session.id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "session row vanished"))?;

    let outcome = match result.outcome {
        SvcOutcome::Created => StartOutcome::Created,
        SvcOutcome::Existing => StartOutcome::Existing,
        SvcOutcome::Takeover => StartOutcome::Takeover,
    };
    Ok(SessionStartResult {
        session: dto_for(state, &ready_session).await?,
        resume_token: result.resume_token,
        outcome,
    })
}

pub async fn get(state: &AppState, params: SessionGetParams) -> HandlerResult<SessionGetResult> {
    // Resolve repository scope when a path was provided.
    let repository_id = match (&params.repository_id, &params.path) {
        (Some(id), _) => Some(id.clone()),
        (None, Some(path)) => {
            let (repo, _, _) =
                super::repository::resolve_registered(state, Some(path), None).await?;
            Some(repo.id)
        }
        (None, None) => None,
    };

    let outcome = state
        .inner
        .sessions
        .get(
            repository_id.as_deref(),
            params.session_id.map(|s| s.to_string()).as_deref(),
            params.agent_instance_id.map(|s| s.to_string()).as_deref(),
            params.agent_type.as_deref(),
        )
        .await?;

    match outcome {
        Ok(row) => Ok(SessionGetResult {
            resolution: GetResolution::Single,
            session: Some(dto_for(state, &row).await?),
            candidates: None,
        }),
        Err(rows) => Ok(SessionGetResult {
            resolution: GetResolution::Ambiguous,
            session: None,
            candidates: Some(
                rows.iter()
                    .map(convert::session_summary_dto)
                    .collect::<HandlerResult<Vec<_>>>()?,
            ),
        }),
    }
}

pub async fn list(state: &AppState, params: SessionListParams) -> HandlerResult<SessionListResult> {
    let states = params.state.as_deref();
    let rows = sdao::list(state.pool(), params.repository_id.as_deref(), states).await?;
    Ok(SessionListResult {
        sessions: rows
            .iter()
            .map(convert::session_summary_dto)
            .collect::<HandlerResult<Vec<_>>>()?,
    })
}

pub async fn heartbeat(
    state: &AppState,
    params: SessionHeartbeatParams,
) -> HandlerResult<SessionHeartbeatResult> {
    let row = state
        .inner
        .sessions
        .heartbeat(
            &params.session_id.to_string(),
            &params.agent_instance_id.to_string(),
            &params.resume_token,
        )
        .await?;
    Ok(SessionHeartbeatResult {
        state: cairn_domain::SessionState::parse(&row.state)
            .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "bad state"))?,
        last_heartbeat_at: cairn_domain::Timestamp::parse(&row.last_heartbeat_at)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
        lease_expires_at: cairn_domain::Timestamp::parse(&row.lease_expires_at)
            .map_err(|e| HandlerError::new(ErrorCode::Internal, e.to_string()))?,
    })
}

pub async fn reattach(
    state: &AppState,
    params: SessionReattachParams,
) -> HandlerResult<SessionReattachResult> {
    let session_id = params.session_id.to_string();
    let row = sdao::get_by_id(state.pool(), &session_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::SessionNotFound, "unknown session"))?;
    let worktree = worktrees::get_by_id(state.pool(), &row.worktree_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "missing worktree"))?;
    let repo = cairn_storage_local::repos::get_by_id(state.pool(), &row.repository_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "missing repository"))?;

    // Fresh snapshot is captured BEFORE authentication only in the success
    // path; compute lazily after basic state check to avoid doing Git work
    // for obviously invalid sessions.
    if row.state != "recovering" {
        return Err(SessionError::NotRecovering.into());
    }
    let (fresh, _) = super::snapshot::ensure_snapshot(
        state,
        &repo.id,
        &worktree.id,
        std::path::Path::new(&worktree.path),
    )
    .await?;

    let (updated, token) = state
        .inner
        .sessions
        .reattach(
            &session_id,
            &params.agent_instance_id.to_string(),
            &params.resume_token,
            &fresh,
        )
        .await?;

    if let Err(failure) = watch_ready(state, &repo.id, &worktree.id, &worktree.path).await {
        state
            .inner
            .sessions
            .interrupt_watcher_start(&updated, failure.stage)
            .await?;
        maybe_unwatch(state, &worktree.id).await;
        return Err(HandlerError::new(
            ErrorCode::WatcherStartFailed,
            "session watcher readiness failed",
        )
        .with_data(ErrorData::watcher_start_failure(failure.stage)));
    }
    Ok(SessionReattachResult {
        session: dto_for(state, &updated).await?,
        fresh_snapshot: convert::snapshot_dto(&fresh)?,
        resume_token: Some(token),
    })
}

pub async fn stop(state: &AppState, params: SessionStopParams) -> HandlerResult<SessionStopResult> {
    // Resolve target session.
    let row = if let Some(id) = &params.session_id {
        sdao::get_by_id(state.pool(), &id.to_string())
            .await?
            .ok_or_else(|| HandlerError::new(ErrorCode::SessionNotFound, "unknown session"))?
    } else {
        let repository_id = match (&params.repository_id, &params.path) {
            (Some(id), _) => Some(id.clone()),
            (None, Some(path)) => {
                let (repo, _, _) =
                    super::repository::resolve_registered(state, Some(path), None).await?;
                Some(repo.id)
            }
            (None, None) => None,
        };
        match state
            .inner
            .sessions
            .get(
                repository_id.as_deref(),
                None,
                params.agent_instance_id.map(|s| s.to_string()).as_deref(),
                None,
            )
            .await?
        {
            Ok(row) => row,
            Err(rows) => return Err(SessionError::Ambiguous(rows).into()),
        }
    };

    let worktree = worktrees::get_by_id(state.pool(), &row.worktree_id)
        .await?
        .ok_or_else(|| HandlerError::new(ErrorCode::Internal, "missing worktree"))?;
    let (final_snapshot, _) = super::snapshot::ensure_snapshot(
        state,
        &row.repository_id,
        &worktree.id,
        std::path::Path::new(&worktree.path),
    )
    .await?;

    let stopped = state
        .inner
        .sessions
        .stop(&row, params.resume_token.as_deref(), &final_snapshot)
        .await?;

    maybe_unwatch(state, &row.worktree_id).await;
    Ok(SessionStopResult {
        session: dto_for(state, &stopped).await?,
    })
}

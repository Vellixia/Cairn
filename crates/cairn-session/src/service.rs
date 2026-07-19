//! T035: session lifecycle policy (FR-013…FR-018, FR-034, FR-036).
//!
//! All mutations run inside per-worktree serialized transactions (analysis
//! I4) so events and projections never interleave out of order.

use std::sync::Arc;

use cairn_domain::{
    InterruptReason, LivenessReason, SessionId, SessionState, Timestamp, TransitionReason,
    WatcherStartStage,
};
use cairn_events::EventBuilder;
use cairn_storage_local::{
    events as ev, sessions as sdao, SessionRow, SnapshotRow, StorageError, WorktreeWriters,
};
use sqlx::SqlitePool;
use thiserror::Error;
use uuid::Uuid;

use crate::liveness::{evaluate, Health};
use crate::token::{generate_resume_token, verify_resume_token};

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session not found")]
    NotFound,
    #[error("session is not live")]
    NotLive,
    #[error("session is not recovering")]
    NotRecovering,
    #[error("multiple live sessions match; a selector is required")]
    Ambiguous(Vec<SessionRow>),
    #[error("resume token or agent instance mismatch")]
    LeaseMismatch,
    #[error("lease already expired")]
    LeaseExpired,
    #[error("reattachment grace deadline has passed")]
    GraceExpired,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Lifecycle timing knobs; env-overridable for tests and site policy.
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    /// Initial lease for callers that cannot heartbeat immediately (A1).
    pub initial_lease_secs: i64,
    /// Lease extension applied by each heartbeat.
    pub heartbeat_ttl_secs: i64,
    /// Reattachment window after entering `recovering` (A2).
    pub grace_secs: i64,
}

impl SessionConfig {
    pub fn from_env() -> Self {
        fn env_i64(key: &str, default: i64) -> i64 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        Self {
            initial_lease_secs: env_i64("CAIRN_INITIAL_LEASE_SECS", 900),
            heartbeat_ttl_secs: env_i64("CAIRN_HEARTBEAT_TTL_SECS", 90),
            grace_secs: env_i64("CAIRN_GRACE_SECS", 900),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartOutcome {
    Created,
    Existing,
    Takeover,
}

pub struct StartResult {
    pub session: SessionRow,
    /// Some on created/takeover; None on idempotent existing (FR-034).
    pub resume_token: Option<String>,
    pub outcome: StartOutcome,
}

pub struct SessionService {
    pool: SqlitePool,
    writers: Arc<WorktreeWriters>,
    pub config: SessionConfig,
}

fn parse_ts(s: &str) -> Timestamp {
    Timestamp::parse(s).unwrap_or_else(|_| Timestamp::now())
}

impl SessionService {
    pub fn new(pool: SqlitePool, writers: Arc<WorktreeWriters>, config: SessionConfig) -> Self {
        Self {
            pool,
            writers,
            config,
        }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn writers(&self) -> &Arc<WorktreeWriters> {
        &self.writers
    }

    /// FR-013/FR-017/FR-034: start (idempotent on healthy collision,
    /// takeover on stale collision). `snapshot` is the current authoritative
    /// snapshot row, already persisted by the caller.
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        repository_id: &str,
        worktree_id: &str,
        local_user: &str,
        agent_type: &str,
        agent_instance_id: &str,
        agent_pid: Option<i64>,
        snapshot: &SnapshotRow,
    ) -> Result<StartResult, SessionError> {
        let now = Timestamp::now();
        let cfg = self.config;
        let repository_id = repository_id.to_string();
        let worktree_id = worktree_id.to_string();
        let local_user = local_user.to_string();
        let agent_type = agent_type.to_string();
        let agent_instance_id = agent_instance_id.to_string();
        let snapshot = snapshot.clone();

        ev::serialized_txn(
            &self.pool,
            &self.writers,
            &worktree_id.clone(),
            Box::new(move |conn| {
                Box::pin(async move {
                    let existing =
                        sdao::get_live_for_instance(&mut *conn, &repository_id, &agent_instance_id)
                            .await?;

                    if let Some(existing) = existing {
                        let state = SessionState::parse(&existing.state)
                            .ok_or_else(|| StorageError::Corrupted("bad state".into()))?;
                        match state {
                            SessionState::Active => {
                                let health = evaluate(
                                    parse_ts(&existing.lease_expires_at),
                                    existing.agent_pid,
                                    now,
                                );
                                match health {
                                    Health::Healthy => {
                                        return Ok(StartResult {
                                            session: existing,
                                            resume_token: None,
                                            outcome: StartOutcome::Existing,
                                        });
                                    }
                                    Health::Stale(reason) => {
                                        interrupt_in_txn(
                                            conn,
                                            &existing,
                                            InterruptReason::StaleTakeover,
                                            reason,
                                            &now,
                                        )
                                        .await?;
                                        let (row, token) = insert_new_session(
                                            conn,
                                            &repository_id,
                                            &worktree_id,
                                            &local_user,
                                            &agent_type,
                                            &agent_instance_id,
                                            agent_pid,
                                            &snapshot,
                                            &now,
                                            cfg.initial_lease_secs,
                                        )
                                        .await?;
                                        return Ok(StartResult {
                                            session: row,
                                            resume_token: Some(token),
                                            outcome: StartOutcome::Takeover,
                                        });
                                    }
                                }
                            }
                            SessionState::Recovering => {
                                // Reattach window still open: return the
                                // recovering session unchanged; the agent
                                // should reattach. Past deadline: interrupt
                                // and start fresh (grace-expired path).
                                let deadline = existing
                                    .recovering_since
                                    .as_deref()
                                    .map(parse_ts)
                                    .map(|t| t.plus_seconds(cfg.grace_secs));
                                let expired = deadline.map(|d| now > d).unwrap_or(false);
                                if !expired {
                                    return Ok(StartResult {
                                        session: existing,
                                        resume_token: None,
                                        outcome: StartOutcome::Existing,
                                    });
                                }
                                grace_interrupt_in_txn(conn, &existing, &now).await?;
                                let (row, token) = insert_new_session(
                                    conn,
                                    &repository_id,
                                    &worktree_id,
                                    &local_user,
                                    &agent_type,
                                    &agent_instance_id,
                                    agent_pid,
                                    &snapshot,
                                    &now,
                                    cfg.initial_lease_secs,
                                )
                                .await?;
                                return Ok(StartResult {
                                    session: row,
                                    resume_token: Some(token),
                                    outcome: StartOutcome::Takeover,
                                });
                            }
                            _ => unreachable!("get_live_for_instance returns live only"),
                        }
                    }

                    let (row, token) = insert_new_session(
                        conn,
                        &repository_id,
                        &worktree_id,
                        &local_user,
                        &agent_type,
                        &agent_instance_id,
                        agent_pid,
                        &snapshot,
                        &now,
                        cfg.initial_lease_secs,
                    )
                    .await?;
                    Ok(StartResult {
                        session: row,
                        resume_token: Some(token),
                        outcome: StartOutcome::Created,
                    })
                })
            }),
        )
        .await
        .map_err(SessionError::from)
    }

    /// FR-036 adaptive resolution: instance context → that session; exactly
    /// one live session → single; multiple → ambiguous. Never recency-picked.
    pub async fn get(
        &self,
        repository_id: Option<&str>,
        session_id: Option<&str>,
        agent_instance_id: Option<&str>,
        agent_type: Option<&str>,
    ) -> Result<Result<SessionRow, Vec<SessionRow>>, SessionError> {
        if let Some(id) = session_id {
            let row = sdao::get_by_id(&self.pool, id)
                .await?
                .ok_or(SessionError::NotFound)?;
            return Ok(Ok(row));
        }
        let live = [SessionState::Active, SessionState::Recovering];
        let mut rows = sdao::list(&self.pool, repository_id, Some(&live)).await?;
        if let Some(instance) = agent_instance_id {
            rows.retain(|r| r.agent_instance_id == instance);
        }
        if let Some(at) = agent_type {
            rows.retain(|r| r.agent_type == at);
        }
        match rows.len() {
            0 => Err(SessionError::NotFound),
            1 => Ok(Ok(rows.into_iter().next().expect("len checked"))),
            _ => Ok(Err(rows)),
        }
    }

    /// Heartbeat: authenticated lease extension (research R8).
    pub async fn heartbeat(
        &self,
        session_id: &str,
        agent_instance_id: &str,
        resume_token: &str,
    ) -> Result<SessionRow, SessionError> {
        let row = sdao::get_by_id(&self.pool, session_id)
            .await?
            .ok_or(SessionError::NotFound)?;
        if row.state != "active" {
            return Err(SessionError::NotLive);
        }
        if row.agent_instance_id != agent_instance_id
            || !verify_resume_token(resume_token, &row.resume_token_hash)
        {
            return Err(SessionError::LeaseMismatch);
        }
        let now = Timestamp::now();
        if now > parse_ts(&row.lease_expires_at) {
            return Err(SessionError::LeaseExpired);
        }
        let new_lease = now.plus_seconds(self.config.heartbeat_ttl_secs);
        sdao::update_heartbeat(
            &self.pool,
            session_id,
            &now.to_rfc3339(),
            &new_lease.to_rfc3339(),
        )
        .await?;
        sdao::get_by_id(&self.pool, session_id)
            .await?
            .ok_or(SessionError::NotFound)
    }

    /// FR-018/analysis I3: authenticated reattachment. Mismatch is
    /// reject-only (audit event, session untouched).
    pub async fn reattach(
        &self,
        session_id: &str,
        agent_instance_id: &str,
        resume_token: &str,
        fresh_snapshot: &SnapshotRow,
    ) -> Result<(SessionRow, String), SessionError> {
        let row = sdao::get_by_id(&self.pool, session_id)
            .await?
            .ok_or(SessionError::NotFound)?;
        if row.state != "recovering" {
            return Err(SessionError::NotRecovering);
        }

        let now = Timestamp::now();
        let deadline = row
            .recovering_since
            .as_deref()
            .map(parse_ts)
            .map(|t| t.plus_seconds(self.config.grace_secs));
        if deadline.map(|d| now > d).unwrap_or(false) {
            // Deterministic expiry on the request path (sweeper may lag).
            let row_c = row.clone();
            ev::serialized_txn(
                &self.pool,
                &self.writers,
                &row.worktree_id.clone(),
                Box::new(move |conn| {
                    Box::pin(async move { grace_interrupt_in_txn(conn, &row_c, &now).await })
                }),
            )
            .await?;
            return Err(SessionError::GraceExpired);
        }

        if row.agent_instance_id != agent_instance_id
            || !verify_resume_token(resume_token, &row.resume_token_hash)
        {
            // Reject-only: audit event, no session mutation (analysis I3).
            let audit = EventBuilder::session_reattach_rejected(
                &row.repository_id,
                &row.worktree_id,
                &row.id,
                agent_instance_id,
                "lease_mismatch",
                &Uuid::new_v4().to_string(),
            );
            let wk = row.worktree_id.clone();
            ev::serialized_txn(
                &self.pool,
                &self.writers,
                &wk,
                Box::new(move |conn| {
                    Box::pin(async move {
                        ev::append_event(conn, &audit).await?;
                        Ok(())
                    })
                }),
            )
            .await?;
            return Err(SessionError::LeaseMismatch);
        }

        let (token, hash) = generate_resume_token();
        let new_lease = now.plus_seconds(self.config.initial_lease_secs);
        let row_c = row.clone();
        let snapshot = fresh_snapshot.clone();
        let wk = row.worktree_id.clone();
        ev::serialized_txn(
            &self.pool,
            &self.writers,
            &wk,
            Box::new(move |conn| {
                Box::pin(async move {
                    sdao::apply_transition(
                        &mut *conn,
                        &row_c.id,
                        SessionState::Recovering,
                        SessionState::Active,
                        TransitionReason::Reattach,
                        &now.to_rfc3339(),
                    )
                    .await?;
                    sdao::update_resume_token_hash(
                        &mut *conn,
                        &row_c.id,
                        &hash,
                        &new_lease.to_rfc3339(),
                    )
                    .await?;
                    sdao::update_current_snapshot(&mut *conn, &row_c.id, &snapshot.id).await?;
                    let event = EventBuilder::session_recovered(
                        &row_c.repository_id,
                        &row_c.worktree_id,
                        &row_c.id,
                        &snapshot.id,
                    );
                    ev::append_event(conn, &event).await?;
                    Ok(())
                })
            }),
        )
        .await?;
        let updated = sdao::get_by_id(&self.pool, session_id)
            .await?
            .ok_or(SessionError::NotFound)?;
        Ok((updated, token))
    }

    /// FR-016: stop. Active stops directly; recovering requires the token
    /// (authenticated owner stop, analysis I3).
    pub async fn stop(
        &self,
        session: &SessionRow,
        resume_token: Option<&str>,
        final_snapshot: &SnapshotRow,
    ) -> Result<SessionRow, SessionError> {
        let now = Timestamp::now();
        let state = SessionState::parse(&session.state).ok_or(SessionError::NotLive)?;
        let reason = match state {
            SessionState::Active => TransitionReason::Stop,
            SessionState::Recovering => {
                let ok = resume_token
                    .map(|t| verify_resume_token(t, &session.resume_token_hash))
                    .unwrap_or(false);
                if !ok {
                    return Err(SessionError::LeaseMismatch);
                }
                TransitionReason::AuthenticatedStop
            }
            _ => return Err(SessionError::NotLive),
        };

        let row_c = session.clone();
        let snapshot = final_snapshot.clone();
        let wk = session.worktree_id.clone();
        ev::serialized_txn(
            &self.pool,
            &self.writers,
            &wk,
            Box::new(move |conn| {
                Box::pin(async move {
                    sdao::apply_transition(
                        &mut *conn,
                        &row_c.id,
                        state,
                        SessionState::Stopped,
                        reason,
                        &now.to_rfc3339(),
                    )
                    .await?;
                    sdao::update_current_snapshot(&mut *conn, &row_c.id, &snapshot.id).await?;
                    let event = EventBuilder::session_stopped(
                        &row_c.repository_id,
                        &row_c.worktree_id,
                        &row_c.id,
                        &snapshot.id,
                    );
                    ev::append_event(conn, &event).await?;
                    Ok(())
                })
            }),
        )
        .await?;
        sdao::get_by_id(&self.pool, &session.id)
            .await?
            .ok_or(SessionError::NotFound)
    }

    /// FR-038: a newly-created session whose watcher readiness protocol
    /// fails must become interrupted in the same transaction that appends
    /// the failure evidence.
    pub async fn interrupt_watcher_start(
        &self,
        session: &SessionRow,
        stage: WatcherStartStage,
    ) -> Result<SessionRow, SessionError> {
        let state = SessionState::parse(&session.state).ok_or(SessionError::NotLive)?;
        if state != SessionState::Active {
            return Err(SessionError::NotLive);
        }
        let now = Timestamp::now();
        let row = session.clone();
        let worktree_id = session.worktree_id.clone();
        ev::serialized_txn(
            &self.pool,
            &self.writers,
            &worktree_id,
            Box::new(move |conn| {
                Box::pin(async move {
                    sdao::apply_transition(
                        &mut *conn,
                        &row.id,
                        SessionState::Active,
                        SessionState::Interrupted,
                        TransitionReason::WatcherStartFailed,
                        &now.to_rfc3339(),
                    )
                    .await?;
                    let event = EventBuilder::session_watcher_start_failed(
                        &row.repository_id,
                        &row.worktree_id,
                        &row.id,
                        stage,
                    );
                    ev::append_event(conn, &event).await?;
                    Ok(())
                })
            }),
        )
        .await?;
        sdao::get_by_id(&self.pool, &session.id)
            .await?
            .ok_or(SessionError::NotFound)
    }

    /// T047: boot recovery — every active session moves to `recovering`,
    /// persisting `recovering_since` only when not already set (analysis A2).
    pub async fn mark_all_active_recovering(&self) -> Result<u64, SessionError> {
        let rows = sdao::list_by_state(&self.pool, SessionState::Active).await?;
        let now = Timestamp::now();
        let mut n = 0;
        for row in rows {
            let wk = row.worktree_id.clone();
            let id = row.id.clone();
            let now_s = now.to_rfc3339();
            ev::serialized_txn(
                &self.pool,
                &self.writers,
                &wk,
                Box::new(move |conn| {
                    Box::pin(async move {
                        sdao::apply_transition(
                            &mut *conn,
                            &id,
                            SessionState::Active,
                            SessionState::Recovering,
                            TransitionReason::DaemonRestart,
                            &now_s,
                        )
                        .await
                    })
                }),
            )
            .await?;
            n += 1;
        }
        Ok(n)
    }

    /// T048: sweeper pass. Recovering sessions past their grace deadline are
    /// interrupted; active sessions whose lease expired AND whose process is
    /// verifiably dead are interrupted (a missing PID never triggers this).
    pub async fn sweep(&self) -> Result<u64, SessionError> {
        let now = Timestamp::now();
        let mut interrupted = 0;

        for row in sdao::list_by_state(&self.pool, SessionState::Recovering).await? {
            let deadline = row
                .recovering_since
                .as_deref()
                .map(parse_ts)
                .map(|t| t.plus_seconds(self.config.grace_secs));
            if deadline.map(|d| now > d).unwrap_or(false) {
                let wk = row.worktree_id.clone();
                let row_c = row.clone();
                ev::serialized_txn(
                    &self.pool,
                    &self.writers,
                    &wk,
                    Box::new(move |conn| {
                        Box::pin(async move { grace_interrupt_in_txn(conn, &row_c, &now).await })
                    }),
                )
                .await?;
                interrupted += 1;
            }
        }

        for row in sdao::list_by_state(&self.pool, SessionState::Active).await? {
            let lease_expired = now > parse_ts(&row.lease_expires_at);
            let verifiably_dead =
                matches!(crate::liveness::process_alive(row.agent_pid), Some(false));
            if lease_expired && verifiably_dead {
                let wk = row.worktree_id.clone();
                let row_c = row.clone();
                ev::serialized_txn(
                    &self.pool,
                    &self.writers,
                    &wk,
                    Box::new(move |conn| {
                        Box::pin(async move {
                            interrupt_in_txn(
                                conn,
                                &row_c,
                                InterruptReason::StaleTakeover,
                                LivenessReason::ProcessDead,
                                &now,
                            )
                            .await
                        })
                    }),
                )
                .await?;
                interrupted += 1;
            }
        }
        Ok(interrupted)
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_new_session(
    conn: &mut sqlx::SqliteConnection,
    repository_id: &str,
    worktree_id: &str,
    local_user: &str,
    agent_type: &str,
    agent_instance_id: &str,
    agent_pid: Option<i64>,
    snapshot: &SnapshotRow,
    now: &Timestamp,
    initial_lease_secs: i64,
) -> Result<(SessionRow, String), StorageError> {
    let (token, hash) = generate_resume_token();
    let row = SessionRow {
        id: SessionId::new_v7().to_string(),
        repository_id: repository_id.to_string(),
        worktree_id: worktree_id.to_string(),
        local_user: local_user.to_string(),
        agent_type: agent_type.to_string(),
        agent_instance_id: agent_instance_id.to_string(),
        agent_pid,
        resume_token_hash: hash,
        lease_expires_at: now.plus_seconds(initial_lease_secs).to_rfc3339(),
        state: "active".to_string(),
        start_snapshot_id: snapshot.id.clone(),
        current_snapshot_id: snapshot.id.clone(),
        started_at: now.to_rfc3339(),
        ended_at: None,
        last_heartbeat_at: now.to_rfc3339(),
        recovering_since: None,
    };
    sdao::insert(&mut *conn, &row).await?;
    let event = cairn_events::EventBuilder::session_started(
        repository_id,
        worktree_id,
        &row.id,
        &cairn_events::SessionStartedPayload {
            agent_type: agent_type.to_string(),
            agent_instance_id: agent_instance_id.to_string(),
            start_snapshot_id: snapshot.id.clone(),
            local_user: local_user.to_string(),
        },
    );
    ev::append_event(&mut *conn, &event).await?;
    Ok((row, token))
}

async fn interrupt_in_txn(
    conn: &mut sqlx::SqliteConnection,
    row: &SessionRow,
    reason: InterruptReason,
    liveness: LivenessReason,
    now: &Timestamp,
) -> Result<(), StorageError> {
    sdao::apply_transition(
        &mut *conn,
        &row.id,
        SessionState::Active,
        SessionState::Interrupted,
        TransitionReason::StaleTakeover,
        &now.to_rfc3339(),
    )
    .await?;
    let event = EventBuilder::session_interrupted(
        &row.repository_id,
        &row.worktree_id,
        &row.id,
        reason.as_str(),
        liveness.as_str(),
    );
    ev::append_event(&mut *conn, &event).await?;
    Ok(())
}

async fn grace_interrupt_in_txn(
    conn: &mut sqlx::SqliteConnection,
    row: &SessionRow,
    now: &Timestamp,
) -> Result<(), StorageError> {
    sdao::apply_transition(
        &mut *conn,
        &row.id,
        SessionState::Recovering,
        SessionState::Interrupted,
        TransitionReason::GraceExpired,
        &now.to_rfc3339(),
    )
    .await?;
    let event = EventBuilder::session_interrupted(
        &row.repository_id,
        &row.worktree_id,
        &row.id,
        InterruptReason::GraceExpired.as_str(),
        LivenessReason::ReattachTimeout.as_str(),
    );
    ev::append_event(&mut *conn, &event).await?;
    Ok(())
}

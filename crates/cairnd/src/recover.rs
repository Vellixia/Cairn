//! Deterministic session-boundary reconciliation (FR-009, D16).
//!
//! Cairn has no liveness signal: no hook payload carries a process identity,
//! and a crash produces no event at all. So Cairn does not guess. Daemon start
//! is a boundary it *can* observe — every session still `active` belongs to a
//! previous run and is reconciled here, with a `recovered` handoff written from
//! whatever it managed to record.
//!
//! If a later event arrives for such a session it resumes to `active` and that
//! handoff stands as a valid boundary record, which is why this costs nothing
//! and still tolerates a mid-session daemon restart (FR-046).
//!
//! Silence is the one exception. A session nothing has touched for hours is
//! presumed abandoned and closed with a `recovered` handoff — not because
//! Cairn knows the agent is gone, but because leaving it `active` is worse:
//! two of them in one worktree make `cairn context` ambiguous, which blocks
//! the briefing an agent asks for before it knows its own session key. The
//! timeout is generous enough that a developer reading and thinking is never
//! mistaken for one who left, and a later event resumes the session anyway.
//!
//! No heartbeat, no lease, no PID table.

use crate::state::Daemon;
use cairn_core::domain::{HandoffTrigger, SessionStatus};
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo;

/// Mark memory whose branch or task no longer resolves as `stale`, for every
/// known project (FR-018, H4).
///
/// Runs once at start; `cairn status` repeats it per project afterwards.
pub async fn reconcile_stale_memory(daemon: &Daemon) -> usize {
    let projects = match repo::list_projects(&daemon.store).await {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error = %e, "could not list projects for stale reconciliation");
            return 0;
        }
    };
    let mut marked = 0;
    for project in projects {
        let worktree = std::path::PathBuf::from(&project.git_common_dir);
        // The common dir is inside the worktree for a normal checkout; its
        // parent is where `git branch` works.
        let candidate = worktree
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or(worktree);
        let branches = match crate::state::git_branches(candidate).await {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(n) = repo::mark_stale_scopes(&daemon.store, project.id, &branches).await {
            marked += n as usize;
        }
    }
    marked
}

/// Hand back outbox rows a previous run claimed but never acknowledged.
///
/// A drainer that was killed mid-send leaves its rows `in_flight`. They would
/// recover on their own once the claim went stale, but daemon start is a
/// boundary Cairn can observe: nothing is draining yet, so any claim still
/// standing belongs to a run that is already gone. Same reasoning as session
/// reconciliation, and the same absence of a heartbeat or lease (FR-056, D16).
pub async fn release_abandoned_claims(daemon: &Daemon) -> u64 {
    match cairn_store::outbox::release_all_claims(&daemon.store).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "could not release abandoned outbox claims");
            0
        }
    }
}

/// How long a session may go without an event before it is presumed abandoned.
///
/// Long enough that a developer reading and thinking is never interrupted;
/// short enough that a lost session does not poison the next one for a whole
/// working day.
pub const IDLE_SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

/// Close sessions nothing has driven for `idle_for`.
///
/// Daemon start already reconciles sessions from a *previous* run, but a daemon
/// that keeps running had no way to notice a session everyone had walked away
/// from. Those accumulate, and two active sessions in one worktree make
/// `cairn context` ambiguous — precisely the call an agent makes before it
/// knows its own session key.
///
/// Returns how many were closed.
pub async fn reap_idle_sessions(daemon: &Daemon, idle_for: std::time::Duration) -> usize {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(idle_for).unwrap_or_else(|_| chrono::Duration::hours(2));

    let idle = match repo::sessions_idle_since(&daemon.store, cutoff).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "could not read idle sessions");
            return 0;
        }
    };

    let mut count = 0;
    for session in idle {
        let policy = match repo::project(&daemon.store, session.project_id).await {
            Ok(p) => SyncPolicy::from_project(&p),
            Err(_) => SyncPolicy {
                linked: false,
                server_project_id: None,
            },
        };

        // Handoff first, as in reconcile_previous_runs: an extra boundary
        // record beats losing the session's work if the status update fails.
        if let Err(e) =
            crate::handoffs::generate(daemon, &session, HandoffTrigger::Recovered, policy).await
        {
            tracing::warn!(session = %session.id, error = %e, "idle handoff failed");
        }

        match repo::end_session(
            &daemon.store,
            session.id,
            SessionStatus::Interrupted,
            Some("no events for longer than the idle timeout"),
            policy,
        )
        .await
        {
            Ok(_) => {
                count += 1;
                tracing::info!(session = %session.id, "reaped idle session");
            }
            Err(e) => tracing::warn!(session = %session.id, error = %e, "idle reap failed"),
        }
    }
    count
}

/// Reconcile sessions left `active` by a previous daemon run.
///
/// Returns how many were reconciled.
pub async fn reconcile_previous_runs(daemon: &Daemon) -> usize {
    let stale = match repo::sessions_from_previous_runs(&daemon.store, daemon.run_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "could not read sessions for reconciliation");
            return 0;
        }
    };

    let mut count = 0;
    for session in stale {
        let policy = match repo::project(&daemon.store, session.project_id).await {
            Ok(p) => SyncPolicy::from_project(&p),
            Err(_) => SyncPolicy {
                linked: false,
                server_project_id: None,
            },
        };

        // The handoff is written first: if the status update fails we would
        // rather have an extra boundary record than none.
        match crate::handoffs::generate(daemon, &session, HandoffTrigger::Recovered, policy).await {
            Ok(_) => {}
            Err(e) => tracing::warn!(session = %session.id, error = %e, "recovery handoff failed"),
        }

        match repo::end_session(
            &daemon.store,
            session.id,
            SessionStatus::Interrupted,
            Some("daemon restart: session was active in a previous run"),
            policy,
        )
        .await
        {
            Ok(_) => {
                count += 1;
                tracing::info!(session = %session.id, "reconciled to interrupted");
            }
            Err(e) => tracing::warn!(session = %session.id, error = %e, "reconciliation failed"),
        }
    }
    count
}

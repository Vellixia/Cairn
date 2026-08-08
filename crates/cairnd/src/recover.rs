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

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport as fx;
    use cairn_core::domain::SessionStatus;

    /// Daemon start is the boundary Cairn can observe (FR-009, D16).
    ///
    /// A session left `active` by a previous run is reconciled to `interrupted`
    /// with a `recovered` handoff written from whatever it managed to record.
    #[tokio::test]
    async fn a_session_from_a_previous_run_is_reconciled_with_a_handoff() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "crashed", None).await;
        // A different run id is what makes it "previous" — there is no liveness
        // signal to consult, which is the whole point of the design.
        let s = fx::session_in_run(&d, &p, "orphan", uuid::Uuid::now_v7()).await;
        fx::observe_edit(&d, &s, "src/lib.rs").await;

        assert_eq!(reconcile_previous_runs(&d).await, 1);

        let reloaded = repo::session(&d.store, s.id).await.expect("session");
        assert_eq!(reloaded.status, SessionStatus::Interrupted);
        assert!(
            reloaded
                .end_reason
                .as_deref()
                .unwrap_or_default()
                .contains("previous run"),
            "the reason should say why: {:?}",
            reloaded.end_reason
        );

        let handoff = repo::latest_handoff(&d.store, s.id)
            .await
            .expect("query")
            .expect("a recovered handoff must exist");
        assert_eq!(handoff.trigger, HandoffTrigger::Recovered);
        assert!(
            handoff.changed_files.iter().any(|f| f == "src/lib.rs"),
            "the handoff carries what the session recorded: {:?}",
            handoff.changed_files
        );
    }

    /// This run's own sessions are not touched.
    ///
    /// Reconciliation keys on the run id, so a daemon that restarts must not
    /// close the session it is currently serving.
    #[tokio::test]
    async fn a_session_from_this_run_is_left_alone() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "live", None).await;
        let s = fx::session(&d, &p, "current").await;

        assert_eq!(reconcile_previous_runs(&d).await, 0);
        assert_eq!(
            repo::session(&d.store, s.id).await.expect("session").status,
            SessionStatus::Active
        );
        assert!(
            repo::latest_handoff(&d.store, s.id)
                .await
                .expect("query")
                .is_none(),
            "a live session should not get a recovery handoff"
        );
    }

    /// Nothing to reconcile is not an error, and writes nothing.
    #[tokio::test]
    async fn an_empty_store_reconciles_nothing() {
        let d = fx::daemon().await;
        assert_eq!(reconcile_previous_runs(&d).await, 0);
        assert_eq!(reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT).await, 0);
        assert_eq!(release_abandoned_claims(&d).await, 0);
    }

    /// A session nothing has touched is presumed abandoned and closed.
    ///
    /// Not because Cairn knows the agent is gone, but because two `active`
    /// sessions in one worktree make `cairn context` ambiguous — the call an
    /// agent makes before it knows its own session key.
    #[tokio::test]
    async fn an_idle_session_is_reaped_with_a_recovered_handoff() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "idle", None).await;
        let s = fx::session(&d, &p, "abandoned").await;

        // Zero tolerance rather than a fabricated timestamp: every session is
        // then idle, which is the condition under test without reaching into
        // the schema to age a row.
        assert_eq!(reap_idle_sessions(&d, std::time::Duration::ZERO).await, 1);

        let reloaded = repo::session(&d.store, s.id).await.expect("session");
        assert_eq!(reloaded.status, SessionStatus::Interrupted);
        assert!(
            reloaded
                .end_reason
                .as_deref()
                .unwrap_or_default()
                .contains("idle timeout"),
            "the reason should name the timeout: {:?}",
            reloaded.end_reason
        );
        assert_eq!(
            repo::latest_handoff(&d.store, s.id)
                .await
                .expect("query")
                .map(|h| h.trigger),
            Some(HandoffTrigger::Recovered)
        );
    }

    /// A session inside the timeout is not reaped.
    ///
    /// The timeout is generous on purpose: a developer reading and thinking
    /// must never be mistaken for one who left.
    #[tokio::test]
    async fn a_recently_active_session_is_not_reaped() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "thinking", None).await;
        let s = fx::session(&d, &p, "reading").await;

        assert_eq!(reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT).await, 0);
        assert_eq!(
            repo::session(&d.store, s.id).await.expect("session").status,
            SessionStatus::Active
        );
    }

    /// Reaping is idempotent: a closed session is not closed again.
    #[tokio::test]
    async fn reaping_twice_closes_nothing_the_second_time() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "twice", None).await;
        fx::session(&d, &p, "once").await;

        assert_eq!(reap_idle_sessions(&d, std::time::Duration::ZERO).await, 1);
        assert_eq!(
            reap_idle_sessions(&d, std::time::Duration::ZERO).await,
            0,
            "an already-interrupted session is no longer active, so not idle"
        );
    }

    /// Claims standing at daemon start belong to a run that is already gone.
    ///
    /// Nothing is draining yet, so any claim still held is abandoned by
    /// definition — the same reasoning as session reconciliation, and the same
    /// absence of a lease (FR-056, D16).
    #[tokio::test]
    async fn claims_left_standing_are_handed_back() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "queued", None).await;
        let target = uuid::Uuid::now_v7();
        repo::link_project(&d.store, p.id, target)
            .await
            .expect("link");

        // Starting a session on a *linked* project queues its provenance — the
        // project has to be re-read first, because `p` still describes the row
        // as it was before the link. Observations deliberately never queue:
        // their content stays local (FR-055).
        let linked = fx::reload(&d, p.id).await;
        fx::session_in_run(&d, &linked, "sender", uuid::Uuid::now_v7()).await;

        let claimed = cairn_store::outbox::claim(&d.store, p.id, 100)
            .await
            .expect("claim");
        assert!(
            !claimed.is_empty(),
            "precondition: a linked project must have queued something"
        );

        assert_eq!(
            release_abandoned_claims(&d).await,
            claimed.len() as u64,
            "every standing claim is handed back"
        );
    }
}

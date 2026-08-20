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

/// How long a session that a newer one in the same worktree has overtaken may
/// stay silent before it is presumed abandoned.
///
/// Much shorter than `IDLE_SESSION_TIMEOUT`, because the evidence is stronger:
/// a session working alone that goes quiet is probably being read; a session
/// that goes quiet while a *newer* session runs in the same worktree has
/// probably been replaced. An agent that is killed and restarted — rather than
/// exited — leaves exactly this trace, and the old session then blocks the
/// briefing for the new one.
///
/// Matched to the sweep interval, so a worktree converges on one session within
/// roughly two ticks instead of the two hours a solo session is granted. Being
/// wrong is cheap and self-correcting: a session closed here is found by its own
/// key regardless of status, and the next event it produces resumes it (D16
/// rule 4). Being wrong in the other direction is what #41 reports — a worktree
/// that stays ambiguous for hours.
pub const SUPERSEDED_SESSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Close sessions nothing has driven for `idle_for`, and sessions a newer
/// session in the same worktree overtook more than `superseded_after` ago.
///
/// Daemon start already reconciles sessions from a *previous* run, but a daemon
/// that keeps running had no way to notice a session everyone had walked away
/// from. Those accumulate, and two active sessions in one worktree make
/// `cairn context` ambiguous — precisely the call an agent makes before it
/// knows its own session key.
///
/// Returns how many were closed.
pub async fn reap_idle_sessions(
    daemon: &Daemon,
    idle_for: std::time::Duration,
    superseded_after: std::time::Duration,
) -> usize {
    let now = chrono::Utc::now();
    let cutoff =
        now - chrono::Duration::from_std(idle_for).unwrap_or_else(|_| chrono::Duration::hours(2));

    let idle = match repo::sessions_idle_since(&daemon.store, cutoff).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "could not read idle sessions");
            return 0;
        }
    };

    // Sessions a newer one in the same worktree replaced. Held to a much
    // shorter silence than a session working alone, and never including the
    // newest in a worktree, so this can narrow a worktree to one session but
    // never empty it (#41).
    let superseded_cutoff = now
        - chrono::Duration::from_std(superseded_after)
            .unwrap_or_else(|_| chrono::Duration::minutes(15));
    let superseded =
        match repo::superseded_sessions_idle_since(&daemon.store, superseded_cutoff).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "could not read superseded sessions");
                Vec::new()
            }
        };

    // The reason travels with the session: "superseded" and "idle" are
    // different findings, and `cairn session show` is where a developer asks
    // why a session ended.
    let mut seen: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
    let mut to_close: Vec<(_, &'static str)> = Vec::new();
    for session in idle {
        if seen.insert(session.id) {
            to_close.push((session, "no events for longer than the idle timeout"));
        }
    }
    for session in superseded {
        if seen.insert(session.id) {
            tracing::info!(
                session = %session.id, agent = %session.agent,
                "a newer session in this worktree has overtaken this one"
            );
            to_close.push((
                session,
                "a newer session in this worktree overtook it and it went silent",
            ));
        }
    }

    let mut count = 0;
    for (session, reason) in to_close {
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
            Some(reason),
            policy,
        )
        .await
        {
            Ok(_) => {
                count += 1;
                tracing::info!(session = %session.id, reason, "reaped idle session");
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

/// How long a sealed boundary may owe its handoff before the sweep takes it.
///
/// The synthesis task spawned at close is the primary path and normally lands
/// in milliseconds; this is the interval after which a boundary is considered
/// abandoned by it — a task that was cancelled, or a process that died between
/// the two phases.
pub const HANDOFF_SWEEP_AFTER: std::time::Duration = crate::handoffs::HANDOFF_BOUND;

/// Synthesize the handoffs sealed boundaries still owe (FR-240 clause 2, D22).
///
/// **Progress is guaranteed while the daemon runs, not only across a
/// restart.** The synthesis task retries on its own; this sweep runs on the
/// maintenance tick that already reaps idle sessions and picks up anything
/// that task gave up on or never got to. No new scheduler.
///
/// Returns how many handoffs it produced.
pub async fn sweep_pending_handoffs(daemon: &Daemon, owed_for: std::time::Duration) -> usize {
    let cutoff = chrono::Utc::now()
        - chrono::Duration::from_std(owed_for).unwrap_or_else(|_| chrono::Duration::seconds(5));

    let owed = match repo::sessions_awaiting_handoff(&daemon.store, cutoff).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "could not read sessions awaiting a handoff");
            return 0;
        }
    };

    let mut produced = 0;
    for session in owed {
        let policy = match repo::project(&daemon.store, session.project_id).await {
            Ok(p) => SyncPolicy::from_project(&p),
            Err(_) => SyncPolicy {
                linked: false,
                server_project_id: None,
            },
        };
        // The trigger stays `session_end`: this is the boundary's own handoff
        // arriving late, not a recovery from silence. Recovery never satisfies
        // the completion guarantee, and mislabelling this would make the
        // distinction meaningless (FR-229).
        match crate::handoffs::generate(daemon, &session, HandoffTrigger::SessionEnd, policy).await
        {
            Ok(_) => {
                if let Err(e) = repo::clear_handoff_pending(&daemon.store, session.id).await {
                    tracing::warn!(session = %session.id, error = %e, "could not clear handoff_pending");
                    continue;
                }
                produced += 1;
                tracing::info!(session = %session.id, "swept a pending handoff");
            }
            Err(e) => {
                let attempts = repo::record_handoff_failure(&daemon.store, session.id, &e.message)
                    .await
                    .unwrap_or(0);
                tracing::warn!(
                    session = %session.id,
                    attempts,
                    error = %e.message,
                    "pending handoff still failing; it stays owed and retryable"
                );
            }
        }
    }
    produced
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
        assert_eq!(
            reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT, IDLE_SESSION_TIMEOUT).await,
            0
        );
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
        assert_eq!(
            reap_idle_sessions(&d, std::time::Duration::ZERO, IDLE_SESSION_TIMEOUT).await,
            1
        );

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

        assert_eq!(
            reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT, IDLE_SESSION_TIMEOUT).await,
            0
        );
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

        assert_eq!(
            reap_idle_sessions(&d, std::time::Duration::ZERO, IDLE_SESSION_TIMEOUT).await,
            1
        );
        assert_eq!(
            reap_idle_sessions(&d, std::time::Duration::ZERO, IDLE_SESSION_TIMEOUT).await,
            0,
            "an already-interrupted session is no longer active, so not idle"
        );
    }

    /// A restarted agent's abandoned session does not poison the worktree for
    /// hours (#41).
    ///
    /// The reported case: three OpenCode sessions in one worktree, started
    /// minutes apart because the agent was restarted rather than exited. The
    /// first did 44 seconds of work and the second 3m22s; both then sat `active`
    /// for two and five hours while the third did 26 hours of real work, and
    /// every `cairn context` in that worktree failed with `ambiguous_session`
    /// throughout.
    ///
    /// The generous idle timeout is correct for a session on its own, so it is
    /// passed here unchanged and proves it is not what does the work: only the
    /// superseded window is zero.
    #[tokio::test]
    async fn a_session_a_newer_one_overtook_is_reaped_without_waiting_for_the_idle_timeout() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "restarted", None).await;
        let abandoned = fx::session(&d, &p, "opencode-first").await;
        let live = fx::session(&d, &p, "opencode-second").await;

        assert_eq!(
            reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT, std::time::Duration::ZERO).await,
            1,
            "the overtaken session was not reaped"
        );

        let abandoned = repo::session(&d.store, abandoned.id)
            .await
            .expect("session");
        assert_eq!(abandoned.status, SessionStatus::Interrupted);
        assert!(
            abandoned
                .end_reason
                .as_deref()
                .unwrap_or_default()
                .contains("overtook"),
            "the reason should say it was overtaken, not that it timed out: {:?}",
            abandoned.end_reason
        );

        // The newest session in a worktree is never reaped by this rule, so a
        // worktree can be narrowed to one session but never emptied.
        assert_eq!(
            repo::session(&d.store, live.id)
                .await
                .expect("session")
                .status,
            SessionStatus::Active,
            "the live session was reaped along with the debris"
        );
    }

    /// The worktree converges on exactly one session, whatever the pile-up.
    #[tokio::test]
    async fn three_sessions_in_one_worktree_collapse_to_the_newest() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "pileup", None).await;
        for key in ["first", "second", "third"] {
            fx::session(&d, &p, key).await;
        }

        assert_eq!(
            reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT, std::time::Duration::ZERO).await,
            2,
            "both abandoned sessions should have been reaped"
        );

        let still_active = repo::list_sessions(&d.store, p.id)
            .await
            .expect("sessions")
            .into_iter()
            .filter(|s| s.status == SessionStatus::Active)
            .count();
        assert_eq!(
            still_active, 1,
            "a worktree must end up with exactly one active session"
        );
    }

    /// Supersession needs a newer sibling, not merely silence.
    ///
    /// A developer with one session, reading and thinking, is the case the
    /// generous timeout exists for. The short window must not reach it.
    #[tokio::test]
    async fn a_lone_silent_session_is_not_superseded_by_nobody() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "alone", None).await;
        let s = fx::session(&d, &p, "thinking").await;

        assert_eq!(
            reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT, std::time::Duration::ZERO).await,
            0,
            "a session with no newer sibling was reaped by the superseded rule"
        );
        assert_eq!(
            repo::session(&d.store, s.id).await.expect("session").status,
            SessionStatus::Active
        );
    }

    /// A session in a *different* worktree is not a sibling.
    ///
    /// Scope resolution is per worktree: two worktrees are two working contexts
    /// and one must never close the other's session.
    #[tokio::test]
    async fn a_session_in_another_worktree_supersedes_nothing() {
        let d = fx::daemon().await;
        let here = fx::project(&d, "here", None).await;
        let elsewhere = fx::project(&d, "elsewhere", None).await;
        let mine = fx::session(&d, &here, "mine").await;
        fx::session(&d, &elsewhere, "theirs").await;

        assert_eq!(
            reap_idle_sessions(&d, IDLE_SESSION_TIMEOUT, std::time::Duration::ZERO).await,
            0,
            "a session in another worktree was treated as a successor"
        );
        assert_eq!(
            repo::session(&d.store, mine.id)
                .await
                .expect("session")
                .status,
            SessionStatus::Active
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

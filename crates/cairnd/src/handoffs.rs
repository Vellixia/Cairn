//! Producing handoffs from recorded state (FR-032 – FR-035, D7).

use crate::state::{git_status, repo_state, storage_err, Daemon};
use cairn_core::domain::*;
use cairn_core::handoff::{synthesize, HandoffInputs};
use cairn_core::wire::WireError;
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo;
use std::path::PathBuf;
use uuid::Uuid;

/// Generate and store a handoff for `session`.
///
/// Every field is derived; an agent narrative is only ever attached afterwards
/// through `annotate` (FR-034).
pub async fn generate(
    daemon: &Daemon,
    session: &Session,
    trigger: HandoffTrigger,
    policy: SyncPolicy,
) -> Result<Handoff, WireError> {
    generate_inner(daemon, session, trigger, policy, true).await
}

/// The boundary record alone, with no checkpoint of its own.
///
/// Used where the caller writes the checkpoint itself — `cairn session
/// checkpoint` derives a handoff to anchor to and then records one explicitly.
/// Without this the single command would write two checkpoints for one boundary.
pub async fn generate_boundary_record(
    daemon: &Daemon,
    session: &Session,
    trigger: HandoffTrigger,
    policy: SyncPolicy,
) -> Result<Handoff, WireError> {
    generate_inner(daemon, session, trigger, policy, false).await
}

async fn generate_inner(
    daemon: &Daemon,
    session: &Session,
    trigger: HandoffTrigger,
    policy: SyncPolicy,
    write_checkpoint: bool,
) -> Result<Handoff, WireError> {
    // Let captures already in flight land first, so the handoff reports the
    // whole session rather than most of it (H3).
    daemon.quiesce_captures().await;

    let store = &daemon.store;

    let observations = repo::observations_for_session(store, session.id)
        .await
        .map_err(storage_err)?;
    let decision_memories = repo::decision_memories_for_session(store, session.id)
        .await
        .map_err(storage_err)?;
    let task = match session.task_id {
        Some(id) => repo::task(store, id).await.ok(),
        None => None,
    };

    // A worktree that has since disappeared must not stop a handoff being
    // written — the recorded observations are the substance (FR-009).
    let (repository_state, git_changed) =
        match git_status(PathBuf::from(&session.worktree_path)).await {
            Ok(st) => (repo_state(&st), st.changed_files),
            Err(_) => (
                RepositoryState {
                    branch: session.branch.clone(),
                    commit_sha: session.commit_sha.clone(),
                    ..Default::default()
                },
                Vec::new(),
            ),
        };

    let handoff = synthesize(
        &HandoffInputs {
            session,
            task: task.as_ref(),
            observations: &observations,
            decision_memories: &decision_memories,
            repository_state,
            git_changed_files: &git_changed,
            agent_note: None,
        },
        trigger,
    );

    let stored = repo::insert_handoff(store, &handoff, session.project_id, policy)
        .await
        .map_err(storage_err)?;

    // The checkpoint is written inside the same step that produces the handoff,
    // so a boundary that owes a handoff owes its checkpoint with it and the
    // existing pending-handoff sweep covers both (Feature 002 D22, FR-425).
    //
    // A turn checkpoint deliberately gets none: `agent_quiesced` is a turn
    // boundary, not a work boundary (Feature 001 D16).
    if let Some(checkpoint_trigger) = checkpoint_trigger_for(trigger).filter(|_| write_checkpoint) {
        let worktree = std::path::PathBuf::from(&session.worktree_path);
        if let Err(e) = crate::continuity::write(
            daemon,
            session,
            stored.id,
            checkpoint_trigger,
            &worktree,
            &stored.next_step,
        )
        .await
        {
            // A missing checkpoint degrades continuity; it does not fail the
            // boundary. The handoff is already durable.
            tracing::warn!(session = %session.id, error = %e, "checkpoint not written");
        }
    }

    Ok(stored)
}

/// Which handoff triggers are work boundaries that owe a checkpoint.
fn checkpoint_trigger_for(trigger: HandoffTrigger) -> Option<CheckpointTrigger> {
    match trigger {
        HandoffTrigger::SessionEnd => Some(CheckpointTrigger::SessionClosed),
        HandoffTrigger::PreCompact => Some(CheckpointTrigger::ContextCompacting),
        // A handoff synthesized at daemon-start reconciliation describes a
        // boundary that already passed; its worktree state is whatever the
        // machine holds now, which is not what the session assumed. Recording
        // that as an assumption set would manufacture a false comparison.
        HandoffTrigger::Recovered => None,
    }
}

/// How many quick attempts a sealed boundary gets before it is reported as a
/// named, still-retryable failure (FR-240 clause 3).
const QUICK_ATTEMPTS: u32 = 4;

/// The documented bound: a recoverable boundary has its durable handoff inside
/// this interval, and a non-recoverable one has surfaced a named condition
/// inside it. Target under 5 seconds at p99 on a running daemon.
///
/// The quick attempts below are sized to fit well inside it; the constant is
/// the contract SC-136 measures against.
pub const HANDOFF_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

/// Whether the quick retries fit inside the documented bound.
///
/// Asserted rather than assumed: a backoff change that pushed the last attempt
/// past the bound would make FR-240 clause 2 unsatisfiable without any test
/// noticing.
#[cfg(test)]
pub fn quick_attempts_fit_the_bound() -> bool {
    let mut total = std::time::Duration::ZERO;
    let mut backoff = std::time::Duration::from_millis(50);
    for _ in 0..QUICK_ATTEMPTS {
        total += backoff;
        backoff *= 2;
    }
    total < HANDOFF_BOUND
}

/// Synthesize the handoff a sealed boundary owes, retrying with bounded
/// backoff (D22, FR-240).
///
/// Progress is guaranteed while the daemon runs: this task is the primary
/// path, the maintenance tick sweeps anything it gives up on, and daemon-start
/// reconciliation remains the backstop for the process dying between the
/// phases — not the only retry path.
///
/// After the quick attempts, the session is reported as `handoff synthesis
/// failed` with its redacted reason in `cairn status` and doctor's core
/// section, and retried at a slow cadence. It stays retryable and actionable;
/// it is never treated as a terminal outcome that closes the matter.
pub async fn synthesize_pending(daemon: &Daemon, session_id: Uuid, policy: SyncPolicy) {
    settle_before_synthesis().await;
    let mut backoff = std::time::Duration::from_millis(50);
    for attempt in 0..QUICK_ATTEMPTS {
        let session = match repo::session(&daemon.store, session_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "sealed session vanished");
                return;
            }
        };
        // Another path — the sweep, or a restart's reconciliation — may have
        // already produced it. That is a success, not a race to lose.
        if !session_owes_handoff(&session) {
            return;
        }
        match generate(daemon, &session, HandoffTrigger::SessionEnd, policy).await {
            Ok(_) => {
                if let Err(e) = repo::clear_handoff_pending(&daemon.store, session_id).await {
                    tracing::warn!(session = %session_id, error = %e, "could not clear handoff_pending");
                }
                return;
            }
            Err(e) => {
                let attempts = repo::record_handoff_failure(&daemon.store, session_id, &e.message)
                    .await
                    .unwrap_or(attempt as i64 + 1);
                tracing::warn!(
                    session = %session_id,
                    attempts,
                    error = %e.message,
                    "handoff synthesis failed; retrying"
                );
            }
        }
        tokio::time::sleep(backoff).await;
        backoff *= 2;
    }
    tracing::error!(
        session = %session_id,
        "handoff synthesis failed after the quick attempts; reported as owed and retried slowly"
    );
}

/// Give a capture that is still on the socket time to arrive.
///
/// `generate` already quiesces captures the daemon has *accepted* (D22 phase
/// two, Feature 001's own mechanism). This covers the moment before that: a
/// tool call the agent reported immediately before the boundary may not have
/// been read off the socket yet, so it would not be counted as in flight and
/// would miss its own boundary's handoff.
///
/// Deliberately short. A boundary is worth a few milliseconds; a handoff that
/// waits on something which is not coming is worse than one that is one
/// observation short.
async fn settle_before_synthesis() {
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
}

/// Whether this session still owes a durable handoff.
fn session_owes_handoff(session: &Session) -> bool {
    session.handoff_pending
}

#[cfg(test)]
mod bound_tests {
    #[test]
    fn the_quick_retries_fit_inside_the_documented_bound() {
        assert!(super::quick_attempts_fit_the_bound());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport as fx;

    /// Every handoff field is derived from what was recorded (FR-034).
    #[tokio::test]
    async fn a_handoff_is_derived_from_the_session_observations() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "derived", None).await;
        let s = fx::session(&d, &p, "work").await;
        fx::observe_edit(&d, &s, "src/one.rs").await;
        fx::observe_edit(&d, &s, "src/two.rs").await;

        let h = generate(
            &d,
            &s,
            HandoffTrigger::SessionEnd,
            SyncPolicy::from_project(&p),
        )
        .await
        .expect("a handoff is written");

        assert_eq!(h.session_id, s.id);
        assert_eq!(h.trigger, HandoffTrigger::SessionEnd);
        for f in ["src/one.rs", "src/two.rs"] {
            assert!(
                h.changed_files.iter().any(|c| c == f),
                "{f} should appear in {:?}",
                h.changed_files
            );
        }
        assert!(
            h.agent_note.is_none(),
            "a narrative is only ever attached afterwards, through annotate (FR-034)"
        );
    }

    /// A worktree that has since disappeared must not stop a handoff being
    /// written — the recorded observations are the substance (FR-009).
    ///
    /// The fixture's worktree path points at nothing, so this is the path every
    /// test here takes; asserting it explicitly is what makes it a claim rather
    /// than an accident. The session's own recorded branch and commit stand in
    /// for the Git state that can no longer be read.
    #[tokio::test]
    async fn a_missing_worktree_still_produces_a_handoff() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "vanished", None).await;
        let s = fx::session(&d, &p, "gone").await;
        fx::observe_edit(&d, &s, "src/kept.rs").await;

        let h = generate(
            &d,
            &s,
            HandoffTrigger::Recovered,
            SyncPolicy::from_project(&p),
        )
        .await
        .expect("a missing worktree is not a failure");

        assert_eq!(h.repository_state.branch, s.branch);
        assert_eq!(h.repository_state.commit_sha, s.commit_sha);
        assert_eq!(
            (
                h.repository_state.staged,
                h.repository_state.unstaged,
                h.repository_state.untracked
            ),
            (0, 0, 0),
            "unreadable Git state is reported as clean, not guessed"
        );
        assert!(
            h.changed_files.iter().any(|c| c == "src/kept.rs"),
            "what the session recorded survives: {:?}",
            h.changed_files
        );
    }

    /// A session that recorded nothing still gets a boundary record.
    ///
    /// Reconciliation writes a handoff for whatever a crashed session managed
    /// to record, and "nothing" is a legitimate amount to have recorded.
    #[tokio::test]
    async fn a_session_with_no_observations_still_gets_a_handoff() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "empty", None).await;
        let s = fx::session(&d, &p, "silent").await;

        let h = generate(
            &d,
            &s,
            HandoffTrigger::Recovered,
            SyncPolicy::from_project(&p),
        )
        .await
        .expect("an empty session still has a boundary");

        assert!(h.changed_files.is_empty());
        assert!(
            repo::latest_handoff(&d.store, s.id)
                .await
                .expect("query")
                .is_some(),
            "the handoff is stored, not merely returned"
        );
    }

    /// Each call stores another handoff rather than replacing the last.
    ///
    /// A session can cross several boundaries — a compaction, then its end —
    /// and each is a record in its own right.
    #[tokio::test]
    async fn successive_triggers_each_store_their_own_handoff() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "twice", None).await;
        let s = fx::session(&d, &p, "long").await;
        let policy = SyncPolicy::from_project(&p);

        generate(&d, &s, HandoffTrigger::PreCompact, policy)
            .await
            .expect("first");
        generate(&d, &s, HandoffTrigger::SessionEnd, policy)
            .await
            .expect("second");

        let all = repo::handoffs_for_session(&d.store, s.id)
            .await
            .expect("query");
        assert_eq!(all.len(), 2);
        assert_eq!(
            repo::latest_handoff(&d.store, s.id)
                .await
                .expect("query")
                .map(|h| h.trigger),
            Some(HandoffTrigger::SessionEnd),
            "the newest boundary is the one a next session reads"
        );
    }
}

//! Producing handoffs from recorded state (FR-032 – FR-035, D7).

use crate::state::{git_status, repo_state, storage_err, Daemon};
use cairn_core::domain::*;
use cairn_core::handoff::{synthesize, HandoffInputs};
use cairn_core::wire::WireError;
use cairn_store::outbox::SyncPolicy;
use cairn_store::repo;
use std::path::PathBuf;

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

    repo::insert_handoff(store, &handoff, session.project_id, policy)
        .await
        .map_err(storage_err)
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

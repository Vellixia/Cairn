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

//! Assemble the briefing from stored state and Git (FR-027 – FR-031).
//!
//! The budgeting itself lives in `cairn-core`; this is the part that reads the
//! database and the working tree.

use crate::state::{repo_state, Daemon, Resolved};
use cairn_core::context::{assemble, ContextInputs};
use cairn_core::domain::*;
use cairn_core::wire::{codes, ContextPayload, WireError};
use cairn_store::{repo, search};
use uuid::Uuid;

const MEMORY_PER_SCOPE: i64 = 12;

/// Build the briefing for the current working context.
///
/// `degraded` is set by the caller when assembly had to proceed without part of
/// its inputs — the agent session still starts (FR-046).
pub async fn build(
    daemon: &Daemon,
    resolved: &Resolved,
    session: Option<&Session>,
    budget: usize,
    degraded: bool,
) -> Result<ContextPayload, WireError> {
    let store = &daemon.store;
    let project = &resolved.project;

    let git = crate::state::git_status(resolved.repo.worktree_path.clone()).await?;
    let repository = repo_state(&git);

    let task = match session.and_then(|s| s.task_id) {
        Some(id) => repo::task(store, id).await.ok(),
        None => None,
    };

    let previous_handoff = match session {
        Some(s) => repo::previous_handoff_for(store, s.id)
            .await
            .map_err(store_err)?,
        None => latest_handoff_for_branch(daemon, project.id, &git.branch).await?,
    };

    // Decisions and known failures come from the previous handoff, which is
    // itself derived from observations — never from an agent narrative (D7).
    let decisions = previous_handoff
        .as_ref()
        .map(|h| h.decisions.clone())
        .unwrap_or_default();
    let known_failures = previous_handoff
        .as_ref()
        .map(|h| h.failures.clone())
        .unwrap_or_default();

    let task_memory = match task.as_ref() {
        Some(t) => scope_memory(daemon, project.id, MemoryScope::Task, &t.id.to_string()).await?,
        None => Vec::new(),
    };
    let branch_memory = scope_memory(daemon, project.id, MemoryScope::Branch, &git.branch).await?;
    let project_memory = scope_memory(
        daemon,
        project.id,
        MemoryScope::Project,
        &project.id.to_string(),
    )
    .await?;

    let has_history = previous_handoff.is_some()
        || !task_memory.is_empty()
        || !branch_memory.is_empty()
        || !project_memory.is_empty();

    Ok(assemble(
        &ContextInputs {
            project,
            repository,
            task: task.as_ref(),
            previous_handoff: previous_handoff.as_ref(),
            decisions: &decisions,
            known_failures: &known_failures,
            task_memory: &task_memory,
            branch_memory: &branch_memory,
            project_memory: &project_memory,
            has_history,
            degraded,
        },
        budget,
    ))
}

async fn scope_memory(
    daemon: &Daemon,
    project_id: Uuid,
    scope: MemoryScope,
    key: &str,
) -> Result<Vec<String>, WireError> {
    let items = search::memory_for_scope(&daemon.store, project_id, scope, key, MEMORY_PER_SCOPE)
        .await
        .map_err(store_err)?;
    Ok(items
        .into_iter()
        .map(|m| format!("[{}] {}", m.kind, m.content))
        .collect())
}

/// The newest handoff on this branch, for a session that has no predecessor of
/// its own — a new session on a repository with prior history still opens
/// informed (US2 scenario 1).
async fn latest_handoff_for_branch(
    daemon: &Daemon,
    project_id: Uuid,
    branch: &str,
) -> Result<Option<Handoff>, WireError> {
    let sessions = repo::list_sessions(&daemon.store, project_id)
        .await
        .map_err(store_err)?;
    for s in sessions
        .iter()
        .filter(|s| s.branch == branch && !s.is_active())
    {
        if let Some(h) = repo::latest_handoff(&daemon.store, s.id)
            .await
            .map_err(store_err)?
        {
            return Ok(Some(h));
        }
    }
    Ok(None)
}

fn store_err(e: cairn_store::StoreError) -> WireError {
    WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string())
}

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
    explain: bool,
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

    // ---- Level 0 -----------------------------------------------------------
    //
    // Read only what the bound task actually has. A project with no task reads
    // nothing here and pays nothing, which is what keeps the no-regression
    // property true on the daemon path as well as in the assembler (FR-442).
    let (criteria, blockers, blocker_text) = match task.as_ref() {
        Some(t) => level0_task_state(daemon, t.id).await,
        None => (Vec::new(), Vec::new(), Vec::new()),
    };

    let warnings = level0_warnings(daemon, session, task.as_ref()).await;
    let pins = level0_pins(daemon, project.id, &git.branch, task.as_ref()).await;

    let config = daemon.config.read().await.clone();
    let caps = cairn_core::context::Caps {
        goal_max_tokens: config.goal_max_tokens,
        warnings_in_context_max: config.warnings_in_context_max,
        pins_in_context_max: config.pins_in_context_max,
        reserve_fraction: config.min_safe_context_fraction,
    };

    Ok(assemble(
        &ContextInputs {
            level0: cairn_core::context::Level0 {
                criteria: &criteria,
                blockers: &blockers,
                blocker_text: &blocker_text,
                warnings: &warnings,
                pins: &pins,
                previous_next_action: None,
                explain,
                caps,
            },
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

// ---------------------------------------------------------------------------
// Level 0 inputs (`contracts/continuity-context.md` Part 2)
// ---------------------------------------------------------------------------

/// The bound task's criteria and blockers, as the assembler's plain data.
async fn level0_task_state(
    daemon: &Daemon,
    task_id: Uuid,
) -> (
    Vec<cairn_core::tasks::CriterionFacts>,
    Vec<cairn_core::tasks::BlockerFacts>,
    Vec<(Uuid, String)>,
) {
    let facts = match cairn_store::criteria::task_state_facts(&daemon.store, task_id).await {
        Ok(f) => f,
        // A briefing is never refused for a read that failed; it is reported
        // degraded by the caller and the tier is simply absent (FR-046).
        Err(_) => return (Vec::new(), Vec::new(), Vec::new()),
    };
    let text = cairn_store::criteria::blockers(&daemon.store, task_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|b| (b.id, b.description))
        .collect();
    (facts.criteria, facts.blockers, text)
}

/// The critical warnings in force.
///
/// Warnings are Level 0 **content**, not diagnostics: they are here whether or
/// not `explain` was requested (FR-464).
async fn level0_warnings(
    daemon: &Daemon,
    session: Option<&Session>,
    task: Option<&Task>,
) -> Vec<cairn_core::wire::ContextWarning> {
    let mut out = Vec::new();

    // A task that advanced under a session that bound at an earlier state.
    if let (Some(s), Some(t)) = (session, task) {
        let snapshot: Option<String> =
            sqlx::query_scalar("SELECT task_snapshot_at_bind FROM sessions WHERE id = ?1")
                .bind(s.id.to_string())
                .fetch_optional(daemon.store.pool())
                .await
                .ok()
                .flatten();
        if let Some(snapshot) = snapshot {
            if let Ok(changes) =
                cairn_store::criteria::divergence(&daemon.store, t.id, &snapshot).await
            {
                for c in changes.iter().take(4) {
                    out.push(cairn_core::wire::ContextWarning {
                        kind: "task_divergence".into(),
                        subject: c.subject.clone(),
                        detail: format!("{} ({})", c.what, c.origin),
                    });
                }
            }
        }
    }

    // Drifted memories — the claim moved out from under what was remembered.
    if let Ok(drifted) = cairn_store::evidence::drifted_memories(
        &daemon.store,
        task.map(|t| t.project_id).unwrap_or_default(),
        4,
    )
    .await
    {
        for (subject, detail) in drifted {
            out.push(cairn_core::wire::ContextWarning {
                kind: "drift".into(),
                subject,
                detail,
            });
        }
    }

    out
}

/// The pinned constraints applicable here.
///
/// A pin never widens scope: a pinned `branch:feature/x` memory is in force only
/// on that branch (FR-453).
async fn level0_pins(
    daemon: &Daemon,
    project_id: Uuid,
    branch: &str,
    task: Option<&Task>,
) -> Vec<cairn_core::wire::PinnedConstraint> {
    repo::applicable_pins(&daemon.store, project_id, branch, task.map(|t| t.id))
        .await
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport as fx;

    /// A new session on a repository with prior history still opens informed
    /// (US2 scenario 1, FR-027).
    ///
    /// With no predecessor of its own, the briefing falls back to the newest
    /// handoff on the same branch.
    #[tokio::test]
    async fn the_newest_handoff_on_the_branch_is_found() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "history", None).await;

        let first = fx::session_on_branch(&d, &p, "one", "main").await;
        fx::observe_edit(&d, &first, "src/first.rs").await;
        crate::handoffs::generate(
            &d,
            &first,
            HandoffTrigger::SessionEnd,
            cairn_store::outbox::SyncPolicy::from_project(&p),
        )
        .await
        .expect("handoff");
        fx::end(&d, &p, &first).await;

        let found = latest_handoff_for_branch(&d, p.id, "main")
            .await
            .expect("query")
            .expect("a handoff on this branch");
        assert_eq!(found.session_id, first.id);
    }

    /// Handoffs from another branch are not offered.
    ///
    /// Switching branch changes what the next session should be told; carrying
    /// the other branch's handoff over would brief it on the wrong work.
    #[tokio::test]
    async fn a_handoff_on_another_branch_is_not_offered() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "branched", None).await;

        let other = fx::session_on_branch(&d, &p, "elsewhere", "feature/x").await;
        crate::handoffs::generate(
            &d,
            &other,
            HandoffTrigger::SessionEnd,
            cairn_store::outbox::SyncPolicy::from_project(&p),
        )
        .await
        .expect("handoff");
        fx::end(&d, &p, &other).await;

        assert!(
            latest_handoff_for_branch(&d, p.id, "main")
                .await
                .expect("query")
                .is_none(),
            "main has no history of its own"
        );
        assert!(
            latest_handoff_for_branch(&d, p.id, "feature/x")
                .await
                .expect("query")
                .is_some(),
            "but the branch that does keeps it"
        );
    }

    /// An *active* session's handoff is not treated as prior history.
    ///
    /// It belongs to work still in progress — very likely the caller's own
    /// session — so offering it back would brief a session on itself.
    #[tokio::test]
    async fn an_active_sessions_handoff_is_not_prior_history() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "inprogress", None).await;

        let live = fx::session_on_branch(&d, &p, "current", "main").await;
        crate::handoffs::generate(
            &d,
            &live,
            HandoffTrigger::PreCompact,
            cairn_store::outbox::SyncPolicy::from_project(&p),
        )
        .await
        .expect("handoff");
        // Deliberately not ended.

        assert!(
            latest_handoff_for_branch(&d, p.id, "main")
                .await
                .expect("query")
                .is_none(),
            "an active session is not a predecessor"
        );
    }

    /// A repository with no history at all is not an error.
    #[tokio::test]
    async fn no_history_is_reported_as_none_rather_than_failing() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "fresh", None).await;
        assert!(latest_handoff_for_branch(&d, p.id, "main")
            .await
            .expect("query")
            .is_none());
    }

    /// Memory is presented to the agent tagged with its kind, so a decision is
    /// distinguishable from a plain fact in the briefing text.
    #[tokio::test]
    async fn scope_memory_tags_each_item_with_its_kind() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "tagged", None).await;
        let s = fx::session(&d, &p, "author").await;

        repo::create_memory(
            &d.store,
            repo::NewMemory {
                project_id: p.id,
                kind: MemoryType::Decision,
                scope: MemoryScope::Project,
                scope_key: &p.id.to_string(),
                content: "chose a token bucket",
                origin_session_id: s.id,
                local_only: false,
                evidence: &[],
                topic_key: None,
                value_key: None,
                importance: cairn_core::Importance::Normal,
            },
            cairn_store::outbox::SyncPolicy::from_project(&p),
        )
        .await
        .expect("memory");

        let items = scope_memory(&d, p.id, MemoryScope::Project, &p.id.to_string())
            .await
            .expect("scope memory");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0], "[decision] chose a token bucket");
    }

    /// An empty scope yields an empty list, not an error — a project with no
    /// memory still gets a briefing (FR-031).
    #[tokio::test]
    async fn an_empty_scope_yields_no_items() {
        let d = fx::daemon().await;
        let p = fx::project(&d, "bare", None).await;
        assert!(scope_memory(&d, p.id, MemoryScope::Branch, "main")
            .await
            .expect("scope memory")
            .is_empty());
    }
}

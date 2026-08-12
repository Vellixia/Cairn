//! Request dispatch: the daemon's whole behaviour, one function per verb.

use crate::state::{git_branches, git_status, repo_state, storage_err, Daemon, Resolved};
use crate::{briefing, capture, handoffs};
use cairn_core::domain::*;
use cairn_core::wire::*;
use cairn_store::repo;
use cairn_store::search::{self, SearchContext};
use serde_json::json;
use uuid::Uuid;

type Reply = Result<serde_json::Value, WireError>;

pub async fn dispatch(daemon: &Daemon, request: Request) -> Envelope {
    match handle(daemon, request).await {
        Ok(value) => Envelope::ok(value),
        Err(e) => Envelope::err(e),
    }
}

async fn handle(d: &Daemon, request: Request) -> Reply {
    match request {
        Request::DaemonStatus => Ok(json!({
            "running": true,
            "run_id": d.run_id,
            "started_at": d.started_at,
            "schema_version": cairn_store::migrate::latest_version(),
        })),
        Request::DaemonShutdown => Ok(json!({ "stopping": true })),

        Request::Init { cwd } => init(d, &cwd).await,
        Request::Status { cwd } => status(d, &cwd).await,

        Request::SessionStart {
            cwd,
            agent,
            agent_session_key,
            task_id,
        } => session_start(d, &cwd, &agent, agent_session_key, task_id).await,
        Request::SessionList { cwd } => session_list(d, &cwd).await,
        Request::SessionShow {
            cwd,
            session_id,
            agent_session_key,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            Ok(json!({ "session": SessionSummary::from_session(&s, chrono::Utc::now()) }))
        }
        Request::SessionBindTask {
            cwd,
            session_id,
            agent_session_key,
            task_id,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            repo::task(&d.store, task_id).await.map_err(storage_err)?;
            let s = repo::bind_task(&d.store, s.id, task_id)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "session": SessionSummary::from_session(&s, chrono::Utc::now()) }))
        }
        Request::SessionEnd {
            cwd,
            session_id,
            agent_session_key,
            status,
            reason,
            wait_for_handoff,
        } => {
            session_end(
                d,
                &cwd,
                session_id,
                agent_session_key,
                status,
                reason,
                wait_for_handoff,
            )
            .await
        }
        Request::TurnCheckpoint {
            cwd,
            agent_session_key,
        } => turn_checkpoint(d, &cwd, agent_session_key).await,

        Request::Observe {
            cwd,
            agent_session_key,
            observation,
        } => observe(d, &cwd, agent_session_key, observation).await,

        Request::Context {
            cwd,
            agent_session_key,
            session_id,
            reason,
            token_budget,
        } => context(d, &cwd, agent_session_key, session_id, reason, token_budget).await,

        Request::HandoffGenerate {
            cwd,
            session_id,
            agent_session_key,
            trigger,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = match session_id {
                Some(_) => resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?,
                None => resolve_session_for_event(d, &r, agent_session_key.as_deref()).await?,
            };
            let h = handoffs::generate(d, &s, trigger, r.policy).await?;
            Ok(json!({ "handoff": h }))
        }
        Request::HandoffLatest {
            cwd,
            session_id,
            agent_session_key,
        } => handoff_latest(d, &cwd, session_id, agent_session_key).await,
        Request::HandoffAnnotate {
            cwd,
            session_id,
            agent_session_key,
            note,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            let latest = repo::latest_handoff(&d.store, s.id)
                .await
                .map_err(storage_err)?
                .ok_or_else(|| WireError::not_found("handoff"))?;
            // Bounded and clearly attributed; it cannot alter derived fields.
            let note = cairn_core::bound::bound_text(&cairn_core::redact::redact(&note), 2000).text;
            let h = repo::annotate_handoff(&d.store, latest.id, &note)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "handoff": h }))
        }

        Request::TaskList { cwd, status } => {
            let r = d.resolve(&cwd).await?;
            let tasks = repo::list_tasks(&d.store, r.project.id, status)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "tasks": tasks }))
        }
        Request::TaskGet { cwd, task_id } => {
            d.resolve(&cwd).await?;
            let t = repo::task(&d.store, task_id).await.map_err(storage_err)?;
            Ok(json!({ "task": t }))
        }
        Request::TaskCreate {
            cwd,
            title,
            goal,
            acceptance_criteria,
        } => {
            let r = d.resolve(&cwd).await?;
            let t = repo::create_task(
                &d.store,
                r.project.id,
                &title,
                &goal,
                &acceptance_criteria,
                r.policy,
            )
            .await
            .map_err(storage_err)?;
            Ok(json!({ "task": t }))
        }
        Request::TaskUpdate {
            cwd,
            task_id,
            title,
            goal,
            acceptance_criteria,
            status,
        } => {
            let r = d.resolve(&cwd).await?;
            let t = repo::update_task(
                &d.store,
                task_id,
                title.as_deref(),
                goal.as_deref(),
                acceptance_criteria.as_deref(),
                status,
                r.policy,
            )
            .await
            .map_err(storage_err)?;
            Ok(json!({ "task": t }))
        }

        Request::MemoryCreate {
            cwd,
            agent_session_key,
            session_id,
            kind,
            scope,
            scope_key,
            content,
            evidence_observation_ids,
            local_only,
        } => {
            memory_create(
                d,
                &cwd,
                agent_session_key,
                session_id,
                kind,
                scope,
                scope_key,
                content,
                evidence_observation_ids,
                local_only,
                None,
            )
            .await
        }
        Request::MemorySupersede {
            cwd,
            agent_session_key,
            memory_id,
            kind,
            scope,
            scope_key,
            content,
            evidence_observation_ids,
            local_only,
        } => {
            memory_create(
                d,
                &cwd,
                agent_session_key,
                None,
                kind,
                scope,
                scope_key,
                content,
                evidence_observation_ids,
                local_only,
                Some(memory_id),
            )
            .await
        }
        Request::MemoryForget { cwd, memory_id } => {
            let r = d.resolve(&cwd).await?;
            repo::delete_memory(&d.store, memory_id, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "deleted": memory_id }))
        }
        Request::MemoryGet { cwd, memory_id } => {
            d.resolve(&cwd).await?;
            let m = repo::memory(&d.store, memory_id)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "memory": m }))
        }
        Request::MemorySearch {
            cwd,
            agent_session_key,
            session_id,
            query,
        } => memory_search(d, &cwd, agent_session_key, session_id, query).await,

        Request::PrivacyExclude { cwd, path, command } => {
            privacy(d, &cwd, path, command, true).await
        }
        Request::PrivacyUnexclude { cwd, path, command } => {
            privacy(d, &cwd, path, command, false).await
        }
        Request::PrivacyList { .. } => {
            let c = d.config.read().await;
            Ok(json!({ "paths": c.excluded_paths, "commands": c.excluded_commands }))
        }

        Request::Delete {
            cwd,
            target,
            id,
            with_memories,
        } => delete(d, &cwd, target, id, with_memories).await,

        Request::Link {
            cwd,
            server_project_id,
            create,
        } => crate::sync::link(d, &cwd, server_project_id, create).await,
        Request::Unlink { cwd } => {
            let r = d.resolve(&cwd).await?;
            let p = repo::unlink_project(&d.store, r.project.id)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "project": ProjectSummary::from(&p) }))
        }
        Request::AuthTokenSet { token, server_url } => {
            crate::sync::set_token(d, &token, server_url).await
        }
        Request::AuthLogout => crate::sync::logout(d).await,
        Request::AuthStatus => crate::sync::auth_status(d).await,
        Request::SyncStatus { cwd } => crate::sync::status(d, &cwd).await,
        Request::SyncNow { cwd } => crate::sync::sync_now(d, &cwd).await,
    }
}

// ---------------------------------------------------------------------------
// Project and status
// ---------------------------------------------------------------------------

async fn init(d: &Daemon, cwd: &str) -> Reply {
    // `init` is the one place a checkout's identity is worth re-reading.
    d.forget_repo(cwd).await;
    let r = d.resolve(cwd).await?;
    Ok(json!({
        "project": ProjectSummary::from(&r.project),
        "worktree_path": r.worktree(),
        "git_common_dir": r.repo.git_common_dir.display().to_string(),
    }))
}

async fn status(d: &Daemon, cwd: &str) -> Reply {
    let r = d.resolve(cwd).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;
    // Memory scoped to a branch or task that no longer resolves becomes
    // `stale` here, and drops out of default recall (FR-018, H4).
    reconcile_stale(d, &r).await;
    let sessions = repo::list_sessions(&d.store, r.project.id)
        .await
        .map_err(storage_err)?;
    let now = chrono::Utc::now();
    let active: Vec<SessionSummary> = sessions
        .iter()
        .filter(|s| s.is_active())
        .map(|s| SessionSummary::from_session(s, now))
        .collect();

    // What is still owed, so a developer never has to guess whether a
    // boundary completed (FR-240 clause 3).
    let debt = repo::handoff_debt(&d.store).await.map_err(storage_err)?;
    let payload = StatusPayload {
        project: ProjectSummary::from(&r.project),
        repository: repo_state(&git),
        worktree_path: r.worktree(),
        sessions: active,
        integration_mode: integration_mode(&r),
        daemon: "running".into(),
        observation_count: repo::count_observations(&d.store, r.project.id)
            .await
            .map_err(storage_err)?,
        memory_count: repo::count_memories(&d.store, r.project.id)
            .await
            .map_err(storage_err)?,
        server_url: d.server.read().await.url.clone(),
        authenticated: d.server.read().await.token.is_some(),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        sessions_awaiting_handoff: debt.0,
        handoff_synthesis_failures: debt
            .1
            .into_iter()
            .map(|(session_id, reason)| cairn_core::wire::HandoffFailure { session_id, reason })
            .collect(),
    };
    Ok(serde_json::to_value(payload).unwrap_or(json!({})))
}

/// Mark memory whose scope key no longer resolves as `stale` (FR-018).
///
/// Never deletes: a stale memory stays retrievable on request, it just stops
/// being offered by default (US3 scenario 5).
pub async fn reconcile_stale(d: &Daemon, r: &Resolved) {
    let branches = match git_branches(r.repo.worktree_path.clone()).await {
        Ok(b) => b,
        Err(e) => {
            tracing::debug!(error = %e, "could not list branches for stale reconciliation");
            return;
        }
    };
    match repo::mark_stale_scopes(&d.store, r.project.id, &branches).await {
        Ok(n) if n > 0 => tracing::info!(marked = n, "memory marked stale"),
        Ok(_) => {}
        Err(e) => tracing::debug!(error = %e, "stale reconciliation failed"),
    }
}

/// Which mode this repository is operating in (FR-042).
fn integration_mode(r: &Resolved) -> String {
    let settings = r.repo.worktree_path.join(".claude").join("settings.json");
    let hooks_installed = std::fs::read_to_string(&settings)
        .map(|t| t.contains("cairn hook"))
        .unwrap_or(false);
    if hooks_installed {
        "claude-code-hooks".into()
    } else {
        "manual-mcp".into()
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Resolve which session a request is about.
///
/// A worktree may hold several active sessions, so ambiguity is reported
/// rather than guessed (FR-010).
async fn resolve_session(
    d: &Daemon,
    r: &Resolved,
    session_id: Option<Uuid>,
    key: Option<&str>,
) -> Result<Session, WireError> {
    if let Some(id) = session_id {
        return repo::session(&d.store, id).await.map_err(storage_err);
    }
    if let Some(key) = key {
        return repo::session_by_key(&d.store, r.project.id, key)
            .await
            .map_err(storage_err)?
            .ok_or_else(|| {
                WireError::new(
                    codes::NO_ACTIVE_SESSION,
                    format!("no session for agent key {key}"),
                )
            });
    }
    let active = repo::active_sessions_in_worktree(&d.store, r.project.id, &r.worktree())
        .await
        .map_err(storage_err)?;
    match active.len() {
        0 => Err(WireError::new(
            codes::NO_ACTIVE_SESSION,
            "no active session in this worktree; start one with `cairn session start`",
        )),
        1 => Ok(active.into_iter().next().expect("length checked")),
        _ => {
            let ids: Vec<String> = active.iter().map(|s| s.id.to_string()).collect();
            Err(WireError::new(
                codes::AMBIGUOUS_SESSION,
                format!(
                    "{} active sessions here; pass --session: {}",
                    ids.len(),
                    ids.join(", ")
                ),
            ))
        }
    }
}

/// Resolve the session an *event* belongs to, resuming it if it was reconciled
/// at daemon start.
///
/// Rule 4 of D16: a later event proves the session is alive after all, so it
/// returns to `active` under the current run. The handoff already written at
/// reconciliation stands as a valid boundary record. A session the developer
/// deliberately completed is never resurrected.
async fn resolve_session_for_event(
    d: &Daemon,
    r: &Resolved,
    key: Option<&str>,
) -> Result<Session, WireError> {
    let session = resolve_session(d, r, None, key).await?;
    if session.status == SessionStatus::Interrupted {
        return repo::resume_session(&d.store, session.id, d.run_id)
            .await
            .map_err(storage_err);
    }
    Ok(session)
}

async fn session_start(
    d: &Daemon,
    cwd: &str,
    agent: &str,
    agent_session_key: Option<String>,
    task_id: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;
    // An agent with no session identity of its own gets one per connection, so
    // manual MCP mode behaves the same way (data-model.md).
    let key = agent_session_key.unwrap_or_else(|| format!("cairn-local-{}", new_id()));

    let session = repo::start_session(
        &d.store,
        repo::StartSession {
            project_id: r.project.id,
            user_id: d.user_id,
            agent,
            agent_session_key: &key,
            branch: &git.branch,
            commit_sha: git.commit_sha.as_deref(),
            worktree_path: &r.worktree(),
            task_id,
            daemon_run_id: d.run_id,
            policy: r.policy,
        },
    )
    .await
    .map_err(storage_err)?;

    // Starting with a task binds it, including when the session already
    // existed — selecting a task at session start is the documented flow
    // (FR-038).
    let session = match (task_id, session.task_id) {
        (Some(task), None) => {
            repo::task(&d.store, task).await.map_err(storage_err)?;
            repo::bind_task(&d.store, session.id, task)
                .await
                .map_err(storage_err)?
        }
        _ => session,
    };

    Ok(json!({
        "session": SessionSummary::from_session(&session, chrono::Utc::now()),
        "agent_session_key": key,
    }))
}

async fn session_list(d: &Daemon, cwd: &str) -> Reply {
    let r = d.resolve(cwd).await?;
    let now = chrono::Utc::now();
    let sessions: Vec<SessionSummary> = repo::list_sessions(&d.store, r.project.id)
        .await
        .map_err(storage_err)?
        .iter()
        .map(|s| SessionSummary::from_session(s, now))
        .collect();
    Ok(json!({ "sessions": sessions }))
}

/// The sealed close (D22, FR-240).
///
/// Two phases. **Seal**, synchronously, before the reply: one transaction sets
/// the terminal status, the end reason, `ended_at` and `handoff_pending`. No
/// Git, no capture quiesce, no synthesis. **Synthesize**, immediately after:
/// build the handoff, write it, clear `handoff_pending`.
///
/// A caller that waits — `cairn session end` from the command line — gets
/// Feature 001's behavior unchanged, because nothing holds a deadline over it.
/// A hook-driven boundary does not wait: Codex's session-end handler has a
/// one-second default budget, and the Feature 001 path can exceed it, which
/// would make the completion guarantee unprovable rather than merely slow.
async fn session_end(
    d: &Daemon,
    cwd: &str,
    session_id: Option<Uuid>,
    agent_session_key: Option<String>,
    status: SessionStatus,
    reason: Option<String>,
    wait_for_handoff: bool,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;

    // Phase one: durable termination, before anything is acknowledged.
    let sealed = repo::seal_session(&d.store, session.id, status, reason.as_deref(), r.policy)
        .await
        .map_err(storage_err)?;

    if wait_for_handoff {
        // Phase two, inline. The caller asked to wait, so a failure here is
        // reported to it rather than left owed.
        let handoff = handoffs::generate(d, &sealed, HandoffTrigger::SessionEnd, r.policy).await?;
        repo::clear_handoff_pending(&d.store, sealed.id)
            .await
            .map_err(storage_err)?;
        let ended = repo::session(&d.store, sealed.id)
            .await
            .map_err(storage_err)?;
        return Ok(json!({
            "session": SessionSummary::from_session(&ended, chrono::Utc::now()),
            "handoff": handoff,
        }));
    }

    // Phase two, after the reply. Progress is guaranteed while the daemon runs
    // (FR-240 clause 2): this task retries with bounded backoff, and the
    // maintenance tick sweeps anything it gives up on.
    let daemon = d.clone();
    let policy = r.policy;
    let id = sealed.id;
    tokio::spawn(async move {
        crate::handoffs::synthesize_pending(&daemon, id, policy).await;
    });

    Ok(json!({
        "session": SessionSummary::from_session(&sealed, chrono::Utc::now()),
        "handoff_pending": true,
    }))
}

/// `Stop`: the agent finished a turn. The session stays `active` and no
/// durable handoff is produced (FR-032, D16).
async fn turn_checkpoint(d: &Daemon, cwd: &str, agent_session_key: Option<String>) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = resolve_session_for_event(d, &r, agent_session_key.as_deref()).await?;
    let s = repo::turn_checkpoint(&d.store, session.id)
        .await
        .map_err(storage_err)?;
    Ok(json!({
        "session": SessionSummary::from_session(&s, chrono::Utc::now()),
        "handoff": serde_json::Value::Null,
        "turn_checkpoint": true,
    }))
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

async fn observe(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    observation: ObservationInput,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = resolve_session_for_event(d, &r, agent_session_key.as_deref()).await?;
    let config = d.config.read().await.clone();

    let stored = capture::capture(
        &d.store,
        &config,
        capture::CaptureContext {
            session_id: session.id,
            branch: &session.branch,
            commit_sha: session.commit_sha.as_deref(),
        },
        observation,
    )
    .await
    .map_err(storage_err)?;

    repo::touch_session(&d.store, session.id)
        .await
        .map_err(storage_err)?;
    match stored {
        Some(o) => Ok(json!({ "observation_id": o.id, "recorded": true })),
        None => Ok(json!({ "recorded": false, "reason": "excluded" })),
    }
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

async fn context(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    _reason: Option<ContextReason>,
    token_budget: Option<usize>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let budget = token_budget.unwrap_or(d.config.read().await.context_budget_tokens);

    // Which session this briefing is for must be explicit whenever it could be
    // more than one. Picking an arbitrary active session would hand an agent
    // another agent's task goal (FR-010, M1).
    let session = session_for_read(d, &r, session_id, agent_session_key.as_deref()).await?;

    let payload = briefing::build(d, &r, session.as_ref(), budget, false).await?;
    Ok(serde_json::to_value(payload).unwrap_or(json!({})))
}

/// The session a read-only request applies to.
///
/// `None` is a legitimate answer — a briefing for a project with no open
/// session is still useful. Ambiguity is not: it is reported.
async fn session_for_read(
    d: &Daemon,
    r: &Resolved,
    session_id: Option<Uuid>,
    key: Option<&str>,
) -> Result<Option<Session>, WireError> {
    if let Some(id) = session_id {
        return repo::session(&d.store, id)
            .await
            .map(Some)
            .map_err(storage_err);
    }
    if let Some(key) = key {
        return repo::session_by_key(&d.store, r.project.id, key)
            .await
            .map_err(storage_err);
    }
    let active = repo::active_sessions_in_worktree(&d.store, r.project.id, &r.worktree())
        .await
        .map_err(storage_err)?;
    match active.len() {
        0 => Ok(None),
        1 => Ok(active.into_iter().next()),
        _ => {
            let ids: Vec<String> = active.iter().map(|s| s.id.to_string()).collect();
            Err(WireError::new(
                codes::AMBIGUOUS_SESSION,
                format!(
                    "{} sessions are active in this worktree; pass --session or \
                     agent_session_key: {}",
                    ids.len(),
                    ids.join(", ")
                ),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Handoffs
// ---------------------------------------------------------------------------

async fn handoff_latest(
    d: &Daemon,
    cwd: &str,
    session_id: Option<Uuid>,
    agent_session_key: Option<String>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = match (session_id, agent_session_key.as_deref()) {
        (None, None) => most_recent_session(d, &r).await?,
        _ => resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?,
    };
    let handoff = repo::latest_handoff(&d.store, session.id)
        .await
        .map_err(storage_err)?
        .ok_or_else(|| WireError::not_found(format!("handoff for session {}", session.id)))?;
    Ok(json!({ "handoff": handoff, "session_id": session.id }))
}

/// The newest session in this project, active or not — what `cairn handoff
/// show` means with no arguments.
async fn most_recent_session(d: &Daemon, r: &Resolved) -> Result<Session, WireError> {
    repo::list_sessions(&d.store, r.project.id)
        .await
        .map_err(storage_err)?
        .into_iter()
        .next()
        .ok_or_else(|| WireError::new(codes::NO_ACTIVE_SESSION, "this project has no sessions yet"))
}

// ---------------------------------------------------------------------------
// Memory
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn memory_create(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    kind: MemoryType,
    scope: Option<MemoryScope>,
    scope_key: Option<String>,
    content: String,
    evidence: Vec<Uuid>,
    local_only: bool,
    supersedes: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    // A memory needs an origin session, and only that. Evidence is optional and
    // is never fabricated (FR-019).
    let session = ensure_session_for_memory(d, &r, session_id, agent_session_key).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;

    let (scope, key) = resolve_scope(&r, &session, &git.branch, scope, scope_key)?;
    let content = cairn_core::redact::redact(&content);

    let new = repo::NewMemory {
        project_id: r.project.id,
        kind,
        scope,
        scope_key: &key,
        content: &content,
        origin_session_id: session.id,
        local_only,
        evidence: &evidence,
    };

    match supersedes {
        Some(original) => {
            let (old, new) = repo::supersede_memory(&d.store, original, new, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "memory": new, "superseded": old.id }))
        }
        None => {
            let m = repo::create_memory(&d.store, new, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "memory": m }))
        }
    }
}

/// Recording memory should not require the caller to have started a session
/// first; one is opened on demand so provenance is always real.
///
/// Only genuine absence opens one. Swallowing every error here also swallowed
/// `ambiguous_session`, which meant a second agent in the same worktree quietly
/// got a throwaway session — worsening the ambiguity for everyone else and
/// stamping the memory with an origin that never did the work. Ambiguity is the
/// caller's to resolve, exactly as it is for `cairn context`.
async fn ensure_session_for_memory(
    d: &Daemon,
    r: &Resolved,
    session_id: Option<Uuid>,
    key: Option<String>,
) -> Result<Session, WireError> {
    match resolve_session(d, r, session_id, key.as_deref()).await {
        Ok(s) => return Ok(s),
        Err(e) if e.code != codes::NO_ACTIVE_SESSION => return Err(e),
        Err(_) => {}
    }
    let git = git_status(r.repo.worktree_path.clone()).await?;
    let key = key.unwrap_or_else(|| format!("cairn-cli-{}", new_id()));
    repo::start_session(
        &d.store,
        repo::StartSession {
            project_id: r.project.id,
            user_id: d.user_id,
            agent: "cairn-cli",
            agent_session_key: &key,
            branch: &git.branch,
            commit_sha: git.commit_sha.as_deref(),
            worktree_path: &r.worktree(),
            task_id: None,
            daemon_run_id: d.run_id,
            policy: r.policy,
        },
    )
    .await
    .map_err(storage_err)
}

fn resolve_scope(
    r: &Resolved,
    session: &Session,
    branch: &str,
    scope: Option<MemoryScope>,
    scope_key: Option<String>,
) -> Result<(MemoryScope, String), WireError> {
    let scope = scope.unwrap_or(if session.task_id.is_some() {
        MemoryScope::Task
    } else {
        MemoryScope::Branch
    });
    let key = match (scope, scope_key) {
        (_, Some(k)) => k,
        (MemoryScope::Project, None) => r.project.id.to_string(),
        (MemoryScope::Branch, None) => branch.to_string(),
        (MemoryScope::Task, None) => session
            .task_id
            .ok_or_else(|| WireError::invalid("task scope needs a bound task or a scope key"))?
            .to_string(),
        (MemoryScope::Session, None) => session.id.to_string(),
    };
    Ok((scope, key))
}

async fn memory_search(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    query: MemoryQuery,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;
    // Same rule as the briefing: explicit, or unambiguous, or reported (M1).
    let session = session_for_read(d, &r, session_id, agent_session_key.as_deref()).await?;

    let ctx = SearchContext {
        branch: Some(git.branch.clone()),
        task_id: session.as_ref().and_then(|s| s.task_id),
        session_id: session.as_ref().map(|s| s.id),
    };
    let results = search::search(&d.store, r.project.id, &query, &ctx)
        .await
        .map_err(storage_err)?;
    let total = results.len();
    Ok(serde_json::to_value(SearchPayload { results, total }).unwrap_or(json!({})))
}

// ---------------------------------------------------------------------------
// Privacy and deletion
// ---------------------------------------------------------------------------

async fn privacy(
    d: &Daemon,
    _cwd: &str,
    path: Option<String>,
    command: Option<String>,
    add: bool,
) -> Reply {
    if path.is_none() && command.is_none() {
        return Err(WireError::invalid("give --path or --command"));
    }
    let mut config = d.config.write().await;
    if let Some(p) = path {
        if add {
            if !config.excluded_paths.contains(&p) {
                config.excluded_paths.push(p);
            }
        } else {
            config.excluded_paths.retain(|x| x != &p);
        }
    }
    if let Some(c) = command {
        if add {
            if !config.excluded_commands.contains(&c) {
                config.excluded_commands.push(c);
            }
        } else {
            config.excluded_commands.retain(|x| x != &c);
        }
    }
    config
        .save()
        .map_err(|e| WireError::new(codes::STORAGE_UNAVAILABLE, e.to_string()))?;
    Ok(json!({ "paths": config.excluded_paths, "commands": config.excluded_commands }))
}

/// Scoped deletion. Removing a session never removes the durable knowledge it
/// produced unless the caller explicitly asks (FR-052).
async fn delete(
    d: &Daemon,
    cwd: &str,
    target: DeleteTarget,
    id: Uuid,
    with_memories: bool,
) -> Reply {
    let r = d.resolve(cwd).await?;
    match target {
        DeleteTarget::Observation => {
            repo::delete_observation(&d.store, id)
                .await
                .map_err(storage_err)?;
        }
        DeleteTarget::Memory => {
            repo::delete_memory(&d.store, id, r.policy)
                .await
                .map_err(storage_err)?;
        }
        DeleteTarget::Handoff => {
            repo::delete_handoff(&d.store, id, r.project.id, r.policy)
                .await
                .map_err(storage_err)?;
        }
        DeleteTarget::Session => {
            repo::delete_session(&d.store, id, with_memories, r.policy)
                .await
                .map_err(storage_err)?;
        }
    }
    Ok(json!({ "deleted": id, "target": target, "with_memories": with_memories }))
}

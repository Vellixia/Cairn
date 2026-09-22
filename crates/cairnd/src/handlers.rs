//! Request dispatch: the daemon's whole behaviour, one function per verb.

use crate::state::{git_status, storage_err, Daemon, Resolved};
use crate::{briefing, capture, handoffs};
use cairn_core::domain::*;
use cairn_core::wire::*;
use cairn_store::repo;
use serde_json::json;
use uuid::Uuid;

type Reply = Result<serde_json::Value, WireError>;

pub async fn dispatch(daemon: &Daemon, request: Request) -> Envelope {
    match handle(daemon, request).await {
        Ok(value) => Envelope::ok(value),
        Err(e) => Envelope::err(e),
    }
}

pub(crate) async fn handle(d: &Daemon, request: Request) -> Reply {
    match request {
        Request::DaemonStatus => Ok(json!({
            "running": true,
            "run_id": d.run_id,
            "started_at": d.started_at,
            "schema_version": cairn_store::migrate::latest_version(),
        })),
        Request::DaemonShutdown => Ok(json!({ "stopping": true })),

        Request::CaptureVocabulary {
            cwd,
            agent,
            agent_session_key,
        } => capture_vocabulary(d, &cwd, &agent, &agent_session_key).await,
        Request::CaptureEvents {
            cwd,
            agent,
            agent_session_key,
            output,
        } => {
            spool_capture(d, &cwd, &agent, &agent_session_key, &output).await?;
            Ok(json!({ "accepted": true }))
        }

        Request::Init { cwd } => init(d, &cwd).await,

        Request::SessionStart {
            cwd,
            agent,
            agent_session_key,
        } => session_start(d, &cwd, &agent, agent_session_key).await,
        Request::SessionShow {
            cwd,
            session_id,
            agent_session_key,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;
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
        // The daemon's single lifecycle entry point (FR-112).
        Request::CanonicalEvent {
            event,
            wait_for_handoff,
            token_budget,
            capture,
        } => {
            crate::integrations::canonical_event(d, event, wait_for_handoff, token_budget, capture)
                .await
        }

        Request::IntegrationEvidence {
            cwd,
            agent,
            capability,
            evidence,
            agent_version,
            degraded,
        } => {
            d.resolve(&cwd).await?;
            crate::integrations::record_evidence(
                d,
                agent,
                capability,
                evidence,
                agent_version,
                degraded,
            )
            .await
        }
        Request::Context {
            cwd,
            agent_session_key,
            session_id,
            reason,
            token_budget,
            explain,
            depth,
            trigger,
            open_trigger,
        } => {
            context(
                d,
                &cwd,
                agent_session_key,
                session_id,
                reason,
                token_budget,
                explain,
                depth,
                trigger,
                open_trigger,
            )
            .await
        }

        // The daemon's own report of what happened to a generated briefing,
        // forwarded to the server (T072, `contracts/retrieval-delivery.md`
        // §3, §6.2). No project or session to resolve here: the trace already
        // carries both, and the server is what checks this account still owns
        // it.
        Request::RetrievalOutcome {
            trace_id,
            transmitted,
            failure_reason,
        } => {
            crate::deliver::report_outcome(d, trace_id, transmitted, failure_reason.as_deref())
                .await;
            Ok(json!({ "reported": true }))
        }

        Request::SessionCheckpoint {
            cwd,
            agent_session_key,
            session_id,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;

            // A checkpoint anchors to a handoff. When none exists yet, one is
            // derived first rather than refusing with `no_boundary_record` —
            // asking for a checkpoint is a reasonable thing to do at any point,
            // and the boundary record is Cairn's job to produce (FR-425).
            let handoff = match repo::latest_handoff(&d.store, s.id)
                .await
                .map_err(storage_err)?
            {
                Some(h) => h,
                None => {
                    handoffs::generate_boundary_record(d, &s, HandoffTrigger::PreCompact, r.policy)
                        .await?
                }
            };

            let worktree = std::path::PathBuf::from(r.worktree());
            let checkpoint = crate::continuity::write(
                d,
                &s,
                handoff.id,
                CheckpointTrigger::Explicit,
                &worktree,
                &handoff.next_step,
            )
            .await?;

            Ok(json!({
                "checkpoint": {
                    "id": checkpoint.id,
                    "handoff_id": checkpoint.handoff_id,
                    "trigger": checkpoint.trigger,
                    "assumed": checkpoint.assumed,
                    "next_action": checkpoint.next_action,
                    "relevant_paths": checkpoint.assumed.path_fingerprints.len(),
                }
            }))
        }

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

        Request::MemoryPin {
            cwd,
            agent_session_key,
            session_id,
            memory_id,
            pinned,
            reason,
        } => {
            let r = d.resolve(&cwd).await?;
            let _ = (agent_session_key, reason);
            queue_knowledge_command(d, Some(r.project.id), session_id,
                cairn_store::spool::CommandKind::Pin,
                &json!({ "target_id": memory_id, "pinned": pinned })).await
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
            topic_key,
            value_key,
            importance: _,
            domain,
        } => match domain {
            // FR-455, FR-527: no MCP action authors team knowledge directly.
            // Team is reached only by `cairn team propose` or by
            // `action: "promote", target: "team"` — never by `create`.
            Some(KnowledgeDomain::Team) => Err(WireError::invalid(
                "domain: \"team\" cannot be created through cairn_remember; team knowledge \
                 is reached only by proposal (`cairn team propose`) or by \
                 `action: \"promote\", target: \"team\"` — no MCP action authors \
                 authoritative team policy directly",
            )),
            Some(KnowledgeDomain::Personal) => personal_create(d, &cwd, kind, content, topic_key, value_key).await,
            None | Some(KnowledgeDomain::Project) => {
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
                    SubjectProposal {
                        topic_key,
                        value_key,
                    },
                )
                .await
            }
        },
        Request::MemorySupersede {
            cwd,
            agent_session_key,
            session_id,
            memory_id,
            kind,
            scope,
            scope_key,
            content,
            evidence_observation_ids,
            local_only,
            topic_key,
            value_key,
            importance: _,
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
                Some(memory_id),
                SubjectProposal {
                    topic_key,
                    value_key,
                },
            )
            .await
        }
        Request::MemoryReinforce {
            cwd,
            agent_session_key,
            session_id,
            memory_id,
            from_memory_id,
        } => {
            memory_reinforce(
                d,
                &cwd,
                agent_session_key,
                session_id,
                memory_id,
                from_memory_id,
            )
            .await
        }
        Request::MemoryReconcile {
            cwd,
            agent_session_key,
            session_id,
            from_memory_id,
            to_memory_id,
            relation,
            basis,
            basis_evidence_id,
            rationale,
        } => {
            memory_reconcile(
                d,
                &cwd,
                agent_session_key,
                session_id,
                from_memory_id,
                to_memory_id,
                relation,
                basis,
                basis_evidence_id,
                rationale,
            )
            .await
        }
        Request::EvidenceAdd {
            cwd,
            agent_session_key,
            session_id,
            kind,
            collector,
            subject,
            observed_value,
            source_locator,
            observation_id,
            memory_id,
            role,
        } => {
            evidence_add(
                d,
                &cwd,
                agent_session_key,
                session_id,
                kind,
                collector,
                subject,
                observed_value,
                source_locator,
                observation_id,
                memory_id,
                role,
            )
            .await
        }
        Request::Verify {
            cwd,
            memory_id,
            all,
            explain,
        } => verify_now(d, &cwd, memory_id, all, explain).await,
        Request::MemoryForget {
            cwd,
            memory_id,
            domain,
        } => match domain {
            // A team entry's lifecycle only advances through `cairn team
            // retire`, by an admin (`contracts/global-memory.md` §5b) — never
            // through this tool.
            Some(KnowledgeDomain::Team) => Err(WireError::invalid(
                "domain: \"team\" cannot be forgotten through cairn_remember; \
                 use `cairn team retire` (admin only)",
            )),
            Some(KnowledgeDomain::Personal) => queue_knowledge_command(d, None, None,
                cairn_store::spool::CommandKind::PersonalForget, &json!({ "target_id": memory_id })).await,
            None | Some(KnowledgeDomain::Project) => {
                let r = d.resolve(&cwd).await?;
                queue_knowledge_command(d, Some(r.project.id), None,
                    cairn_store::spool::CommandKind::Forget, &json!({ "target_id": memory_id })).await
            }
        },
        Request::MemorySearch {
            cwd,
            agent_session_key,
            session_id,
            query,
        } => memory_search(d, &cwd, agent_session_key, session_id, query).await,
        Request::Graph {
            cwd,
            memory_id,
            hops,
        } => server_graph(d, &cwd, memory_id, hops).await,
        Request::Replay { cwd } => server_replay(d, &cwd).await,
        Request::Governance { cwd } => {
            // Resolve caller's repository before a server-wide governance read;
            // an arbitrary cwd must not become an authenticated control path.
            d.resolve(&cwd).await?;
            crate::sync::client(d)
                .await?
                .get("/api/team/knowledge?limit=50")
                .await
        }

    }
}

// ---------------------------------------------------------------------------
// Project and status
// ---------------------------------------------------------------------------

async fn init(d: &Daemon, cwd: &str) -> Reply {
    // `init` is the one place a checkout's identity is worth re-reading.
    d.forget_repo(cwd).await;
    let r = d.resolve(cwd).await?;
    let legacy_migration = migrate_removed_feature_tasks(d).await;
    Ok(json!({
        "project": ProjectSummary::from(&r.project),
        "worktree_path": r.worktree(),
        "git_common_dir": r.repo.git_common_dir.display().to_string(),
        "legacy_migration": legacy_migration,
    }))
}

/// Setup is the sole automatic migration boundary. Failure is a warning: the
/// already-open store remains usable for safe capture, and no legacy row is
/// removed until backup, manifest, and removed-feature bundle all verify.
async fn migrate_removed_feature_tasks(d: &Daemon) -> serde_json::Value {
    migrate_removed_feature_tasks_at(
        d,
        &cairn_core::paths::home().join("removed_feature").join("tasks-v1"),
    )
    .await
}

async fn migrate_removed_feature_tasks_at(d: &Daemon, dir: &std::path::Path) -> serde_json::Value {
    use cairn_store::transfer;

    let pending = match transfer::removed_feature_tasks_pending(&d.store).await {
        Ok(value) => value,
        Err(error) => return json!({ "status": "warning", "detail": error.to_string() }),
    };
    if !pending {
        return json!({ "status": "not_pending" });
    }
    if let Err(error) = std::fs::create_dir_all(dir) {
        return json!({ "status": "warning", "detail": error.to_string() });
    }
    let snapshot = dir.join("legacy.sqlite");
    let manifest_path = dir.join("legacy.manifest.json");
    let bundle_path = dir.join("removed_feature.json");
    if let Ok(Some((path, _))) = transfer::removed_feature_tasks_exported_pending_cleanup(&d.store).await {
        let bundle_path = std::path::PathBuf::from(path);
        return match transfer::cleanup_removed_feature_tasks(&d.store).await {
            Ok(()) => json!({ "status": "exported_cleaned", "backup": snapshot, "manifest": manifest_path, "bundle": bundle_path, "detail": "resumed cleanup" }),
            Err(error) => json!({ "status": "warning", "backup": snapshot, "manifest": manifest_path, "bundle": bundle_path, "detail": error.to_string() }),
        };
    }
    let result = async {
        let manifest = transfer::export_snapshot(&d.store, &snapshot).await?;
        transfer::write_manifest(&manifest, &manifest_path)?;
        let bundle = transfer::export_removed_feature_tasks(&d.store, &bundle_path).await?;
        transfer::cleanup_removed_feature_tasks(&d.store).await?;
        Ok::<_, cairn_store::StoreError>(bundle)
    }
    .await;
    match result {
        Ok(bundle) => json!({
            "status": "exported_cleaned",
            "backup": snapshot,
            "manifest": manifest_path,
            "bundle": bundle_path,
            "records": bundle.records.len(),
        }),
        Err(error) => json!({
            "status": "warning",
            "backup": snapshot,
            "manifest": manifest_path,
            "bundle": bundle_path,
            "detail": error.to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Resolve which session a request is about.
///
/// A worktree may hold several active sessions, so ambiguity is reported
/// rather than guessed (FR-010).
/// The capture agent one adapter name denotes.
///
/// The two vocabularies spell the same agent differently — `AgentId` uses
/// hyphens because that is what a command line reads well, `EventAgent` uses
/// underscores because that is what a key-shaped wire value reads well — and
/// this is the one place the two meet. Both spellings are accepted so a caller
/// need not know which side of the boundary it is on.
pub(crate) fn event_agent(name: &str) -> Option<cairn_core::event::EventAgent> {
    use cairn_core::event::EventAgent;
    match name {
        "claude-code" | "claude_code" => Some(EventAgent::ClaudeCode),
        "codex" => Some(EventAgent::Codex),
        "opencode" => Some(EventAgent::OpenCode),
        // `generic-mcp` is not part of the automatic capture population
        // (FR-838f) and its adapter produces nothing, so it never reaches here.
        _ => None,
    }
}

/// The vocabulary a hook needs before it can build a semantic signal.
///
/// The hook holds the transient vendor text and the daemon holds the event
/// stream, and neither can do the §13.7 mapping alone. Sending the text here
/// would put a prompt fragment across the capture-process boundary, which
/// FR-730 forbids, so the derived token set travels the other way instead. It
/// discloses nothing new: every token in it is a path segment, a command verb,
/// a test identifier or an established project key that anyone who can read the
/// project can already see.
///
/// A session that does not exist yet answers with an empty vocabulary rather
/// than an error. The first event of a session legitimately arrives before any
/// event has established anything, and an error there would make the hook treat
/// an ordinary case as a failure.
async fn capture_vocabulary(d: &Daemon, cwd: &str, agent: &str, key: &str) -> Reply {
    let _ = agent;
    let r = d.resolve(cwd).await?;
    let session = repo::session_by_key(&d.store, r.project.id, key)
        .await
        .map_err(storage_err)?;
    let Some(session) = session else {
        return Ok(
            json!({ "vocabulary": cairn_core::vocabulary::SessionVocabulary::new(),
                          "established_values": {} }),
        );
    };
    let (vocabulary, established) =
        crate::capture::session_vocabulary(&d.store, r.project.id, session.id)
            .await
            .map_err(storage_err)?;
    Ok(json!({ "vocabulary": vocabulary, "established_values": established }))
}

/// Spool one vendor event's approved canonical events.
///
/// Account-bound and it fails closed. The claim predicate matches an account
/// exactly, so a row spooled with no account could never be claimed by anyone —
/// queueing one would be a silent black hole rather than a queued event
/// (FR-790, FR-864a). Capture is fail-soft toward the *agent*, never toward the
/// truth: the decline is counted rather than hidden.
pub(crate) async fn spool_capture(
    d: &Daemon,
    cwd: &str,
    agent: &str,
    key: &str,
    output: &cairn_core::event::CaptureOutput,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = resolve_session_for_event(d, &r, Some(key)).await?;

    // The adapter that ran, named by the caller. An agent Feature 005 does not
    // capture from reaches here only if a caller invented the name, and it is
    // refused rather than filed under a neighbour.
    let Some(agent) = event_agent(agent) else {
        return Err(WireError::invalid(format!(
            "{agent} is not an agent Feature 005 captures from"
        )));
    };

    let Some(account_id) = d.account_identity().await else {
        // Counted, not silent. An unsigned-in machine still produces capture,
        // and a health report that could not tell "nothing happened" from
        // "nobody was signed in" would be reporting the wrong problem.
        for draft in &output.events {
            cairn_store::spool::record_disposition(
                &d.store,
                r.project.id,
                agent.as_str(),
                draft.kind.as_str(),
                cairn_core::event::Disposition::DeclinedByPolicy,
            )
            .await
            .map_err(storage_err)?;
        }
        return Ok(json!({
            "spooled": 0,
            "declined": output.events.len(),
            "reason": "no account is signed in, so a spooled event could never be delivered",
        }));
    };

    let summary = crate::capture::spool_safe_events(
        &d.store,
        r.project.id,
        account_id,
        session.id,
        agent,
        output,
    )
    .await
    .map_err(storage_err)?;
    let _ = agent;

    Ok(json!({
        "spooled": summary.spooled,
        "declined": summary.declined,
        "overflow_dropped": summary.overflow_dropped,
        "saturated": summary.saturated,
    }))
}

pub(crate) async fn resolve_session(
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
        _ => Err(ambiguous_session(&active)),
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
            daemon_run_id: d.run_id,
            policy: r.policy,
        },
    )
    .await
    .map_err(storage_err)?;

    Ok(json!({
        "session": SessionSummary::from_session(&session, chrono::Utc::now()),
        "agent_session_key": key,
    }))
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
pub(crate) async fn turn_checkpoint(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
) -> Reply {
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

pub(crate) async fn observe(
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

    // Drift marking rides the capture path (T063). It is one indexed lookup by
    // exact locator, capped at `evidence_lookups_per_event_max`, and it writes
    // exactly `verification` on the memories the fact supports. Exceeding the
    // cap defers to the background pass and is not an error, which is what
    // keeps a hook inside Feature 001's 250 ms deadline with its always-exit-0
    // rule unchanged (FR-374, FR-475).
    if let Some(o) = &stored {
        if o.kind == ObservationType::FileChanged {
            if let Some(path) = o.path.as_deref() {
                let report = crate::drift::mark_for_path(d, r.project.id, path).await;
                if report.marked > 0 {
                    tracing::debug!(
                        path,
                        marked = report.marked,
                        deferred = report.deferred,
                        "marked claims for recheck"
                    );
                }
            }
        }
    }

    match stored {
        Some(o) => Ok(json!({ "observation_id": o.id, "recorded": true })),
        None => Ok(json!({ "recorded": false, "reason": "excluded" })),
    }
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/// Ten arguments, three past the lint's limit, and each one is read.
///
/// `reason` decides the post-compaction path; `depth` decides whether the global
/// sections are assembled at all (FR-477); `trigger`/`open_trigger` decide
/// whether this retrieval goes through the server and as what
/// (`contracts/retrieval-delivery.md` §1–§3); the rest were already
/// load-bearing. Bundling them into a request struct would only move the same
/// values behind one name — this function's caller destructures them straight
/// out of `Request::Context`, so a struct would be that variant with a second
/// name.
#[allow(clippy::too_many_arguments)]
async fn context(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    reason: Option<ContextReason>,
    token_budget: Option<usize>,
    explain: bool,
    depth: Option<cairn_core::wire::ContextDepth>,
    trigger: Option<String>,
    open_trigger: Option<String>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let budget = token_budget.unwrap_or(d.config.read().await.context_budget_tokens);

    // Which session this briefing is for must be explicit whenever it could be
    // more than one. Picking an arbitrary active session would hand an agent
    // another agent's session context (FR-010, M1).
    let session = session_for_read(d, &r, session_id, agent_session_key.as_deref()).await?;

    // Absent means `standard` — today's full assembly — so a caller that has
    // never named `depth` sees no change (FR-481, T156).
    let depth = depth.unwrap_or(cairn_core::wire::ContextDepth::Standard);

    let mut out = match (explain, session.as_ref()) {
        // `--explain` diagnoses the daemon's own local assembly and its
        // reasons; the server's `sections` carry a `selection_rule` of their
        // own but no per-reader diagnostic to merge with it, so this stays a
        // purely local read exactly as it was before Feature 005 US2
        // (`contracts/retrieval-delivery.md` §8 keeps a *reason* out of the
        // trace for the parallel cause).
        (true, _) => {
            let payload = briefing::build(
                d,
                &r,
                session.as_ref(),
                briefing::Assembly::local(budget, depth).explaining(true),
            )
            .await?;
            serde_json::to_value(payload).unwrap_or(json!({}))
        }
        // No session bound in this worktree: `/api/retrieve` requires one to
        // bind to, and there is none, so this is the daemon's own local
        // assembly exactly as it always was (FR-031).
        (false, None) => {
            let payload =
                briefing::build(d, &r, None, briefing::Assembly::local(budget, depth)).await?;
            serde_json::to_value(payload).unwrap_or(json!({}))
        }
        (false, Some(s)) => {
            let trigger = trigger
                .as_deref()
                .map(crate::deliver::Trigger::parse)
                .unwrap_or(crate::deliver::Trigger::Explicit);
            let deadline =
                std::time::Duration::from_millis(d.config.read().await.context_deadline_ms);
            let delivered = crate::deliver::deliver(
                d,
                &r,
                s.id,
                trigger,
                open_trigger.as_deref(),
                budget,
                deadline,
            )
            .await;
            // These three already travel inside `delivered.payload` too
            // (a caller that only sees the wire reply, such as the hook
            // process, has no other way to read them) — logged here as well
            // because this is the one place a server outage or a degraded
            // level is otherwise silent on the daemon's own side.
            tracing::debug!(
                trace_id = ?delivered.trace_id,
                degradation_level = %delivered.degradation_level,
                served_from_cache = delivered.served_from_cache,
                "server-side retrieval delivered"
            );
            let mut payload = delivered.payload;
            // FR-477: `minimum` excludes both global sections entirely,
            // unconditionally. `deliver` has no `depth` parameter of its own
            // — the merge is identical at every depth — so the gate is
            // enforced here, on the merged result, instead of before the
            // fetch. The server has no notion of `depth` either, so this is
            // the only place the guarantee can live regardless.
            if depth.is_minimum() {
                if let Some(briefing) = payload.get_mut("briefing").and_then(|b| b.as_object_mut())
                {
                    briefing.remove("personal_notes");
                    briefing.remove("team_guidance");
                }
            }
            payload
        }
    };

    // The mode Cairn can honestly promise this agent — derived from Feature
    // 002's capability profile, never from a capability of its own (FR-426).
    if let Some(mode) = continuity_mode(d, session.as_ref()).await {
        if let Some(o) = out.as_object_mut() {
            o.insert("continuity_mode".into(), json!(mode));
        }
    }

    // A post-compaction refresh is where a checkpoint is restored.
    if reason == Some(ContextReason::PostCompaction) {
        if let Some(restored) = restore_checkpoint(d, &r, session.as_ref()).await {
            if let Some(o) = out.as_object_mut() {
                o.insert("checkpoint".into(), restored);
            }
        }
    }

    Ok(out)
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
        _ => Err(ambiguous_session(&active)),
    }
}

/// Report the ambiguity with enough to settle it.
///
/// The ids alone name the candidates but say nothing about which one the caller
/// wants. Naming each session's agent and how long it has been silent is what
/// makes the answer obvious in the case that actually occurs: an agent that was
/// restarted rather than exited leaves its old session active and silent, and
/// the live one is the one that just spoke (#41).
fn ambiguous_session(active: &[Session]) -> WireError {
    let now = chrono::Utc::now();
    let described: Vec<String> = active
        .iter()
        .map(|s| {
            let quiet_for = (now - s.last_event_at).num_minutes().max(0);
            format!("{} ({}, silent {quiet_for}m)", s.id, s.agent)
        })
        .collect();
    WireError::new(
        codes::AMBIGUOUS_SESSION,
        format!(
            "{} sessions are active in this worktree; pass --session or \
             agent_session_key: {}",
            described.len(),
            described.join(", ")
        ),
    )
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

/// The subject identity a caller proposed, carried as one value so the create
/// path does not grow three more positional arguments.
///
/// Every field optional: a caller that supplies none receives Feature 001
/// behaviour exactly, and the memory is stored free-form (FR-313, FR-497).
#[derive(Debug, Clone, Default)]
pub struct SubjectProposal {
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Evidence and verification (T057)
// ---------------------------------------------------------------------------

/// Render one fact for output, with its value already redacted and bounded at
/// the point it was stored.
#[cfg(any())]
fn evidence_json(f: &cairn_store::evidence::EvidenceFact) -> serde_json::Value {
    json!({
        "id": f.id,
        "kind": f.kind,
        "collector": f.collector,
        "subject": f.subject,
        "observed_value": f.observed_value,
        "source_locator": f.source_locator,
        "repo_branch": f.repo_branch,
        "repo_commit": f.repo_commit,
        "collected_by_session": f.collected_by_session,
        "observation_id": f.observation_id,
        // A deleted fact resolves as deleted rather than disappearing
        // (FR-358, FR-505).
        "deleted": f.deleted,
    })
}

#[allow(clippy::too_many_arguments)]
#[cfg(any())]
async fn evidence_add(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    kind: EvidenceKind,
    collector: Option<EvidenceCollector>,
    subject: String,
    observed_value: String,
    source_locator: String,
    observation_id: Option<Uuid>,
    memory_id: Option<Uuid>,
    role: Option<EvidenceRole>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = ensure_session_for_memory(d, &r, session_id, agent_session_key).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;
    let config = d.config.read().await.clone();

    // A path Cairn was told not to look at yields no fact at all, and the
    // reason is `evidence_excluded` rather than `no_evidence` — "I was told not
    // to look" and "nobody attached anything" are different answers.
    if config.is_path_excluded(&source_locator) {
        return Err(WireError::new(
            codes::EVIDENCE_EXCLUDED,
            "that locator matches a privacy exclusion; no evidence was created",
        ));
    }

    // Cairn may only claim to have collected something it can actually read.
    // Anything else is an agent's attestation, and is labelled as one.
    let collector = collector.unwrap_or(match kind {
        EvidenceKind::RuntimeState => EvidenceCollector::Agent,
        _ => EvidenceCollector::Cairn,
    });

    let fingerprint = cairn_core::digest(&observed_value);
    let fact = cairn_store::evidence::record(
        &d.store,
        cairn_store::evidence::NewEvidence {
            project_id: r.project.id,
            kind,
            collector,
            subject: &subject,
            observed_value: &observed_value,
            source_locator: &source_locator,
            fingerprint: &fingerprint,
            observation_id,
            repo_branch: &git.branch,
            repo_commit: git.commit_sha.as_deref(),
            collected_by_session: session.id,
        },
        config.evidence_value_max_bytes,
        config.evidence_locator_max_bytes,
    )
    .await
    .map_err(|e| {
        let text = e.to_string();
        if text.contains(codes::ABSOLUTE_LOCATOR) {
            WireError::new(codes::ABSOLUTE_LOCATOR, text)
        } else if text.contains(codes::EVIDENCE_OUTSIDE_WORKTREE) {
            WireError::new(codes::EVIDENCE_OUTSIDE_WORKTREE, text)
        } else {
            storage_err(e)
        }
    })?;

    let mut body = json!({ "evidence": evidence_json(&fact) });
    if let Some(memory_id) = memory_id {
        cairn_store::evidence::attach_to_memory(
            &d.store,
            memory_id,
            fact.id,
            role.unwrap_or(EvidenceRole::Supports),
            session.id,
        )
        .await
        .map_err(storage_err)?;
        body["attached_to"] = json!(memory_id);

        // Whether either branch below has already read the post-attachment
        // state back, so the second one does not repeat a rebuild the first
        // already performed against the same attachment.
        let mut rebuilt = false;

        // The attestation **is** the act that establishes the claim.
        //
        // `contracts/evidence-verification.md` §Agent-attested says an agent
        // submitting an observed value and its digest may move a memory to
        // `verified` with authority `attested`. Nothing did. `cairn verify`
        // correctly refuses to re-run an agent's observation — Cairn has no way
        // to — and no other path recorded a run, so `attested` was reachable
        // from a store-level call in the test suite and from no caller at all.
        //
        // The run goes through the ordinary verifier with the submission as its
        // captured outcome, rather than being written `verified` directly: that
        // way the digest is really compared, and re-attesting a *different*
        // value against the same fact drifts instead of quietly re-verifying.
        if collector == EvidenceCollector::Agent {
            if let Some(verifier @ VerifierKind::RuntimeState) = crate::verify::verifier_for(&fact)
            {
                let captured = crate::verify::CapturedOutcome {
                    outcome: observed_value.clone(),
                    exit_code: 0,
                    commit: git.commit_sha.clone(),
                };
                let worktree = std::path::PathBuf::from(&r.repo.worktree_path);
                let outcome = crate::verify::run_verifier(
                    &worktree,
                    &config,
                    &fact,
                    verifier,
                    Some(&captured),
                );
                cairn_store::evidence::record_run(
                    &d.store,
                    cairn_store::evidence::NewRun {
                        project_id: r.project.id,
                        memory_id: Some(memory_id),
                        criterion_id: None,
                        verifier,
                        evidence_id: Some(fact.id),
                        expected_digest: fact.fingerprint.as_deref(),
                        observed_digest: outcome.observed.as_deref(),
                        result: outcome.result,
                        detail: outcome.detail.as_deref(),
                        repo_branch: &git.branch,
                        repo_commit: git.commit_sha.as_deref(),
                        trigger: VerifyTrigger::Attach,
                    },
                )
                .await
                .map_err(storage_err)?;
                let (state, authority) =
                    cairn_store::evidence::rebuild_verification_after_run(&d.store, memory_id)
                        .await
                        .map_err(storage_err)?;
                body["verification"] = json!({ "state": state, "authority": authority });
                rebuilt = true;
            }
        }

        // A contradiction is not inert. It has no run of its own to record —
        // the fact itself, already attached above, is what the derivation
        // reads — but it can move the memory to `conflicted` on the spot, and
        // a caller told only `attached_to` would have no way to learn that
        // without a second round trip. Skipped when the branch above already
        // rebuilt: that call saw the same attachment, since it happens after
        // it, so a second rebuild here would only repeat it.
        if !rebuilt && role == Some(EvidenceRole::Contradicts) {
            let (state, authority) =
                cairn_store::evidence::rebuild_verification(&d.store, memory_id)
                    .await
                    .map_err(storage_err)?;
            body["verification"] = json!({ "state": state, "authority": authority });
        }
    }
    Ok(body)
}

/// Verify on demand: the same verifiers and the same caps as the background
/// pass, reported synchronously (FR-472).
#[cfg(any())]
async fn verify_now(
    d: &Daemon,
    cwd: &str,
    memory_id: Option<Uuid>,
    all: bool,
    explain: bool,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let worktree = std::path::PathBuf::from(&r.repo.worktree_path);

    if let Some(id) = memory_id {
        let (state, authority) = verify_one(d, r.project.id, &worktree, id).await?;
        let mut body = json!({
            "memory_id": id,
            "verification": state,
            // Never bare: every surface that shows a state shows its authority
            // (FR-370).
            "authority": authority,
        });
        if explain {
            body["runs"] = json!(run_history(d, id).await?);
        } else if state != VerificationState::Verified {
            // Why it is not verified, without making the caller ask twice. A
            // locator that names no key produces an `inconclusive` run with the
            // reason on it, and reporting only `unverified` hid the one line
            // that says what to change.
            if let Some(last) = run_history(d, id).await?.into_iter().next() {
                body["last_run"] = json!({
                    "result": last["result"],
                    "detail": last["detail"],
                    "verifier": last["verifier"],
                });
            }
        }
        return Ok(body);
    }

    if !all {
        return Err(WireError::invalid("verify needs --memory or --all"));
    }

    let report = crate::verify::bounded_pass(d, r.project.id, &worktree).await;
    let mut body = json!({
        "facts_examined": report.facts_examined,
        "runs_recorded": report.runs_recorded,
        "memories_updated": report.memories_updated,
    });
    if report.yielded {
        // A cap bound. Remaining work is queued for the next tick; this is an
        // outcome, not a failure (FR-473).
        body["notes"] = json!([codes::VERIFY_PASS_YIELDED]);
    }
    Ok(body)
}

#[cfg(any())]
async fn verify_one(
    d: &Daemon,
    project_id: Uuid,
    worktree: &std::path::Path,
    memory_id: Uuid,
) -> Result<(VerificationState, Option<VerificationAuthority>), WireError> {
    let config = d.config.read().await.clone();
    let git = git_status(worktree.to_path_buf()).await?;

    let linked = cairn_store::evidence::facts_for_memory(&d.store, memory_id)
        .await
        .map_err(storage_err)?;
    if linked.is_empty() {
        // No evidence is a state, not an error: the memory stays unverified and
        // the reason is that nobody attached anything (FR-473).
        return Err(WireError::new(
            codes::NO_EVIDENCE,
            "that memory carries no evidence, so nothing can be checked",
        ));
    }

    for (role, fact) in linked {
        if role != EvidenceRole::Supports {
            continue;
        }
        let Some(verifier) = crate::verify::verifier_for(&fact) else {
            continue;
        };
        let outcome = crate::verify::run_verifier(worktree, &config, &fact, verifier, None);
        cairn_store::evidence::record_run(
            &d.store,
            cairn_store::evidence::NewRun {
                project_id,
                memory_id: Some(memory_id),
                criterion_id: None,
                verifier,
                evidence_id: Some(fact.id),
                expected_digest: fact.fingerprint.as_deref(),
                observed_digest: outcome.observed.as_deref(),
                result: outcome.result,
                detail: outcome.detail.as_deref(),
                repo_branch: &git.branch,
                repo_commit: git.commit_sha.as_deref(),
                trigger: VerifyTrigger::OnDemand,
            },
        )
        .await
        .map_err(storage_err)?;
    }

    // A run was just recorded here, so the conservative guard against
    // resurrecting a `needs_recheck` state does not apply: these records are
    // newer than the state they replace.
    cairn_store::evidence::rebuild_verification_after_run(&d.store, memory_id)
        .await
        .map_err(storage_err)
}

#[cfg(any())]
async fn run_history(d: &Daemon, memory_id: Uuid) -> Result<Vec<serde_json::Value>, WireError> {
    Ok(cairn_store::evidence::runs_for_memory(&d.store, memory_id)
        .await
        .map_err(storage_err)?
        .into_iter()
        .map(|r| {
            json!({
                "verifier": r.verifier,
                "result": r.result,
                "detail": r.detail,
                "repo_branch": r.repo_branch,
                "repo_commit": r.repo_commit,
                "checked_at": r.checked_at,
                "triggered_by": r.trigger,
            })
        })
        .collect())
}

/// Record that a session confirms an existing memory is still true (FR-321).
#[cfg(any())]
async fn memory_reinforce(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    memory_id: Uuid,
    from_memory_id: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    if server_owns_knowledge(d).await {
        // `reinforcement_count` is derived, and a client that could send it
        // could assert it (`knowledge-commands.md` §3.1). So the intent travels
        // and the server does the counting.
        //
        // The `from` endpoint is still required, and still refused here rather
        // than server-side, because this is where the caller finds out. It does
        // **not** travel: the server's `reinforce` command increments the count
        // and records no edge — recording one is `relate`, a command of its own
        // (`knowledge-commands.md` §3). Sending a field the handler does not read
        // would look like the edge had crossed when it had not.
        let from = from_memory_id.ok_or_else(|| {
            WireError::invalid(
                "reinforcement needs the memory that carries the confirming statement",
            )
        })?;
        let _ = from;
        return queue_knowledge_command(
            d,
            Some(r.project.id),
            session_id,
            cairn_store::spool::CommandKind::Reinforce,
            &json!({ "target_id": memory_id }),
        )
        .await;
    }
    let session = ensure_session_for_memory(d, &r, session_id, agent_session_key).await?;
    let target = repo::memory(&d.store, memory_id)
        .await
        .map_err(storage_err)?;

    // Without a memory of its own, the confirmation still needs a `from`
    // endpoint. The session's own most recent memory is not a substitute — it
    // may be about something else entirely — so the caller supplies one.
    let from = from_memory_id.ok_or_else(|| {
        WireError::invalid("reinforcement needs the memory that carries the confirming statement")
    })?;

    let wrote = cairn_store::knowledge::reinforce(
        &d.store,
        target.project_id,
        from,
        memory_id,
        session.id,
        RelationBasis::ExplicitAgent,
    )
    .await
    .map_err(storage_err)?;

    let counts: (i64, i64) = sqlx::query_as(
        "SELECT reinforcement_count, distinct_origin_count FROM memories WHERE id = ?1",
    )
    .bind(memory_id.to_string())
    .fetch_one(d.store.pool())
    .await
    .map_err(|e| storage_err(cairn_store::StoreError::Sqlx(e)))?;

    Ok(json!({
        "reinforced": memory_id,
        "recorded": wrote,
        // Never presented as a number of independent verifications (FR-406).
        "reinforcements": counts.0,
        "distinct_origins": counts.1,
    }))
}

/// Record an explicit reconciliation decision (FR-335).
#[allow(clippy::too_many_arguments)]
#[cfg(any())]
async fn memory_reconcile(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    from_memory_id: Uuid,
    to_memory_id: Uuid,
    relation: RelationKind,
    basis: RelationBasis,
    basis_evidence_id: Option<Uuid>,
    rationale: Option<String>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = ensure_session_for_memory(d, &r, session_id, agent_session_key).await?;

    // A conflict is detected automatically and resolved never. Leaving one
    // requires a supersession, a narrowing, or a verification result that
    // distinguishes the members (FR-334).
    if relation == RelationKind::ConflictsWith {
        return Err(WireError::new(
            codes::NOT_CONFLICTED,
            "a conflict is detected, not declared; resolve it by superseding or narrowing",
        ));
    }

    let rationale = rationale.map(|t| cairn_core::redact::redact(&t));
    let wrote = cairn_store::knowledge::reconcile_as(
        &d.store,
        r.project.id,
        session.id,
        from_memory_id,
        to_memory_id,
        relation,
        basis,
        basis_evidence_id,
        rationale.as_deref(),
    )
    .await
    .map_err(|e| {
        let text = e.to_string();
        if text.contains("relation_conflict") {
            WireError::new(codes::RELATION_CONFLICT, text)
        } else if text.contains("invalid_request") {
            WireError::invalid(text)
        } else {
            storage_err(e)
        }
    })?;

    Ok(json!({
        "from": from_memory_id,
        "to": to_memory_id,
        "relation": relation,
        "basis": basis,
        "recorded": wrote,
    }))
}

/// The identity tokens for the project currently being worked in, if any —
/// what `create_personal` and personal/team promotion screen `content`
/// against (`contracts/global-memory.md` §"D446", T074, T079).
///
/// The client-side counterpart of `cairn_server::global::identities_for`: that
/// function unions every project a *user* is a member of, because the server
/// cannot know which project a client was in; this one only ever has the one
/// project in front of it, so it derives tokens from that project alone. Same
/// rule for what counts as a token — a project's name, plus the host,
/// organisation and repository parts of its remote — deliberately duplicated
/// here rather than imported, because the server's version reads Postgres
/// rows this client never has.
#[cfg(any())]
fn current_project_identities(project: &Project) -> Vec<ProjectIdentity> {
    let mut out = Vec::new();
    let name = project.name.trim();
    if !name.is_empty() {
        out.push(ProjectIdentity(name.to_string()));
    }
    if let Some(remote) = &project.repository_remote {
        out.extend(remote_identity_tokens(remote));
    }
    out
}

/// The host, organisation and repository parts of a git remote. See
/// [`current_project_identities`].
#[cfg(any())]
fn remote_identity_tokens(remote: &str) -> Vec<ProjectIdentity> {
    const STRUCTURAL: &[&str] = &["git", "ssh", "www", "http", "https", "com", "org", "net"];
    remote
        .trim_end_matches(".git")
        .split(['/', ':', '@'])
        .filter(|part| {
            !part.is_empty()
                && part.len() >= 3
                && !STRUCTURAL.contains(&part.to_ascii_lowercase().as_str())
        })
        .map(|part| ProjectIdentity(part.to_string()))
        .collect()
}

/// `cairn_remember action: "create", domain: "personal"` (T079, FR-431).
///
/// This is the first of `validate_global_content`'s five entry points
/// (`create_personal` runs it internally, T074) — there is no separate call
/// here, only the identities to screen against and the write itself.
#[cfg(any())]
async fn personal_create(
    d: &Daemon,
    cwd: &str,
    kind: MemoryType,
    content: String,
    topic_key: Option<String>,
    value_key: Option<String>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let content = cairn_core::redact::redact(&content);
    let identities = current_project_identities(&r.project);

    // Once the server owns durable knowledge this is a request, not a write
    // (FR-712). Screening still happens here and not only server-side: a
    // command carrying content the boundary refuses should be refused before it
    // is queued, so the user learns now rather than when the drain reports it.
    cairn_core::validate::validate_global_content(
        &content,
        topic_key.as_deref(),
        value_key.as_deref(),
        &[],
        &identities,
    )
    .map_err(|e| WireError::new(codes::INVALID_REQUEST, e.to_string()))?;
    if server_owns_knowledge(d).await {
        let payload = json!({
            "knowledge_type": kind.as_str(),
            "content": content,
            "topic_key": topic_key,
            "value_key": value_key,
        });
        return queue_knowledge_command(
            d,
            None,
            None,
            cairn_store::spool::CommandKind::PersonalCreate,
            &payload,
        )
        .await;
    }

    let new = cairn_store::global::NewPersonalKnowledge::direct(
        d.owner_identity().await,
        kind,
        &content,
        topic_key.as_deref(),
        value_key.as_deref(),
        // No applicability argument on direct creation via `cairn_remember`
        // today (FR-435): an entry created with none applies to every
        // project, which is the ordinary case this tool exists for.
        Vec::new(),
    );
    let outcome = cairn_store::global::create_personal(&d.store, new, &identities)
        .await
        .map_err(|e| WireError::new(codes::INVALID_REQUEST, e.to_string()))?;

    let report = ReconciliationReport::build(
        &outcome.reconciliation,
        outcome.subject.as_deref(),
        outcome.relation_recorded,
        outcome.matched_value_key.clone(),
    );
    let mut body = json!({ "memory": outcome.record, "domain": "personal" });
    body["reconciliation"] = serde_json::to_value(&report).unwrap_or(serde_json::Value::Null);
    if !outcome.notes.is_empty() {
        body["notes"] = json!(outcome.notes);
    }
    Ok(body)
}

// ---------------------------------------------------------------------------
// Post-cutover routing (T027, FR-701, FR-712, FR-815a)
// ---------------------------------------------------------------------------

/// Turn an explicit knowledge mutation into a command once the server owns
/// durable knowledge.
///
/// Before cutover this does nothing and the local write stands. After it, a
/// local write would be exactly what FR-712 forbids — "a local write the server
/// later discovers" — so the mutation becomes a **request** instead.
///
/// It is always spooled rather than sent inline, and that is deliberate.
/// FR-781 says an agent operation must not block on the server, and FR-815a
/// says an explicit creation made offline becomes a queued write rather than a
/// local durable record. Sending inline would satisfy neither when the server
/// is slow: the caller would wait, and a failure would leave the daemon
/// choosing between blocking and inventing a local record. Spooling gives one
/// path for both cases, and the drain (T039) delivers it — promptly when the
/// server is there, later when it is not.
///
/// The caller is told the command was **accepted for delivery**, never that it
/// is durable. Nothing local becomes authoritative because a command is
/// waiting (FR-709, FR-787).
/// Whether an explicit mutation must become a request rather than a local write.
///
/// **Read on every explicit mutation, and the reason it is a function rather
/// than a flag captured once is that the answer changes under a running
/// daemon**: cutover flips it, and a handler holding a stale copy would keep
/// writing local durable rows after the server took ownership.
///
/// A store that cannot answer is treated as not authoritative. The local path is
/// the one that works without a server, and guessing the other way would queue
/// commands nothing will ever apply.
#[cfg(any())]
async fn server_owns_knowledge(d: &Daemon) -> bool {
    cairn_store::authority::mode(&d.store)
        .await
        .map(|m| m.commands_are_authoritative())
        .unwrap_or(false)
}

pub(crate) async fn queue_knowledge_command(
    d: &Daemon,
    project_id: Option<Uuid>,
    session_id: Option<Uuid>,
    kind: cairn_store::spool::CommandKind,
    payload: &serde_json::Value,
) -> Reply {
    // Account-bound, and it fails closed. A command spooled with no account
    // could not be claimed by anyone — the claim predicate matches an account
    // exactly — so queueing one would be a silent black hole rather than a
    // queued write (FR-790, FR-864a).
    let Some(account_id) = d.account_identity().await else {
        return Err(WireError::new(
            codes::NOT_LINKED,
            "sign in before recording knowledge: the server owns durable \
             knowledge now, and a command with no account could never be \
             delivered",
        ));
    };

    // Sessionless is a real case, not a degenerate one. The CLI permits memory
    // operations outside any session, and the honest representation is a
    // store-scoped command rather than a throwaway session row — which would
    // leave a second active session in the worktree and make the next agent's
    // context ambiguous (`contracts/knowledge-commands.md` §4.1).
    let scope = match session_id {
        Some(session) => cairn_store::spool::CommandScope::Session(session),
        None => cairn_store::spool::store_scope(&d.store)
            .await
            .map_err(storage_err)?,
    };

    let admission = cairn_store::spool::spool_command(
        &d.store,
        cairn_store::spool::NewCommand {
            // Bound to the server this store has established a lane with, at the
            // moment the command is written (FR-791). Never re-decided later.
            server_instance_id: cairn_store::cursor::bound_server_instance(&d.store)
                .await
                .map_err(storage_err)?,
            scope,
            project_id,
            account_id,
            kind,
            payload,
        },
        cairn_store::spool::SpoolCapacity::default(),
    )
    .await
    .map_err(storage_err)?;

    match admission {
        cairn_store::spool::CommandAdmission::Spooled(command) => Ok(json!({
            // Not "stored". The distinction is the contract's: a queued command
            // is not a local durable record, and saying so would be the claim
            // FR-709 and FR-787 exist to prevent.
            "accepted_for_delivery": true,
            "command_id": command.command_id,
            "scope": command.scope.kind(),
            "command_seq": command.command_seq,
        })),
        // Refused visibly, and nothing queued was discarded to make room: no
        // explicit command is droppable (FR-785 as applied in `spool.rs`).
        cairn_store::spool::CommandAdmission::Saturated { queued } => Err(WireError::new(
            codes::STORAGE_UNAVAILABLE,
            format!(
                "the command queue is full at {queued} undelivered \
                     commands; nothing was dropped, and this command was not \
                     accepted"
            ),
        )),
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(any())]
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
    subject: SubjectProposal,
) -> Reply {
    let r = d.resolve(cwd).await?;
    // A memory needs an origin session, and only that. Evidence is optional and
    // is never fabricated (FR-019).
    let session = ensure_session_for_memory(d, &r, session_id, agent_session_key).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;

    let (scope, key) = resolve_scope(&r, &session, &git.branch, scope, scope_key)?;
    let content = cairn_core::redact::redact(&content);

    // Once the server owns durable knowledge, this stops being a local write.
    // `local_only` is the one exception and stays local by definition: it is
    // knowledge the user asked never to leave the machine (FR-051), so routing
    // it through the server would be the opposite of what it means.
    if !local_only
        && cairn_store::authority::mode(&d.store)
            .await
            .map_err(storage_err)?
            .commands_are_authoritative()
    {
        // Intent only. Nothing derived travels — no state, no counts, no
        // verification — because the server computes those and a client that
        // could send them could assert them (`knowledge-commands.md` §3.1).
        let payload = json!({
            "type": kind.as_str(),
            "scope": scope.as_str(),
            "scope_key": key,
            "content": content,
            "topic_key": subject.topic_key,
            "value_key": subject.value_key,
            "supersedes": supersedes,
        });
        let command = match supersedes {
            Some(_) => cairn_store::spool::CommandKind::Supersede,
            None => cairn_store::spool::CommandKind::Remember,
        };
        return queue_knowledge_command(d, Some(r.project.id), Some(session.id), command, &payload)
            .await;
    }

    let new = repo::NewMemory {
        project_id: r.project.id,
        kind,
        scope,
        scope_key: &key,
        content: &content,
        origin_session_id: session.id,
        local_only,
        evidence: &evidence,
        topic_key: subject.topic_key.as_deref(),
        value_key: subject.value_key.as_deref(),
        importance: subject.importance.unwrap_or(cairn_core::Importance::Normal),
    };

    match supersedes {
        Some(original) => {
            let (old, new) = repo::supersede_memory(&d.store, original, new, r.policy)
                .await
                .map_err(storage_err)?;
            let mut body = json!({ "memory": new, "superseded": old.id });
            note_local_only_durability(d, local_only, &mut body).await;
            Ok(body)
        }
        None => {
            let out = repo::create_memory_reconciled(
                &d.store,
                new,
                r.policy,
                d.config.read().await.reconcile_members_max,
            )
            .await
            .map_err(storage_err)?;

            // What reconciliation decided, and the notes that ride an `ok: true`
            // envelope: an unrepresentable topic key, a deferred decision, or a
            // corroborating member the writer should look at (FR-312, FR-327,
            // FR-474).
            let mut body = json!({ "memory": out.memory });
            body["reconciliation"] =
                serde_json::to_value(out.report()).unwrap_or(serde_json::Value::Null);
            if !out.notes.is_empty() {
                body["notes"] = json!(out.notes);
            }
            note_local_only_durability(d, local_only, &mut body).await;
            Ok(body)
        }
    }
}

/// Say what `--local-only` costs, on the reply to the write that chose it.
///
/// **FR-706 asks for this at the point of choosing, and this is that point.**
/// Local-only is the one deliberate exclusion from FR-703's durability
/// guarantee: the record is excluded because the user asked for it to be, and a
/// choice whose consequence is stated only in a manual is a choice made without
/// it. Deleting this store deletes the record, and no pull brings it back —
/// there is nothing on the server to pull.
///
/// Attached to the reply rather than logged, so it reaches the human and the
/// agent that made the call. Silent when the flag was not set: a note on every
/// write would be noise, and noise is how a warning stops being read.
///
/// Only in the end state. While the local store is still the authority, every
/// memory is local and `--local-only` withholds nothing a colleague would
/// otherwise have — the warning would be true of the whole store, which is a
/// statement about the installation and not about this write.
#[cfg(any())]
async fn note_local_only_durability(d: &Daemon, local_only: bool, body: &mut serde_json::Value) {
    if !local_only {
        return;
    }
    let authoritative = cairn_store::authority::mode(&d.store)
        .await
        .map(|m| m.commands_are_authoritative())
        .unwrap_or(false);
    if !authoritative {
        return;
    }
    body["durability"] = json!({
        "class": cairn_store::diag::DurabilityClass::LocalOnly.as_str(),
        "survives_local_loss": false,
        "note": "local-only: this stays on this machine. It is not sent to the \
                 server, it is excluded from the durability guarantee, and \
                 deleting this store deletes it — there is nothing to restore \
                 it from.",
    });
}

/// Recording memory should not require the caller to have started a session
/// first; one is opened on demand so provenance is always real.
///
/// Only genuine absence opens one. Swallowing every error here also swallowed
/// `ambiguous_session`, which meant a second agent in the same worktree quietly
/// got a throwaway session — worsening the ambiguity for everyone else and
/// stamping the memory with an origin that never did the work. Ambiguity is the
/// caller's to resolve, exactly as it is for `cairn context`.
#[cfg(any())]
pub(crate) async fn ensure_session_for_memory(
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
    // One on-demand session per worktree, not one per call.
    //
    // A fresh `new_id()` here minted a distinct key every time, and
    // `start_session` is idempotent *per key* — so every keyless write created
    // another session. Two `cairn memory add` calls left two, and the third
    // call, along with every `cairn context` after it, failed with
    // `ambiguous_session`: the command that opened the sessions was the command
    // that broke the worktree. Under concurrency it is worse; 32 parallel
    // writes left 32 sessions and 21 of them failed outright.
    //
    // Deriving the key from the worktree makes the on-demand session stable and
    // idempotent, which is what `start_session`'s key contract already assumes.
    // It stays per-worktree because scope resolution is: two worktrees are two
    // working contexts and must not share one session.
    let key = key.unwrap_or_else(|| {
        format!(
            "cairn-cli-{}",
            &cairn_core::digest(&r.worktree())[..16.min(cairn_core::digest(&r.worktree()).len())]
        )
    });
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
            daemon_run_id: d.run_id,
            policy: r.policy,
        },
    )
    .await
    .map_err(storage_err)
}

#[cfg(any())]
fn resolve_scope(
    r: &Resolved,
    session: &Session,
    branch: &str,
    scope: Option<MemoryScope>,
    scope_key: Option<String>,
) -> Result<(MemoryScope, String), WireError> {
    let scope = scope.unwrap_or(MemoryScope::Branch);
    let key = match (scope, scope_key) {
        (_, Some(k)) => k,
        (MemoryScope::Project, None) => r.project.id.to_string(),
        (MemoryScope::Branch, None) => branch.to_string(),
        (MemoryScope::Session, None) => session.id.to_string(),
    };
    Ok((scope, key))
}

async fn server_graph(d: &Daemon, cwd: &str, memory_id: Uuid, hops: Option<i64>) -> Reply {
    let resolved = d.resolve(cwd).await?;
    let project_id = resolved
        .project
        .server_project_id
        .ok_or_else(|| WireError::new(codes::NOT_LINKED, "project is not linked to a server"))?;
    let hops = hops.unwrap_or(1).clamp(1, 2);
    crate::sync::client(d)
        .await?
        .get(&format!(
            "/api/projects/{project_id}/graph?memory_id={memory_id}&hops={hops}"
        ))
        .await
}

async fn server_replay(d: &Daemon, cwd: &str) -> Reply {
    let resolved = d.resolve(cwd).await?;
    let project_id = resolved
        .project
        .server_project_id
        .ok_or_else(|| WireError::new(codes::NOT_LINKED, "project is not linked to a server"))?;
    crate::sync::client(d)
        .await?
        .get(&format!("/api/projects/{project_id}/replay"))
        .await
}

#[cfg(any())]
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
        session_id: session.as_ref().map(|s| s.id),
    };
    let include_patterns = query.include_patterns;
    // Absent means all three (FR-472). A caller with no personal or team
    // knowledge of their own sees zero difference from one who never
    // touches either domain (FR-481, T164): searching those domains against
    // an empty store costs one query each and returns `[]`, exactly what an
    // omitted field would have shown.
    let domains = query.domains.clone().unwrap_or_else(|| {
        vec![
            KnowledgeDomain::Project,
            KnowledgeDomain::Personal,
            KnowledgeDomain::Team,
        ]
    });

    // `results`/`total` describe project results only, computed exactly as
    // they always were — before `personal[]`/`team[]` are considered at all
    // (D424, FR-469, FR-470). A caller that excluded `project` from
    // `domains` gets none, the same way excluding `personal`/`team` gets
    // those two `[]`.
    let results = if domains.contains(&KnowledgeDomain::Project) {
        search::search(&d.store, r.project.id, &query, &ctx)
            .await
            .map_err(storage_err)?
    } else {
        Vec::new()
    };
    let total = results.len();
    let mut payload = serde_json::to_value(SearchPayload { results, total }).unwrap_or(json!({}));

    // Two more **sibling** arrays, spliced in exactly as `patterns[]` is
    // below — never merged into `results` (§7, FR-469). Each is ranked
    // within its own FTS5 corpus alone (T162); there is no comparator that
    // ranks one against `results` or against each other (D425, FR-471).
    if let Some(object) = payload.as_object_mut() {
        let limit = query.limit.unwrap_or(search::GLOBAL_SEARCH_DEFAULT_LIMIT);
        let needs_traits = domains.contains(&KnowledgeDomain::Personal)
            || domains.contains(&KnowledgeDomain::Team);
        let traits = if needs_traits {
            d.project_traits(&r).await
        } else {
            Vec::new()
        };

        let personal = if domains.contains(&KnowledgeDomain::Personal) {
            search::search_personal(
                &d.store,
                d.owner_identity().await,
                query.query.as_deref(),
                &traits,
                limit,
            )
            .await
            .map_err(storage_err)?
        } else {
            Vec::new()
        };
        object.insert("personal".into(), json!(personal));

        let team = if domains.contains(&KnowledgeDomain::Team) {
            // Authoritative only, for every caller including its own
            // proposer — a proposed entry is invisible to *all* recall
            // (FR-452); `cairn team list` is the one surface that shows a
            // proposer their own pending proposals, and it does not share
            // this function.
            search::search_team(&d.store, query.query.as_deref(), &traits, limit)
                .await
                .map_err(storage_err)?
        } else {
            Vec::new()
        };
        object.insert("team".into(), json!(team));
    }

    // A **separate** array, and only when asked for. Merging a pattern into
    // `results` would hand a caller another project's knowledge among its own
    // memories, with nothing in the shape to say which was which (SC-312).
    if include_patterns {
        let signals = crate::briefing::project_signals_for(d, r.project.id, &git.branch).await;
        let config = d.config.read().await.clone();
        let matched = cairn_store::patterns::matching(
            &d.store,
            &signals,
            config.pattern_signals_min,
            config.patterns_in_context_max,
        )
        .await
        .unwrap_or_default();

        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "patterns".into(),
                json!(matched
                    .into_iter()
                    .map(|(p, overlap)| json!({
                        "id": p.id,
                        "title": p.title,
                        "trust": p.trust,
                        // Always. A pattern is offered, never asserted here.
                        "verified_in_this_project": false,
                        "applicability": p.applicability,
                        "approach": p.approach,
                        "constraints": p.constraints,
                        "signal_overlap": overlap,
                    }))
                    .collect::<Vec<_>>()),
            );
        }
    }
    Ok(payload)
}

async fn personal_create(
    d: &Daemon, cwd: &str, kind: MemoryType, content: String,
    topic_key: Option<String>, value_key: Option<String>,
) -> Reply {
    d.resolve(cwd).await?;
    queue_knowledge_command(d, None, None, cairn_store::spool::CommandKind::PersonalCreate,
        &json!({ "knowledge_type": kind.as_str(), "content": cairn_core::redact::redact(&content), "topic_key": topic_key, "value_key": value_key })).await
}

#[allow(clippy::too_many_arguments)]
async fn memory_create(
    d: &Daemon, cwd: &str, _agent_session_key: Option<String>, session_id: Option<Uuid>,
    kind: MemoryType, scope: Option<MemoryScope>, scope_key: Option<String>, content: String,
    _evidence: Vec<Uuid>, local_only: bool, supersedes: Option<Uuid>, subject: SubjectProposal,
) -> Reply {
    if local_only {
        return Err(WireError::invalid("local-only memory is unavailable; server owns durable knowledge"));
    }
    let r = d.resolve(cwd).await?;
    let scope = scope.unwrap_or(MemoryScope::Project);
    let payload = json!({
        "type": kind.as_str(), "scope": scope.as_str(),
        "scope_key": scope_key.unwrap_or_else(|| r.project.id.to_string()),
        "content": cairn_core::redact::redact(&content),
        "topic_key": subject.topic_key, "value_key": subject.value_key,
        "session_id": session_id,
    });
    queue_knowledge_command(d, Some(r.project.id), session_id,
        if supersedes.is_some() { cairn_store::spool::CommandKind::Supersede } else { cairn_store::spool::CommandKind::Remember },
        &payload).await
}

async fn memory_reinforce(
    d: &Daemon, cwd: &str, _agent_session_key: Option<String>, session_id: Option<Uuid>,
    memory_id: Uuid, from_memory_id: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    from_memory_id.ok_or_else(|| WireError::invalid("reinforcement needs the memory that carries the confirming statement"))?;
    queue_knowledge_command(d, Some(r.project.id), session_id, cairn_store::spool::CommandKind::Reinforce,
        &json!({ "target_id": memory_id, "session_id": session_id })).await
}

#[allow(clippy::too_many_arguments)]
async fn memory_reconcile(
    d: &Daemon, cwd: &str, _agent_session_key: Option<String>, session_id: Option<Uuid>,
    from_memory_id: Uuid, to_memory_id: Uuid, relation: RelationKind, basis: RelationBasis,
    basis_evidence_id: Option<Uuid>, rationale: Option<String>,
) -> Reply {
    if relation == RelationKind::ConflictsWith {
        return Err(WireError::new(codes::NOT_CONFLICTED, "a conflict is detected, not declared; resolve it by superseding or narrowing"));
    }
    let r = d.resolve(cwd).await?;
    queue_knowledge_command(d, Some(r.project.id), session_id, cairn_store::spool::CommandKind::Relate,
        &json!({ "from_memory_id": from_memory_id, "to_memory_id": to_memory_id,
            "kind": relation.as_str(), "basis": basis.as_str(), "basis_evidence_id": basis_evidence_id,
            "rationale": rationale.map(|text| cairn_core::redact::redact(&text)), "session_id": session_id })).await
}

#[allow(clippy::too_many_arguments)]
async fn evidence_add(
    d: &Daemon, cwd: &str, _agent_session_key: Option<String>, session_id: Option<Uuid>,
    _kind: EvidenceKind, _collector: Option<EvidenceCollector>, _subject: String, _observed_value: String,
    _source_locator: String, _observation_id: Option<Uuid>, memory_id: Option<Uuid>, _role: Option<EvidenceRole>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let memory_id = memory_id.ok_or_else(|| WireError::invalid("evidence needs a memory target"))?;
    queue_knowledge_command(d, Some(r.project.id), session_id, cairn_store::spool::CommandKind::VerificationAttestation,
        &json!({ "memory_ref": { "domain": "project", "knowledge_id": memory_id },
            "verdict": "inconclusive", "verifier_kind": "runtime_state",
            "attesting_agent": "mcp-client", "run_at": chrono::Utc::now().to_rfc3339() })).await
}

async fn verify_now(d: &Daemon, cwd: &str, memory_id: Option<Uuid>, all: bool, _explain: bool) -> Reply {
    let r = d.resolve(cwd).await?;
    let memory_id = memory_id.ok_or_else(|| WireError::invalid(if all { "verify all is unavailable; verify a memory" } else { "verify needs --memory or --all" }))?;
    queue_knowledge_command(d, Some(r.project.id), None, cairn_store::spool::CommandKind::VerificationRun,
        &json!({ "memory_ref": { "domain": "project", "knowledge_id": memory_id },
            "verdict": "inconclusive", "verifier_kind": "runtime_state", "run_at": chrono::Utc::now().to_rfc3339() })).await
}

async fn memory_search(
    d: &Daemon, cwd: &str, _agent_session_key: Option<String>, _session_id: Option<Uuid>, query: MemoryQuery,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let project_id = r.project.server_project_id.ok_or_else(|| WireError::new(codes::NOT_LINKED, "project is not linked to a server"))?;
    let mut params = vec![("domain".into(), "project".into())];
    for (key, value) in [("q", query.query), ("scope", query.scope.map(|v| v.as_str().to_string())),
        ("scope_key", query.scope_key), ("type", query.kind.map(|v| v.as_str().to_string())),
        ("state", query.state.map(|v| v.as_str().to_string())), ("limit", query.limit.map(|v| v.to_string()))] {
        if let Some(value) = value { params.push((key.into(), value)); }
    }
    let client = crate::sync::client(d).await?;
    let domains = query.domains.unwrap_or_else(|| vec![KnowledgeDomain::Project, KnowledgeDomain::Personal, KnowledgeDomain::Team]);
    let project = if domains.contains(&KnowledgeDomain::Project) {
        client.get_with_query(&format!("/api/projects/{project_id}/memories"), &params).await?
    } else { json!({ "memories": [], "total": 0 }) };
    let personal = if domains.contains(&KnowledgeDomain::Personal) {
        client.get_with_query("/api/personal/knowledge", &params).await?
    } else { json!([]) };
    let team = if domains.contains(&KnowledgeDomain::Team) {
        client.get_with_query("/api/team/knowledge", &params).await?
    } else { json!([]) };
    Ok(json!({
        "results": project.get("memories").cloned().unwrap_or_else(|| json!([])),
        "total": project.get("total").cloned().unwrap_or_else(|| json!(0)),
        "personal": personal,
        "team": team,
    }))
}

async fn continuity_mode(d: &Daemon, session: Option<&Session>) -> Option<String> {
    let _ = d;
    let agent = session.map(|s| s.agent.as_str())?;
    let agent = cairn_integrate::AgentId::parse(agent)?;
    let adapter = cairn_integrate::adapter_for(agent);
    // The declared profile, not a detected one: the mode is a statement about
    // what this agent's lifecycle can do, which does not depend on whether its
    // configuration happens to be installed right now.
    let profile = adapter.capabilities(&cairn_integrate::Detection::found(None, None));
    Some(profile.continuity_mode().as_str().to_string())
}

/// Restore the checkpoint this session should resume from.
///
/// The session's own newest checkpoint, else the newest on this branch — a
/// session that compacted before it had one still resumes informed.
async fn restore_checkpoint(
    d: &Daemon,
    r: &Resolved,
    session: Option<&Session>,
) -> Option<serde_json::Value> {
    let checkpoint = match session {
        Some(s) => match cairn_store::continuity::latest(&d.store, s.id).await {
            Ok(Some(c)) => Some(c),
            _ => cairn_store::continuity::latest_on_branch(&d.store, r.project.id, &s.branch)
                .await
                .ok()
                .flatten(),
        },
        None => None,
    }?;

    let worktree = std::path::PathBuf::from(r.worktree());
    let restored = crate::continuity::restore(d, &checkpoint, &worktree)
        .await
        .ok()?;
    serde_json::to_value(restored).ok()
}

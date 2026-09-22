//! Request dispatch: the daemon's whole behaviour, one function per verb.

use crate::state::{git_status, storage_err, Daemon, Resolved};
use cairn_core::domain::*;
use cairn_core::event::{EventContent, EventKind, OpenTrigger, SafeCanonicalEvent};
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
            queue_knowledge_command(
                d,
                Some(r.project.id),
                Some(s.id),
                cairn_store::spool::CommandKind::VerificationAttestation,
                &json!({ "recovery_override": "checkpoint" }),
            )
            .await
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
            queue_knowledge_command(
                d,
                Some(r.project.id),
                Some(s.id),
                cairn_store::spool::CommandKind::HandoffGenerate,
                &json!({ "trigger": trigger.as_str(), "recovery_override": "handoff" }),
            )
            .await
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
            let note = cairn_core::bound::bound_text(&cairn_core::redact::redact(&note), 2000).text;
            queue_knowledge_command(
                d,
                Some(r.project.id),
                Some(s.id),
                cairn_store::spool::CommandKind::HandoffAnnotate,
                &json!({ "note": note, "recovery_override": "handoff_annotation" }),
            )
            .await
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
            queue_knowledge_command(
                d,
                Some(r.project.id),
                session_id,
                cairn_store::spool::CommandKind::Pin,
                &json!({ "target_id": memory_id, "pinned": pinned }),
            )
            .await
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
            Some(KnowledgeDomain::Personal) => {
                personal_create(d, &cwd, kind, content, topic_key, value_key).await
            }
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
            Some(KnowledgeDomain::Personal) => {
                queue_knowledge_command(
                    d,
                    None,
                    None,
                    cairn_store::spool::CommandKind::PersonalForget,
                    &json!({ "target_id": memory_id }),
                )
                .await
            }
            None | Some(KnowledgeDomain::Project) => {
                let r = d.resolve(&cwd).await?;
                queue_knowledge_command(
                    d,
                    Some(r.project.id),
                    None,
                    cairn_store::spool::CommandKind::Forget,
                    &json!({ "target_id": memory_id }),
                )
                .await
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
        &cairn_core::paths::home()
            .join("removed_feature")
            .join("tasks-v1"),
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
    if let Ok(Some((path, _))) =
        transfer::removed_feature_tasks_exported_pending_cleanup(&d.store).await
    {
        let bundle_path = std::path::PathBuf::from(path);
        return match transfer::cleanup_removed_feature_tasks(&d.store).await {
            Ok(()) => {
                json!({ "status": "exported_cleaned", "backup": snapshot, "manifest": manifest_path, "bundle": bundle_path, "detail": "resumed cleanup" })
            }
            Err(error) => {
                json!({ "status": "warning", "backup": snapshot, "manifest": manifest_path, "bundle": bundle_path, "detail": error.to_string() })
            }
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
        },
    )
    .await
    .map_err(storage_err)?;

    spool_lifecycle_event(
        d,
        &r,
        &session,
        EventKind::SessionOpened,
        EventContent::SessionOpen {
            open_trigger: OpenTrigger::Startup,
        },
    )
    .await?;
    Ok(json!({
        "session": SessionSummary::from_session(&session, chrono::Utc::now()),
        "agent_session_key": key,
    }))
}

async fn session_end(
    d: &Daemon,
    cwd: &str,
    session_id: Option<Uuid>,
    agent_session_key: Option<String>,
    status: SessionStatus,
    reason: Option<String>,
    _wait_for_handoff: bool,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let session = resolve_session(d, &r, session_id, agent_session_key.as_deref()).await?;

    let sealed = repo::seal_session(&d.store, session.id, status, reason.as_deref())
        .await
        .map_err(storage_err)?;
    spool_lifecycle_event(
        d,
        &r,
        &sealed,
        EventKind::SessionClosed,
        EventContent::SessionClose {
            close_reason: status.as_str().to_owned(),
        },
    )
    .await?;

    Ok(json!({
        "session": SessionSummary::from_session(&sealed, chrono::Utc::now()),
        "accepted_for_delivery": true,
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
    _observation: ObservationInput,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let _ = resolve_session_for_event(d, &r, agent_session_key.as_deref()).await?;
    Ok(json!({ "recorded": false, "reason": "use capture_events" }))
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
    _reason: Option<ContextReason>,
    token_budget: Option<usize>,
    _explain: bool,
    _depth: Option<cairn_core::wire::ContextDepth>,
    trigger: Option<String>,
    open_trigger: Option<String>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let budget = token_budget.unwrap_or(d.config.read().await.context_budget_tokens);

    // Which session this briefing is for must be explicit whenever it could be
    // more than one. Picking an arbitrary active session would hand an agent
    // another agent's session context (FR-010, M1).
    let session = session_for_read(d, &r, session_id, agent_session_key.as_deref()).await?;

    let session = session.ok_or_else(|| {
        WireError::new(codes::NO_ACTIVE_SESSION, "context needs an active session")
    })?;
    let trigger = trigger
        .as_deref()
        .map(crate::deliver::Trigger::parse)
        .unwrap_or(crate::deliver::Trigger::Explicit);
    let deadline = std::time::Duration::from_millis(d.config.read().await.context_deadline_ms);
    Ok(crate::deliver::deliver(
        d,
        &r,
        session.id,
        trigger,
        open_trigger.as_deref(),
        budget,
        deadline,
    )
    .await
    .payload)
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
    crate::sync::client(d)
        .await?
        .get(&format!("/api/sessions/{}/handoff", session.id))
        .await
}

async fn most_recent_session(d: &Daemon, r: &Resolved) -> Result<Session, WireError> {
    repo::list_sessions(&d.store, r.project.id)
        .await
        .map_err(storage_err)?
        .into_iter()
        .next()
        .ok_or_else(|| WireError::new(codes::NO_ACTIVE_SESSION, "this project has no sessions yet"))
}

async fn spool_lifecycle_event(
    d: &Daemon,
    r: &Resolved,
    session: &Session,
    kind: EventKind,
    content: EventContent,
) -> Result<(), WireError> {
    let account_id = d.account_identity().await.ok_or_else(|| {
        WireError::new(
            codes::NOT_LINKED,
            "sign in before recording a lifecycle event",
        )
    })?;
    let agent = event_agent(&session.agent)
        .ok_or_else(|| WireError::invalid("unsupported lifecycle agent"))?;
    let event = SafeCanonicalEvent {
        event_id: Uuid::nil(),
        contract_version: cairn_core::event::CONTRACT_VERSION,
        kind,
        agent,
        vendor_event: None,
        session_id: session.id,
        session_seq: 0,
        occurred_at: chrono::Utc::now(),
        content: Some(content),
    };
    match cairn_store::spool::spool_event(
        &d.store,
        cairn_store::spool::SpoolCapacity::default(),
        cairn_store::spool::NewEvent {
            project_id: r.project.id,
            account_id,
            server_instance_id: cairn_store::cursor::bound_server_instance(&d.store)
                .await
                .map_err(storage_err)?,
            event,
        },
    )
    .await
    .map_err(storage_err)?
    {
        cairn_store::spool::EventAdmission::Spooled { .. } => Ok(()),
        cairn_store::spool::EventAdmission::Saturated { .. } => Err(WireError::new(
            codes::STORAGE_UNAVAILABLE,
            "lifecycle event queue is full",
        )),
    }
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
async fn personal_create(
    d: &Daemon,
    cwd: &str,
    kind: MemoryType,
    content: String,
    topic_key: Option<String>,
    value_key: Option<String>,
) -> Reply {
    d.resolve(cwd).await?;
    queue_knowledge_command(d, None, None, cairn_store::spool::CommandKind::PersonalCreate,
        &json!({ "knowledge_type": kind.as_str(), "content": cairn_core::redact::redact(&content), "topic_key": topic_key, "value_key": value_key })).await
}

#[allow(clippy::too_many_arguments)]
async fn memory_create(
    d: &Daemon,
    cwd: &str,
    _agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    kind: MemoryType,
    scope: Option<MemoryScope>,
    scope_key: Option<String>,
    content: String,
    _evidence: Vec<Uuid>,
    local_only: bool,
    supersedes: Option<Uuid>,
    subject: SubjectProposal,
) -> Reply {
    if local_only {
        return Err(WireError::invalid(
            "local-only memory is unavailable; server owns durable knowledge",
        ));
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
    queue_knowledge_command(
        d,
        Some(r.project.id),
        session_id,
        if supersedes.is_some() {
            cairn_store::spool::CommandKind::Supersede
        } else {
            cairn_store::spool::CommandKind::Remember
        },
        &payload,
    )
    .await
}

async fn memory_reinforce(
    d: &Daemon,
    cwd: &str,
    _agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    memory_id: Uuid,
    from_memory_id: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    from_memory_id.ok_or_else(|| {
        WireError::invalid("reinforcement needs the memory that carries the confirming statement")
    })?;
    queue_knowledge_command(
        d,
        Some(r.project.id),
        session_id,
        cairn_store::spool::CommandKind::Reinforce,
        &json!({ "target_id": memory_id, "session_id": session_id }),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn memory_reconcile(
    d: &Daemon,
    cwd: &str,
    _agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    from_memory_id: Uuid,
    to_memory_id: Uuid,
    relation: RelationKind,
    basis: RelationBasis,
    basis_evidence_id: Option<Uuid>,
    rationale: Option<String>,
) -> Reply {
    if relation == RelationKind::ConflictsWith {
        return Err(WireError::new(
            codes::NOT_CONFLICTED,
            "a conflict is detected, not declared; resolve it by superseding or narrowing",
        ));
    }
    let r = d.resolve(cwd).await?;
    queue_knowledge_command(d, Some(r.project.id), session_id, cairn_store::spool::CommandKind::Relate,
        &json!({ "from_memory_id": from_memory_id, "to_memory_id": to_memory_id,
            "kind": relation.as_str(), "basis": basis.as_str(), "basis_evidence_id": basis_evidence_id,
            "rationale": rationale.map(|text| cairn_core::redact::redact(&text)), "session_id": session_id })).await
}

#[allow(clippy::too_many_arguments)]
async fn evidence_add(
    d: &Daemon,
    cwd: &str,
    _agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    _kind: EvidenceKind,
    _collector: Option<EvidenceCollector>,
    _subject: String,
    _observed_value: String,
    _source_locator: String,
    _observation_id: Option<Uuid>,
    memory_id: Option<Uuid>,
    _role: Option<EvidenceRole>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let memory_id =
        memory_id.ok_or_else(|| WireError::invalid("evidence needs a memory target"))?;
    queue_knowledge_command(
        d,
        Some(r.project.id),
        session_id,
        cairn_store::spool::CommandKind::VerificationAttestation,
        &json!({ "memory_ref": { "domain": "project", "knowledge_id": memory_id },
            "verdict": "inconclusive", "verifier_kind": "runtime_state",
            "attesting_agent": "mcp-client", "run_at": chrono::Utc::now().to_rfc3339() }),
    )
    .await
}

async fn verify_now(
    d: &Daemon,
    cwd: &str,
    memory_id: Option<Uuid>,
    all: bool,
    _explain: bool,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let memory_id = memory_id.ok_or_else(|| {
        WireError::invalid(if all {
            "verify all is unavailable; verify a memory"
        } else {
            "verify needs --memory or --all"
        })
    })?;
    queue_knowledge_command(d, Some(r.project.id), None, cairn_store::spool::CommandKind::VerificationRun,
        &json!({ "memory_ref": { "domain": "project", "knowledge_id": memory_id },
            "verdict": "inconclusive", "verifier_kind": "runtime_state", "run_at": chrono::Utc::now().to_rfc3339() })).await
}

async fn memory_search(
    d: &Daemon,
    cwd: &str,
    _agent_session_key: Option<String>,
    _session_id: Option<Uuid>,
    query: MemoryQuery,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let project_id = r
        .project
        .server_project_id
        .ok_or_else(|| WireError::new(codes::NOT_LINKED, "project is not linked to a server"))?;
    let mut params = vec![("domain".into(), "project".into())];
    for (key, value) in [
        ("q", query.query),
        ("scope", query.scope.map(|v| v.as_str().to_string())),
        ("scope_key", query.scope_key),
        ("type", query.kind.map(|v| v.as_str().to_string())),
        ("state", query.state.map(|v| v.as_str().to_string())),
        ("limit", query.limit.map(|v| v.to_string())),
    ] {
        if let Some(value) = value {
            params.push((key.into(), value));
        }
    }
    let client = crate::sync::client(d).await?;
    let domains = query.domains.unwrap_or_else(|| {
        vec![
            KnowledgeDomain::Project,
            KnowledgeDomain::Personal,
            KnowledgeDomain::Team,
        ]
    });
    let project = if domains.contains(&KnowledgeDomain::Project) {
        client
            .get_with_query(&format!("/api/projects/{project_id}/memories"), &params)
            .await?
    } else {
        json!({ "memories": [], "total": 0 })
    };
    let personal = if domains.contains(&KnowledgeDomain::Personal) {
        client
            .get_with_query("/api/personal/knowledge", &params)
            .await?
    } else {
        json!([])
    };
    let team = if domains.contains(&KnowledgeDomain::Team) {
        client
            .get_with_query("/api/team/knowledge", &params)
            .await?
    } else {
        json!([])
    };
    Ok(json!({
        "results": project.get("memories").cloned().unwrap_or_else(|| json!([])),
        "total": project.get("total").cloned().unwrap_or_else(|| json!(0)),
        "personal": personal,
        "team": team,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ServerCredentials;
    use crate::testsupport as fx;

    #[tokio::test]
    async fn latest_handoff_never_reads_sqlite_when_server_is_unavailable() {
        let repo = fx::Repo::with(cairn_core::CairnConfig::default()).await;
        *repo.daemon.server.write().await = ServerCredentials {
            url: Some("http://127.0.0.1:1".into()),
            token: Some("test-token".into()),
            account_id: Some(Uuid::now_v7()),
        };
        let resolved = repo.daemon.resolve(&repo.cwd).await.expect("resolve");
        let session = fx::session(&repo.daemon, &resolved.project, "handoff").await;

        let error = handoff_latest(&repo.daemon, &repo.cwd, Some(session.id), None)
            .await
            .expect_err("server is unavailable");
        assert_eq!(error.code, codes::SERVER_UNAVAILABLE);
    }
}

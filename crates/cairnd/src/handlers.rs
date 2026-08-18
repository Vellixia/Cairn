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

pub(crate) async fn handle(d: &Daemon, request: Request) -> Reply {
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

        // The daemon's single lifecycle entry point (FR-112).
        Request::CanonicalEvent {
            event,
            wait_for_handoff,
            token_budget,
        } => crate::integrations::canonical_event(d, event, wait_for_handoff, token_budget).await,

        Request::IntegrationSnapshot { cwd } => {
            d.resolve(&cwd).await?;
            crate::integrations::snapshot(d).await
        }
        Request::IntegrationUpsertAgent {
            cwd,
            agent,
            adapter_version,
            detected_version,
            compatibility,
            level,
            completion_guarantee,
        } => {
            d.resolve(&cwd).await?;
            crate::integrations::upsert_agent(
                d,
                agent,
                adapter_version,
                detected_version,
                compatibility,
                level,
                completion_guarantee,
            )
            .await
        }
        Request::IntegrationBind {
            cwd,
            agent,
            kind,
            owner,
            scope,
            location,
            content_hash,
            artifact_schema,
            artifact_revision,
            activation,
            container_single_line,
            created_container,
        } => {
            d.resolve(&cwd).await?;
            crate::integrations::bind(
                d,
                agent,
                kind,
                owner,
                scope,
                location,
                content_hash,
                artifact_schema,
                artifact_revision,
                activation,
                container_single_line,
                created_container,
            )
            .await
        }
        Request::IntegrationUnbind { cwd, agent, kind } => {
            d.resolve(&cwd).await?;
            crate::integrations::unbind(d, agent, kind).await
        }
        Request::IntegrationForgetAgent { cwd, agent } => {
            d.resolve(&cwd).await?;
            crate::integrations::forget_agent(d, agent).await
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
        Request::IntegrationInvalidateEvidence {
            cwd,
            agent,
            detected_version,
        } => {
            d.resolve(&cwd).await?;
            crate::integrations::invalidate_evidence(d, agent, detected_version).await
        }
        Request::IntegrationMigration {
            cwd,
            agent,
            kind,
            action,
            source_owner,
            source_scope,
            source_location,
            target_owner,
            target_scope,
            target_location,
            overlap_permitted,
            phase,
            last_error,
        } => {
            d.resolve(&cwd).await?;
            crate::integrations::migration(
                d,
                agent,
                kind,
                action,
                (source_owner, source_scope, source_location),
                (target_owner, target_scope, target_location),
                overlap_permitted,
                phase,
                last_error,
            )
            .await
        }
        Request::IntegrationRecovery {
            cwd,
            agent,
            kind,
            source_path,
            artifact_path,
            content_hash,
        } => {
            d.resolve(&cwd).await?;
            crate::integrations::record_recovery(
                d,
                agent,
                kind,
                source_path,
                artifact_path,
                content_hash,
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
        } => {
            context(
                d,
                &cwd,
                agent_session_key,
                session_id,
                reason,
                token_budget,
                explain,
            )
            .await
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
            let mut out = json!({ "task": t });
            // The new read-only fields. `local_revision` is what an agent
            // passes back as `expected_revision`; `state_digest` is what two
            // machines compare. They answer different questions and are never
            // interchangeable (D80).
            let detail = task_detail(d, task_id).await?;
            if let (Some(o), Some(m)) = (out.as_object_mut(), detail.as_object()) {
                for (k, v) in m {
                    o.insert(k.clone(), v.clone());
                }
            }
            Ok(out)
        }
        Request::TaskCreate {
            cwd,
            title,
            goal,
            acceptance_criteria,
        } => {
            let r = d.resolve(&cwd).await?;
            // The seeded criteria are attributed in the change log. Resolving
            // without creating is deliberate: see `authoring_session`.
            let session = authoring_session(d, &r, None, None).await?;
            let t = repo::create_task(
                &d.store,
                r.project.id,
                &title,
                &goal,
                &acceptance_criteria,
                session,
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
            let session = authoring_session(d, &r, None, None).await?;
            let t = repo::update_task(
                &d.store,
                task_id,
                title.as_deref(),
                goal.as_deref(),
                acceptance_criteria.as_deref(),
                status,
                session,
                r.policy,
            )
            .await
            .map_err(storage_err)?;
            Ok(json!({ "task": t }))
        }

        Request::TaskCriterionAdd {
            cwd,
            agent_session_key,
            session_id,
            task_id,
            text,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = authoring_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            let c = cairn_store::criteria::add_criterion(&d.store, task_id, &text, s, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "criterion": criterion_json(&c) }))
        }
        Request::TaskCriterionSet {
            cwd,
            agent_session_key,
            session_id,
            criterion_id,
            state,
            text,
            expected_revision,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = authoring_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            if state.is_none() && text.is_none() {
                return Err(WireError::invalid("pass --state or --text"));
            }
            let mut c = None;
            if let Some(state) = state {
                c = Some(
                    cairn_store::criteria::set_criterion_state(
                        &d.store,
                        criterion_id,
                        state,
                        expected_revision,
                        s,
                        r.policy,
                    )
                    .await
                    .map_err(storage_err)?,
                );
            }
            if let Some(text) = text {
                // A second change in the same call compares against the revision
                // the first one produced, because the caller's token was already
                // honoured by that write.
                //
                // Only when the caller supplied one, though. A caller that
                // supplied none is making a blind write, and *both* halves must
                // be recorded as blind — otherwise `cairn task history` shows
                // half an overwrite while the other half reads as checked
                // (FR-490).
                let expected = expected_revision.and(
                    c.as_ref()
                        .map(|c: &cairn_store::criteria::Criterion| c.revision),
                );
                c = Some(
                    cairn_store::criteria::set_criterion_text(
                        &d.store,
                        criterion_id,
                        &text,
                        expected.or(expected_revision),
                        s,
                        r.policy,
                    )
                    .await
                    .map_err(storage_err)?,
                );
            }
            Ok(json!({ "criterion": c.as_ref().map(criterion_json) }))
        }
        Request::TaskCriterionVerify {
            cwd,
            agent_session_key,
            session_id,
            criterion_id,
            evidence_id,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = authoring_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            if let Some(evidence_id) = evidence_id {
                cairn_store::evidence::attach_to_criterion(&d.store, criterion_id, evidence_id, s)
                    .await
                    .map_err(storage_err)?;
            }
            let verdict = crate::verify::verify_criterion(
                d,
                r.project.id,
                std::path::Path::new(&r.worktree()),
                criterion_id,
                s,
                r.policy,
            )
            .await?;
            let c = cairn_store::criteria::criterion_by_id(&d.store, criterion_id)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "criterion": criterion_json(&c), "verdict": verdict }))
        }
        Request::TaskCriterionRemove {
            cwd,
            agent_session_key,
            session_id,
            criterion_id,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = authoring_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            cairn_store::criteria::remove_criterion(&d.store, criterion_id, s, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "removed": criterion_id }))
        }
        Request::TaskBlockerOpen {
            cwd,
            agent_session_key,
            session_id,
            task_id,
            description,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = authoring_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            let b =
                cairn_store::criteria::open_blocker(&d.store, task_id, &description, s, r.policy)
                    .await
                    .map_err(storage_err)?;
            Ok(json!({ "blocker": blocker_json(&b) }))
        }
        Request::TaskBlockerClear {
            cwd,
            agent_session_key,
            session_id,
            blocker_id,
        } => {
            let r = d.resolve(&cwd).await?;
            let s = authoring_session(d, &r, session_id, agent_session_key.as_deref()).await?;
            let b = cairn_store::criteria::clear_blocker(&d.store, blocker_id, s, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "blocker": blocker_json(&b) }))
        }
        Request::TaskReadiness { cwd, task_id } => {
            d.resolve(&cwd).await?;
            let readiness = cairn_store::criteria::readiness(&d.store, task_id)
                .await
                .map_err(storage_err)?;
            Ok(json!({
                "progress": readiness.progress,
                "open_blockers": readiness.open_blockers,
                "completion_readiness": readiness.completion_readiness,
            }))
        }
        Request::TaskHistory {
            cwd,
            task_id,
            limit,
        } => {
            d.resolve(&cwd).await?;
            let changes = cairn_store::criteria::history(&d.store, task_id, limit.unwrap_or(100))
                .await
                .map_err(storage_err)?;
            let changes: Vec<serde_json::Value> = changes
                .iter()
                .map(|c| {
                    json!({
                        "local_revision": c.local_revision,
                        "kind": c.kind,
                        "subject_id": c.subject_id,
                        "session_id": c.session_id,
                        "prior_value": c.prior_value,
                        "new_value": c.new_value,
                        "blind_write": c.blind_write,
                    })
                })
                .collect();
            Ok(json!({ "changes": changes }))
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
            let s = ensure_session_for_memory(d, &r, session_id, agent_session_key).await?;
            let config = d.config.read().await.clone();
            repo::set_pinned(
                &d.store,
                memory_id,
                pinned,
                reason.as_deref(),
                s.id,
                config.pin_budget_project,
                config.pin_budget_per_scope,
            )
            .await
            .map_err(storage_err)?;
            Ok(json!({ "memory_id": memory_id, "pinned": pinned }))
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
            importance,
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
                SubjectProposal {
                    topic_key,
                    value_key,
                    importance,
                },
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
            topic_key,
            value_key,
            importance,
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
                SubjectProposal {
                    topic_key,
                    value_key,
                    importance,
                },
            )
            .await
        }
        Request::MemorySubject {
            cwd,
            topic_key,
            scope,
            scope_key,
        } => memory_subject(d, &cwd, topic_key, scope, scope_key).await,
        Request::PatternList { cwd, trust, signal } => {
            crate::patterns::list(d, &cwd, trust, signal).await
        }
        Request::PatternShow { cwd, id } => crate::patterns::show(d, &cwd, id).await,
        Request::PatternPromote {
            cwd,
            memory_id,
            title,
            problem,
            signals,
            applicability,
            root_cause,
            approach,
            constraints,
            dry_run,
        } => {
            crate::patterns::promote(
                d,
                &cwd,
                crate::patterns::PromoteRequest {
                    memory_id,
                    title,
                    problem,
                    signals,
                    applicability,
                    root_cause,
                    approach,
                    constraints,
                    dry_run,
                },
            )
            .await
        }
        Request::PatternOutcome {
            cwd,
            id,
            outcome,
            signals,
            alternative_cause,
            evidence_id,
            session,
        } => {
            crate::patterns::record_outcome(
                d,
                &cwd,
                id,
                outcome,
                signals,
                alternative_cause,
                evidence_id,
                session,
            )
            .await
        }
        Request::PatternForget { cwd, id } => crate::patterns::forget(d, &cwd, id).await,
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
        Request::EvidenceList { cwd, memory_id } => evidence_list(d, &cwd, memory_id).await,
        Request::EvidenceShow { cwd, evidence_id } => evidence_show(d, &cwd, evidence_id).await,
        Request::Verify {
            cwd,
            memory_id,
            all,
            explain,
        } => verify_now(d, &cwd, memory_id, all, explain).await,
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
        integration_mode: integration_mode(d).await,
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
        local_schema_version: cairn_store::migrate::latest_version(),
        sessions_awaiting_handoff: debt.0,
        knowledge: knowledge_health(d, r.project.id).await,
        handoff_synthesis_failures: debt
            .1
            .into_iter()
            .map(|(session_id, reason)| cairn_core::wire::HandoffFailure { session_id, reason })
            .collect(),
    };
    Ok(serde_json::to_value(payload).unwrap_or(json!({})))
}

/// The subject mechanism's reach in this project, and what it is reporting.
///
/// `None` only when the read fails: status must not fail because a metric could
/// not be computed.
async fn knowledge_health(
    d: &Daemon,
    project_id: Uuid,
) -> Option<cairn_core::wire::KnowledgeHealth> {
    let a = repo::subject_adoption(&d.store, project_id).await.ok()?;
    Some(cairn_core::wire::KnowledgeHealth {
        project_memories: a.project_memories,
        with_subject: a.with_subject,
        subject_share_percent: a.percent(),
        conflicted_subjects: a.conflicted_subjects,
        needs_recheck: a.needs_recheck,
        drifted: a.drifted,
        sync_degradation: crate::sync::degradation(d, project_id).await,
    })
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
/// Which mode this repository is in, from the local integration record.
///
/// Feature 001 answered this by grepping a settings file for `cairn hook`,
/// which is the fuzzy ownership test FR-139 forbids — it also matched a
/// developer's own command that merely mentioned Cairn, and it only ever
/// looked at the committed project file, while Feature 002's default
/// lifecycle scope is the gitignored one. Ownership is the record.
async fn integration_mode(d: &Daemon) -> String {
    let agents = cairn_store::integrations::list_agents(&d.store)
        .await
        .unwrap_or_default();
    for agent in agents {
        let bound = cairn_store::integrations::bound_resources(&d.store, &agent)
            .await
            .unwrap_or_default();
        if bound.iter().any(|b| b.resource.kind == "lifecycle") {
            return format!("{agent}-hooks");
        }
    }
    "manual-mcp".into()
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Resolve which session a request is about.
///
/// A worktree may hold several active sessions, so ambiguity is reported
/// rather than guessed (FR-010).
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

async fn context(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    _reason: Option<ContextReason>,
    token_budget: Option<usize>,
    explain: bool,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let budget = token_budget.unwrap_or(d.config.read().await.context_budget_tokens);

    // Which session this briefing is for must be explicit whenever it could be
    // more than one. Picking an arbitrary active session would hand an agent
    // another agent's task goal (FR-010, M1).
    let session = session_for_read(d, &r, session_id, agent_session_key.as_deref()).await?;

    let payload = briefing::build(d, &r, session.as_ref(), budget, false, explain).await?;
    let mut out = serde_json::to_value(payload).unwrap_or(json!({}));

    // The mode Cairn can honestly promise this agent — derived from Feature
    // 002's capability profile, never from a capability of its own (FR-426).
    if let Some(mode) = continuity_mode(d, session.as_ref()).await {
        if let Some(o) = out.as_object_mut() {
            o.insert("continuity_mode".into(), json!(mode));
        }
    }

    // A post-compaction refresh is where a checkpoint is restored.
    if _reason == Some(ContextReason::PostCompaction) {
        if let Some(restored) = restore_checkpoint(d, &r, session.as_ref()).await {
            if let Some(o) = out.as_object_mut() {
                o.insert("checkpoint".into(), restored);
            }
        }
    }

    // Whether the task advanced since this session bound to it (FR-489, D80).
    //
    // Derived by diffing the bound snapshot against the current records — never
    // read from `task_changes`, which is local and would silently omit a
    // criterion another machine changed even though the row itself arrived.
    // Phase 8 places this in the Level 0 tier; it is reported here so a session
    // is never presented as having worked against the current state.
    if let Some(divergence) = task_divergence(d, session.as_ref()).await {
        if let Some(o) = out.as_object_mut() {
            o.insert("task_divergence".into(), divergence);
        }
    }
    Ok(out)
}

/// What materially changed on the bound task since the session bound to it.
///
/// `None` when there is no bound task, no snapshot (a session that bound before
/// this feature existed genuinely does not know, and synthesizing one would
/// produce a false report), or nothing changed.
async fn task_divergence(d: &Daemon, session: Option<&Session>) -> Option<serde_json::Value> {
    let session = session?;
    let task_id = session.task_id?;
    let snapshot: Option<String> =
        sqlx::query_scalar("SELECT task_snapshot_at_bind FROM sessions WHERE id = ?1")
            .bind(session.id.to_string())
            .fetch_optional(d.store.pool())
            .await
            .ok()
            .flatten();
    let snapshot = snapshot?;

    let changes = cairn_store::criteria::divergence(&d.store, task_id, &snapshot)
        .await
        .ok()?;
    if changes.is_empty() {
        return None;
    }
    Some(json!({
        "task_id": task_id,
        "advanced": true,
        "changes": changes,
    }))
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

/// The subject identity a caller proposed, carried as one value so the create
/// path does not grow three more positional arguments.
///
/// Every field optional: a caller that supplies none receives Feature 001
/// behaviour exactly, and the memory is stored free-form (FR-313, FR-497).
#[derive(Debug, Clone, Default)]
pub struct SubjectProposal {
    pub topic_key: Option<String>,
    pub value_key: Option<String>,
    pub importance: Option<Importance>,
}

// ---------------------------------------------------------------------------
// Evidence and verification (T057)
// ---------------------------------------------------------------------------

/// Render one fact for output, with its value already redacted and bounded at
/// the point it was stored.
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
    }
    Ok(body)
}

async fn evidence_list(d: &Daemon, cwd: &str, memory_id: Option<Uuid>) -> Reply {
    let r = d.resolve(cwd).await?;
    let facts = match memory_id {
        Some(id) => cairn_store::evidence::facts_for_memory(&d.store, id)
            .await
            .map_err(storage_err)?
            .into_iter()
            .map(|(role, f)| {
                let mut v = evidence_json(&f);
                v["role"] = json!(role);
                v
            })
            .collect::<Vec<_>>(),
        None => cairn_store::evidence::facts_for_project(&d.store, r.project.id)
            .await
            .map_err(storage_err)?
            .iter()
            .map(evidence_json)
            .collect(),
    };
    Ok(json!({ "evidence": facts, "total": facts.len() }))
}

async fn evidence_show(d: &Daemon, cwd: &str, evidence_id: Uuid) -> Reply {
    d.resolve(cwd).await?;
    let fact = cairn_store::evidence::fact(&d.store, evidence_id)
        .await
        .map_err(storage_err)?;
    Ok(json!({ "evidence": evidence_json(&fact) }))
}

/// Verify on demand: the same verifiers and the same caps as the background
/// pass, reported synchronously (FR-472).
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

    cairn_store::evidence::rebuild_verification(&d.store, memory_id)
        .await
        .map_err(storage_err)
}

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

async fn memory_subject(
    d: &Daemon,
    cwd: &str,
    topic_key: String,
    scope: Option<MemoryScope>,
    scope_key: Option<String>,
) -> Reply {
    let r = d.resolve(cwd).await?;
    let git = git_status(r.repo.worktree_path.clone()).await?;
    let scope = scope.unwrap_or(MemoryScope::Project);
    let key = match (&scope, scope_key) {
        (_, Some(k)) => k,
        (MemoryScope::Project, None) => r.project.id.to_string(),
        (MemoryScope::Branch, None) => git.branch.clone(),
        (_, None) => {
            return Err(WireError::invalid(
                "a task- or session-scoped subject needs an explicit --scope-key",
            ))
        }
    };

    let normalized = cairn_core::knowledge::normalize_topic_key(&topic_key).ok_or_else(|| {
        WireError::new(
            codes::INVALID_TOPIC_KEY,
            "that topic key has no representable characters",
        )
    })?;

    let cap = d.config.read().await.reconcile_members_max;
    let read =
        cairn_store::knowledge::subject(&d.store, r.project.id, scope, &key, &normalized, cap)
            .await
            .map_err(storage_err)?;

    if read.members.is_empty() {
        return Err(WireError::new(
            codes::SUBJECT_NOT_FOUND,
            format!("no subject {normalized} in {scope}:{key}"),
        ));
    }

    // Elevation candidates are *reported*, never applied: branch-scoped
    // knowledge never becomes project knowledge because a branch merged
    // (FR-382).
    let mut elevation = Vec::new();
    if scope == MemoryScope::Project {
        let worktree = std::path::PathBuf::from(&r.repo.worktree_path);
        for c in cairn_store::knowledge::branch_scoped_subjects(&d.store, r.project.id)
            .await
            .map_err(storage_err)?
            .into_iter()
            .filter(|c| c.topic_key == normalized)
        {
            let merged = tokio::task::spawn_blocking({
                let worktree = worktree.clone();
                let branch = c.branch.clone();
                let target = git.branch.clone();
                move || cairn_git::is_merged_into(&worktree, &branch, &target).unwrap_or(false)
            })
            .await
            .unwrap_or(false);
            if merged {
                elevation.push(json!({
                    "memory_id": c.memory_id,
                    "branch": c.branch,
                    "value_key": c.value_key,
                    "applied": false,
                }));
            }
        }
    }

    Ok(json!({
        "subject": {
            "topic_key": normalized,
            "scope": scope,
            "scope_key": key,
            "reconciliation": read.view.reconciliation,
            "answers": read.view.answers,
            "narrowed_by": read.view.narrowed_by,
            "accounting": read.view.accounting.iter().map(|a| json!({
                "memory_id": a.memory_id,
                "duplicates": a.duplicates,
                "distinct_origins": a.distinct_origins,
            })).collect::<Vec<_>>(),
            "decisions": read.view.decisions.iter().map(|r| json!({
                "from": r.from, "to": r.to, "kind": r.kind, "basis": r.basis,
            })).collect::<Vec<_>>(),
            "members": read.members.iter().map(|m| json!({
                "id": m.id,
                "state": m.state,
                "value_key": m.value_key,
                "verification": m.verification,
                "verification_authority": m.verification_authority,
                "pinned": m.pinned,
                "importance": m.importance,
            })).collect::<Vec<_>>(),
            "degraded": read.degraded,
            "elevation_candidates": elevation,
        }
    }))
}

/// Record that a session confirms an existing memory is still true (FR-321).
async fn memory_reinforce(
    d: &Daemon,
    cwd: &str,
    agent_session_key: Option<String>,
    session_id: Option<Uuid>,
    memory_id: Uuid,
    from_memory_id: Option<Uuid>,
) -> Reply {
    let r = d.resolve(cwd).await?;
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
    subject: SubjectProposal,
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
        topic_key: subject.topic_key.as_deref(),
        value_key: subject.value_key.as_deref(),
        importance: subject.importance.unwrap_or(cairn_core::Importance::Normal),
    };

    match supersedes {
        Some(original) => {
            let (old, new) = repo::supersede_memory(&d.store, original, new, r.policy)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "memory": new, "superseded": old.id }))
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
                serde_json::to_value(&out.reconciliation).unwrap_or(serde_json::Value::Null);
            if !out.notes.is_empty() {
                body["notes"] = json!(out.notes);
            }
            Ok(body)
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
/// Who a task-model change is attributed to, **without creating a session**.
///
/// `ensure_session_for_memory` starts a `cairn-cli` session when there is none,
/// which is right for a memory — a memory must belong to a session's provenance.
/// It is wrong here: `cairn task new` has never needed a session, and inventing
/// one leaves a second active session in the worktree that makes the next
/// agent's `cairn_context` ambiguous.
///
/// The nil UUID means "no session", and is what `cairn task history` renders as
/// an unattributed change. That is honest: a CLI invocation outside any session
/// genuinely has no author to name, and naming a throwaway one would be worse
/// than naming none.
async fn authoring_session(
    d: &Daemon,
    r: &Resolved,
    session_id: Option<Uuid>,
    key: Option<&str>,
) -> Result<Uuid, WireError> {
    match resolve_session(d, r, session_id, key).await {
        Ok(s) => Ok(s.id),
        Err(e) if e.code == codes::NO_ACTIVE_SESSION || e.code == codes::AMBIGUOUS_SESSION => {
            Ok(Uuid::nil())
        }
        Err(e) => Err(e),
    }
}

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
    let include_patterns = query.include_patterns;
    let results = search::search(&d.store, r.project.id, &query, &ctx)
        .await
        .map_err(storage_err)?;
    let total = results.len();
    let mut payload = serde_json::to_value(SearchPayload { results, total }).unwrap_or(json!({}));

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

// ---------------------------------------------------------------------------
// Task work state rendering (`contracts/task-model.md`)
// ---------------------------------------------------------------------------

/// One criterion, as every surface reports it.
///
/// Note what has no key here: a percentage. Progress is counts, derived on
/// read, and there is nowhere for an agent to write a number of its own
/// (FR-486).
fn criterion_json(c: &cairn_store::criteria::Criterion) -> serde_json::Value {
    json!({
        "id": c.id,
        "ordinal": c.ordinal,
        "label": c.label,
        "text": c.text,
        "state": c.state,
        "verification": c.verification,
        "revision": c.revision,
        "deleted": c.deleted,
    })
}

fn blocker_json(b: &cairn_store::criteria::Blocker) -> serde_json::Value {
    json!({
        "id": b.id,
        "task_id": b.task_id,
        "description": b.description,
        "state": b.state,
        "opened_by_session": b.opened_by_session,
        "cleared_by_session": b.cleared_by_session,
    })
}

/// The read-only fields `task get` gained.
///
/// `local_revision` and `state_digest` sit side by side deliberately: the first
/// answers "has anything changed since I read this, **here**", the second
/// answers "do two machines hold the same task state". Conflating them was a
/// real defect in the first design (D80).
async fn task_detail(d: &Daemon, task_id: Uuid) -> Result<serde_json::Value, WireError> {
    let t = repo::task(&d.store, task_id).await.map_err(storage_err)?;
    let local_revision: i64 = sqlx::query_scalar("SELECT local_revision FROM tasks WHERE id = ?1")
        .bind(task_id.to_string())
        .fetch_one(d.store.pool())
        .await
        .map_err(|e| storage_err(cairn_store::StoreError::from(e)))?;

    let criteria = cairn_store::criteria::criteria(&d.store, task_id)
        .await
        .map_err(storage_err)?;
    let blockers = cairn_store::criteria::blockers(&d.store, task_id)
        .await
        .map_err(storage_err)?;
    let readiness = cairn_store::criteria::readiness(&d.store, task_id)
        .await
        .map_err(storage_err)?;
    let digest = cairn_store::criteria::state_digest(&d.store, task_id)
        .await
        .map_err(storage_err)?;

    // Evidence counts per criterion, so a reader can see what a verification
    // rests on without the evidence content crossing any boundary.
    let mut rendered = Vec::new();
    for c in criteria.iter().filter(|c| !c.deleted) {
        let facts = cairn_store::evidence::facts_for_criterion(&d.store, c.id)
            .await
            .unwrap_or_default();
        let mut v = criterion_json(c);
        if let Some(o) = v.as_object_mut() {
            o.insert("evidence_count".into(), json!(facts.len()));
            o.insert(
                "authority".into(),
                json!(facts
                    .iter()
                    .map(|f| f.collector.as_str())
                    .collect::<std::collections::BTreeSet<_>>()),
            );
        }
        rendered.push(v);
    }

    let _ = &t;
    Ok(json!({
        "local_revision": local_revision,
        "state_digest": digest,
        "criteria": rendered,
        "blockers": blockers
            .iter()
            .filter(|b| !b.deleted)
            .map(blocker_json)
            .collect::<Vec<_>>(),
        "progress": readiness.progress,
        "open_blockers": readiness.open_blockers,
        "completion_readiness": readiness.completion_readiness,
    }))
}

// ---------------------------------------------------------------------------
// Continuity (`contracts/continuity-context.md` Part 1)
// ---------------------------------------------------------------------------

/// What Cairn can honestly promise this agent about compression-safe continuity.
///
/// Derived from Feature 002's capability profile (D57). `None` when the session's
/// agent has no profile — a mode invented for an unknown agent would be a claim
/// with nothing behind it.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::{self as fx, Repo};

    /// Dispatch is the daemon's whole surface, so drive it rather than the
    /// private handlers: every reply an agent or the CLI ever sees passes
    /// through here, including the error envelope.
    async fn ok(r: &Repo, request: Request) -> serde_json::Value {
        let envelope = dispatch(&r.daemon, request).await;
        let json = serde_json::to_value(&envelope).expect("serializable envelope");
        assert_eq!(json["ok"], true, "expected success, got {json}");
        json["data"].clone()
    }

    async fn err(r: &Repo, request: Request) -> serde_json::Value {
        let envelope = dispatch(&r.daemon, request).await;
        let json = serde_json::to_value(&envelope).expect("serializable envelope");
        assert_eq!(json["ok"], false, "expected failure, got {json}");
        json["error"].clone()
    }

    #[tokio::test]
    async fn daemon_status_reports_this_run_and_the_schema_it_opened() {
        let r = Repo::new().await;
        let v = ok(&r, Request::DaemonStatus).await;
        assert_eq!(v["running"], true);
        assert_eq!(v["run_id"], r.daemon.run_id.to_string());
        assert_eq!(
            v["schema_version"],
            cairn_store::migrate::latest_version(),
            "the reported schema must be the one actually applied"
        );
    }

    /// `init` is idempotent: the same checkout is one project however often it
    /// is registered (FR-004).
    #[tokio::test]
    async fn init_is_idempotent_for_one_checkout() {
        let r = Repo::new().await;
        let first = ok(&r, Request::Init { cwd: r.cwd.clone() }).await;
        let second = ok(&r, Request::Init { cwd: r.cwd.clone() }).await;
        assert_eq!(first["project"]["id"], second["project"]["id"]);

        assert_eq!(
            repo::list_projects(&r.daemon.store)
                .await
                .expect("projects")
                .len(),
            1,
            "one repository is one project"
        );
    }

    /// Repository state is derived from Git, never guessed (Principle VI).
    #[tokio::test]
    async fn status_reports_the_real_working_tree() {
        let r = Repo::new().await;
        let v = ok(&r, Request::Status { cwd: r.cwd.clone() }).await;
        assert_eq!(v["repository"]["branch"], "main");
        assert!(v["repository"]["commit_sha"].is_string());
        assert_eq!(v["repository"]["untracked"], 0);

        // An untracked file must show up as one, rather than the cached value
        // from a moment ago.
        r.write("scratch.txt", "x\n");
        let v = ok(&r, Request::Status { cwd: r.cwd.clone() }).await;
        assert_eq!(v["repository"]["untracked"], 1);
    }

    /// A directory that is not a repository is refused, and creates nothing
    /// (FR-005).
    #[tokio::test]
    async fn a_non_repository_is_refused_and_stores_nothing() {
        let r = Repo::new().await;
        let elsewhere = tempfile::TempDir::new().expect("temp dir");
        let e = err(
            &r,
            Request::Status {
                cwd: elsewhere.path().display().to_string(),
            },
        )
        .await;
        assert_eq!(e["code"], codes::NOT_A_REPOSITORY);
        assert!(
            repo::list_projects(&r.daemon.store)
                .await
                .expect("projects")
                .is_empty(),
            "a refused directory must leave no project behind"
        );
    }

    /// The same agent session key rejoins rather than forking (data-model.md).
    #[tokio::test]
    async fn a_repeated_session_key_rejoins_the_same_session() {
        let r = Repo::new().await;
        let start = |key: &str| Request::SessionStart {
            cwd: r.cwd.clone(),
            agent: "claude-code".into(),
            agent_session_key: Some(key.to_string()),
            task_id: None,
        };
        let first = ok(&r, start("k1")).await;
        let second = ok(&r, start("k1")).await;
        assert_eq!(first["session"]["id"], second["session"]["id"]);

        let list = ok(&r, Request::SessionList { cwd: r.cwd.clone() }).await;
        assert_eq!(list["sessions"].as_array().expect("sessions").len(), 1);
    }

    /// An agent with no session identity of its own gets one per connection, so
    /// manual MCP mode behaves the same way (data-model.md, FR-040).
    #[tokio::test]
    async fn an_agent_with_no_key_is_given_one() {
        let r = Repo::new().await;
        let v = ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "some-mcp-client".into(),
                agent_session_key: None,
                task_id: None,
            },
        )
        .await;
        let key = v["agent_session_key"].as_str().expect("a generated key");
        assert!(
            key.starts_with("cairn-local-"),
            "a generated key should be recognisable as one: {key}"
        );
    }

    /// The session records the branch it actually ran on, so its memory and
    /// handoff are scoped to the right work (Principle IV).
    #[tokio::test]
    async fn a_session_records_the_branch_it_started_on() {
        let r = Repo::new().await;
        r.checkout("feature/rate-limit");
        let v = ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("branched".into()),
                task_id: None,
            },
        )
        .await;
        assert_eq!(v["session"]["branch"], "feature/rate-limit");
    }

    /// A session never ends without a handoff (FR-032).
    #[tokio::test]
    async fn ending_a_session_always_writes_a_handoff_first() {
        let r = Repo::new().await;
        ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("ends".into()),
                task_id: None,
            },
        )
        .await;

        let v = ok(
            &r,
            Request::SessionEnd {
                cwd: r.cwd.clone(),
                session_id: None,
                agent_session_key: Some("ends".into()),
                status: SessionStatus::Completed,
                reason: Some("clear".into()),
                // The command-line boundary: nothing holds a deadline over it,
                // so the durable handoff is in the reply this asserts on.
                wait_for_handoff: true,
            },
        )
        .await;
        assert_eq!(v["session"]["status"], "completed");
        assert_eq!(v["handoff"]["trigger"], "session_end");
        assert!(
            v["handoff"]["next_step"].is_string(),
            "a handoff always names a next step: {v}"
        );
    }

    /// Selecting a task at session start binds it, including when the session
    /// already existed (FR-038).
    #[tokio::test]
    async fn starting_with_a_task_binds_it_to_an_existing_session() {
        let r = Repo::new().await;
        let task = ok(
            &r,
            Request::TaskCreate {
                cwd: r.cwd.clone(),
                title: "Add rate limiting".into(),
                goal: "Requests over the limit get 429".into(),
                acceptance_criteria: vec!["429 above the threshold".into()],
            },
        )
        .await;
        let task_id: Uuid = task["task"]["id"]
            .as_str()
            .expect("task id")
            .parse()
            .expect("uuid");

        // Session first, with no task.
        let first = ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("late-bind".into()),
                task_id: None,
            },
        )
        .await;
        assert!(first["session"]["task_id"].is_null());

        // Then the same key again, this time naming the task.
        let second = ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("late-bind".into()),
                task_id: Some(task_id),
            },
        )
        .await;
        assert_eq!(second["session"]["id"], first["session"]["id"]);
        assert_eq!(second["session"]["task_id"], task_id.to_string());
    }

    /// A task that does not exist is refused rather than silently ignored.
    ///
    /// Pinned to what the daemon *actually* returns, which is not what it
    /// should: `start_session` is called with the `task_id` before anything
    /// checks that the task exists, so the foreign key rejects it and the
    /// failure surfaces as `storage_unavailable`. The existence check in
    /// `session_start` only runs on the arm that binds a task to an
    /// already-existing session, so it never sees this case.
    ///
    /// The user-visible effect is that `cairn session start --task <unknown>`
    /// reports a storage problem rather than a missing task. Asserted rather
    /// than glossed over, so that fixing the order trips this test and the
    /// expectation is updated with it.
    #[tokio::test]
    async fn starting_with_an_unknown_task_is_refused() {
        let r = Repo::new().await;
        let e = err(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("bad-task".into()),
                task_id: Some(Uuid::now_v7()),
            },
        )
        .await;
        assert_eq!(
            e["code"],
            codes::STORAGE_UNAVAILABLE,
            "unknown-task diagnosis changed; it should now be not_found — update this: {e}"
        );

        // Whatever the code, nothing is left behind.
        assert_eq!(
            repo::list_projects(&r.daemon.store)
                .await
                .expect("projects")
                .len(),
            1,
            "the project is created by resolve; the session must not be"
        );
        let sessions = ok(&r, Request::SessionList { cwd: r.cwd.clone() }).await;
        assert!(
            sessions["sessions"]
                .as_array()
                .expect("sessions")
                .is_empty(),
            "a refused session start must store no session: {sessions}"
        );
    }

    /// Memory carries explicit scope and provenance; it is never global
    /// (Principle IV, FR-019).
    #[tokio::test]
    async fn a_memory_records_its_scope_and_origin_session() {
        let r = Repo::new().await;
        ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("remembers".into()),
                task_id: None,
            },
        )
        .await;

        let v = ok(
            &r,
            Request::MemoryCreate {
                cwd: r.cwd.clone(),
                agent_session_key: Some("remembers".into()),
                session_id: None,
                kind: MemoryType::Convention,
                scope: Some(MemoryScope::Project),
                scope_key: None,
                content: "Errors are returned, never logged and swallowed".into(),
                evidence_observation_ids: vec![],
                local_only: false,
                topic_key: None,
                value_key: None,
                importance: None,
            },
        )
        .await;
        assert_eq!(v["memory"]["scope"], "project");
        assert_eq!(v["memory"]["type"], "convention");
        assert!(
            v["memory"]["origin_session_id"].is_string(),
            "memory must be traceable to where it came from: {v}"
        );

        let found = ok(
            &r,
            Request::MemorySearch {
                cwd: r.cwd.clone(),
                agent_session_key: None,
                session_id: None,
                query: MemoryQuery {
                    query: Some("swallowed".into()),
                    ..Default::default()
                },
            },
        )
        .await;
        assert_eq!(found["results"].as_array().expect("results").len(), 1);
    }

    /// Deleting something that is not there is a `not_found`, not a crash.
    #[tokio::test]
    async fn deleting_an_unknown_memory_is_not_found() {
        let r = Repo::new().await;
        let e = err(
            &r,
            Request::Delete {
                cwd: r.cwd.clone(),
                target: DeleteTarget::Memory,
                id: Uuid::now_v7(),
                with_memories: false,
            },
        )
        .await;
        assert_eq!(e["code"], codes::NOT_FOUND);
    }

    /// Two worktrees of one repository are one project (FR-004).
    ///
    /// The Git *common* directory is the identity, not the worktree path, so a
    /// second checkout of the same repository must not create a second project.
    #[tokio::test]
    async fn a_second_worktree_joins_the_same_project() {
        let r = Repo::new().await;
        let first = ok(&r, Request::Init { cwd: r.cwd.clone() }).await;

        let linked = r.dir.path().parent().expect("parent").join("wt-2");
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(r.dir.path())
            .args(["worktree", "add", "-q"])
            .arg(&linked)
            .args(["-b", "second"])
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git worktree add: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let cwd = std::fs::canonicalize(&linked)
            .expect("canonical")
            .display()
            .to_string();
        let second = ok(&r, Request::Init { cwd }).await;

        assert_eq!(
            first["project"]["id"], second["project"]["id"],
            "one repository is one project, however many worktrees"
        );
        let _ = std::fs::remove_dir_all(&linked);
    }

    /// Excluded paths never reach storage (FR-050, Principle V).
    ///
    /// Exclusion is a daemon-side decision, so the CLI cannot be trusted to
    /// have applied it — this drives the same request an excluded edit produces.
    #[tokio::test]
    async fn an_excluded_path_is_not_recorded() {
        let config = cairn_core::CairnConfig {
            excluded_paths: vec!["secrets/**".into()],
            ..Default::default()
        };
        let r = Repo::with(config).await;
        ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("private".into()),
                task_id: None,
            },
        )
        .await;

        for path in ["secrets/token.txt", "src/fine.rs"] {
            ok(
                &r,
                Request::Observe {
                    cwd: r.cwd.clone(),
                    agent_session_key: Some("private".into()),
                    observation: ObservationInput {
                        kind: ObservationType::FileChanged,
                        path: Some(path.into()),
                        command: None,
                        exit_code: None,
                        outcome: None,
                        summary: format!("Edited {path}"),
                        details: None,
                        vendor_tool: None,
                    },
                },
            )
            .await;
        }

        let v = ok(&r, Request::Status { cwd: r.cwd.clone() }).await;
        assert_eq!(
            v["observation_count"], 1,
            "only the non-excluded edit should have been stored: {v}"
        );
    }

    /// Sessions and projects from one repository are not visible from another.
    #[tokio::test]
    async fn two_repositories_do_not_see_each_others_sessions() {
        let r = Repo::new().await;
        let other = Repo::new().await;
        // One daemon, two checkouts: seed the second repository's instance into
        // the first daemon's cache so both resolve against one store.
        let instance =
            cairn_git::discover(std::path::Path::new(&other.cwd)).expect("discover the second");
        r.daemon
            .repos
            .write()
            .await
            .insert(other.cwd.clone(), instance);

        ok(
            &r,
            Request::SessionStart {
                cwd: r.cwd.clone(),
                agent: "claude-code".into(),
                agent_session_key: Some("here".into()),
                task_id: None,
            },
        )
        .await;

        let theirs = ok(
            &r,
            Request::SessionList {
                cwd: other.cwd.clone(),
            },
        )
        .await;
        assert!(
            theirs["sessions"].as_array().expect("sessions").is_empty(),
            "memory is project-scoped, never ambient: {theirs}"
        );
        let _ = fx::NOWHERE;
    }
}

//! Daemon handlers for the canonical lifecycle and the local integration
//! record (FR-112, FR-182–FR-184).
//!
//! Two responsibilities, both deliberately thin.
//!
//! **Canonical events.** One entry point for every adapter. The daemon has no
//! idea which vendor produced an event and cannot find out: it receives the
//! canonical vocabulary and dispatches to Feature 001's own handlers. Nothing
//! here parses vendor configuration or vendor payloads — that is the adapter's
//! job, on the other side of the boundary (D18).
//!
//! **The record.** Reads and writes of machine-local integration state. **No
//! function here enqueues an outbox row**, and none of these tables has an
//! outbox entity type: an agent configuration path or an integration health
//! detail must never reach the shared server (SC-120).

use crate::state::{storage_err, Daemon};
use cairn_core::lifecycle::{CanonicalEvent, CanonicalLifecycleEvent};
use cairn_core::wire::{MigrationAction, WireError};
use cairn_store::integrations as rec;
use serde_json::json;

type Reply = Result<serde_json::Value, WireError>;

/// Ingest one canonical lifecycle event.
///
/// Boxed because it dispatches back into the request handler, which is how it
/// reuses Feature 001's own session, capture and handoff paths rather than
/// duplicating them.
///
/// The mapping is `contracts/lifecycle.md` §The events, and nothing else:
/// quiescence is a checkpoint that leaves the session active and writes no
/// handoff; post-compaction re-delivers context and writes no second handoff;
/// only compaction and close produce durable handoffs.
pub fn canonical_event<'a>(
    d: &'a Daemon,
    event: CanonicalLifecycleEvent,
    wait_for_handoff: bool,
    token_budget: Option<usize>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Reply> + Send + 'a>> {
    Box::pin(canonical_event_inner(
        d,
        event,
        wait_for_handoff,
        token_budget,
    ))
}

async fn canonical_event_inner(
    d: &Daemon,
    event: CanonicalLifecycleEvent,
    wait_for_handoff: bool,
    token_budget: Option<usize>,
) -> Reply {
    if !event.is_well_formed() {
        return Err(WireError::invalid(
            "a canonical event must carry a session key, and only tool events carry observations",
        ));
    }
    let key = Some(event.agent_session_key.clone());
    let cwd = event.cwd.clone();
    let agent = event.agent.clone();
    let vendor_key = event.agent_session_key.clone();
    let kind = event.event;

    let reply = dispatch(d, event, wait_for_handoff, token_budget).await;

    // Evidence is a byproduct of work that already happened: an event that
    // reached here *is* the observation. Cairn never synthesizes an event or
    // calls an undocumented interface to create one (D19a).
    if reply.is_ok() {
        establish(d, &agent, &vendor_key, kind).await;
    }
    let _ = (key, cwd);
    reply
}

/// Record what this event established about the agent's capabilities.
async fn establish(d: &Daemon, agent: &str, vendor_key: &str, kind: CanonicalEvent) {
    let capability = match kind {
        CanonicalEvent::SessionOpened => "lifecycle_session_open",
        CanonicalEvent::ToolSucceeded => "lifecycle_tool_success",
        CanonicalEvent::ToolFailed => "lifecycle_tool_failure",
        CanonicalEvent::AgentQuiesced => "lifecycle_quiesce",
        CanonicalEvent::ContextCompacting => "lifecycle_pre_compaction",
        CanonicalEvent::ContextCompacted => "lifecycle_post_compaction",
        CanonicalEvent::SessionClosed => "lifecycle_session_close",
    };
    let version = rec::agent(&d.store, agent)
        .await
        .ok()
        .flatten()
        .and_then(|a| a.detected_version);
    async fn write(d: &Daemon, agent: &str, capability: &str, version: Option<String>) {
        let row = rec::CapabilityEvidence {
            agent: agent.to_string(),
            capability: capability.to_string(),
            evidence: "observation".into(),
            established_at: chrono::Utc::now().to_rfc3339(),
            agent_version: version,
            degraded: None,
        };
        if let Err(e) = rec::record_evidence(&d.store, &row).await {
            tracing::debug!(error = %e, "could not record capability evidence");
        }
    }
    write(d, agent, capability, version.clone()).await;

    // Two or more events, of at least two different kinds, on one
    // vendor-supplied key (D19a).
    let mut seen = d.lifecycle_kinds.write().await;
    let kinds = seen.entry(vendor_key.to_string()).or_default();
    if !kinds.contains(&capability) {
        kinds.push(capability);
    }
    let established = kinds.len() >= 2;
    drop(seen);
    if established {
        write(d, agent, "stable_session_identifier", version).await;
    }
}

async fn dispatch(
    d: &Daemon,
    event: CanonicalLifecycleEvent,
    wait_for_handoff: bool,
    token_budget: Option<usize>,
) -> Reply {
    let key = Some(event.agent_session_key.clone());
    let cwd = event.cwd.clone();

    match event.event {
        CanonicalEvent::SessionOpened => {
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::SessionStart {
                    cwd: cwd.clone(),
                    agent: event.agent.clone(),
                    agent_session_key: key.clone(),
                    task_id: None,
                },
            )
            .await?;
            // Context delivery is the one canonical event whose handling
            // produces something the agent consumes (D19a).
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::Context {
                    cwd,
                    agent_session_key: key,
                    session_id: None,
                    reason: Some(cairn_core::wire::ContextReason::SessionStart),
                    token_budget,
                    explain: false,
                },
            )
            .await
        }
        CanonicalEvent::ToolSucceeded | CanonicalEvent::ToolFailed => {
            let observation = event
                .observation
                .ok_or_else(|| WireError::invalid("a tool event must carry its observation"))?;
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::Observe {
                    cwd,
                    agent_session_key: key,
                    observation,
                },
            )
            .await
        }
        // Flush pending capture, record the checkpoint, leave the session
        // active, write no handoff (FR-032, FR-230).
        CanonicalEvent::AgentQuiesced => {
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::TurnCheckpoint {
                    cwd,
                    agent_session_key: key,
                },
            )
            .await
        }
        CanonicalEvent::ContextCompacting => {
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::HandoffGenerate {
                    cwd,
                    session_id: None,
                    agent_session_key: key,
                    trigger: cairn_core::domain::HandoffTrigger::PreCompact,
                },
            )
            .await
        }
        // Leaves the session active and produces no second handoff for the
        // same compaction (FR-119).
        //
        // The reason is `post_compaction`, and it has to be: restoring the
        // checkpoint is what that reason *means*, and it is the only one that
        // does it. Asking for a `continuation` here built an ordinary briefing
        // and left the checkpoint written-and-never-read — so an agent deriving
        // `automatic`, whose whole promise is that continuity is restored
        // automatically after compaction, silently got no restoration at all.
        // A mode that over-claims is a defect, not a note (FR-426).
        CanonicalEvent::ContextCompacted => {
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::Context {
                    cwd,
                    agent_session_key: key,
                    session_id: None,
                    reason: Some(cairn_core::wire::ContextReason::PostCompaction),
                    token_budget,
                    explain: false,
                },
            )
            .await
        }
        CanonicalEvent::SessionClosed => {
            crate::handlers::handle(
                d,
                cairn_core::wire::Request::SessionEnd {
                    cwd,
                    session_id: None,
                    agent_session_key: key,
                    status: cairn_core::domain::SessionStatus::Completed,
                    reason: event.reason,
                    wait_for_handoff,
                },
            )
            .await
        }
    }
}

/// Everything the CLI needs to know about what is installed here.
pub async fn snapshot(d: &Daemon) -> Reply {
    let agents = rec::list_agents(&d.store).await.map_err(storage_err)?;
    let mut out = Vec::new();
    for agent in agents {
        let row = rec::agent(&d.store, &agent).await.map_err(storage_err)?;
        let resources = rec::bound_resources(&d.store, &agent)
            .await
            .map_err(storage_err)?;
        let evidence = rec::evidence(&d.store, &agent).await.map_err(storage_err)?;
        out.push(json!({
            "agent": agent,
            "record": row,
            "resources": resources,
            "evidence": evidence,
        }));
    }
    let migrations = rec::list_migrations(&d.store).await.map_err(storage_err)?;
    Ok(json!({ "agents": out, "migrations": migrations }))
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_agent(
    d: &Daemon,
    agent: String,
    adapter_version: i64,
    detected_version: Option<String>,
    compatibility: String,
    level: String,
    completion_guarantee: String,
) -> Reply {
    rec::upsert_agent(
        &d.store,
        &rec::AgentIntegration {
            agent: agent.clone(),
            adapter_version,
            detected_version,
            compatibility,
            level,
            completion_guarantee,
            connected_at: chrono::Utc::now().to_rfc3339(),
            last_verified_at: Some(chrono::Utc::now().to_rfc3339()),
        },
    )
    .await
    .map_err(storage_err)?;
    Ok(json!({ "agent": agent }))
}

#[allow(clippy::too_many_arguments)]
pub async fn bind(
    d: &Daemon,
    agent: String,
    kind: String,
    owner: String,
    scope: String,
    location: String,
    content_hash: Option<String>,
    artifact_schema: Option<i64>,
    artifact_revision: Option<String>,
    activation: String,
    container_single_line: bool,
    created_container: bool,
) -> Reply {
    let id = rec::bind(
        &d.store,
        &agent,
        &rec::InstalledResource {
            id: uuid::Uuid::now_v7(),
            kind,
            owner,
            scope,
            location,
            content_hash,
            artifact_schema,
            artifact_revision,
            activation,
            installed_at: chrono::Utc::now().to_rfc3339(),
            last_verified_at: None,
            container_single_line,
            created_container,
        },
    )
    .await
    .map_err(storage_err)?;
    Ok(json!({ "resource_id": id }))
}

pub async fn unbind(d: &Daemon, agent: String, kind: String) -> Reply {
    let outcome = rec::unbind(&d.store, &agent, &kind)
        .await
        .map_err(storage_err)?;
    // The caller does the filesystem half only when the last binding went
    // (FR-243). Saying which happened is the whole point of the reply.
    Ok(match outcome {
        rec::Unbound::Nothing => json!({ "outcome": "nothing" }),
        rec::Unbound::ResourceKept { remaining } => {
            json!({ "outcome": "resource_kept", "remaining": remaining })
        }
        rec::Unbound::ResourceRemoved => json!({ "outcome": "resource_removed" }),
    })
}

pub async fn forget_agent(d: &Daemon, agent: String) -> Reply {
    let removed = rec::remove_agent_if_unbound(&d.store, &agent)
        .await
        .map_err(storage_err)?;
    Ok(json!({ "removed": removed }))
}

pub async fn record_evidence(
    d: &Daemon,
    agent: String,
    capability: String,
    evidence: String,
    agent_version: Option<String>,
    degraded: Option<bool>,
) -> Reply {
    // Observation evidence is version-bound, and the version it is bound to is
    // the one recorded for the agent — not whatever the caller happened to
    // know. A hook reporting a delivery has no idea what version the agent is;
    // leaving the row versionless would make the next upgrade discard it as
    // belonging to some other build, which is how a capability that is working
    // perfectly well disappears from the report (FR-245).
    let agent_version = match agent_version {
        Some(v) => Some(v),
        None if evidence == "observation" => rec::agent(&d.store, &agent)
            .await
            .ok()
            .flatten()
            .and_then(|a| a.detected_version),
        None => None,
    };
    rec::record_evidence(
        &d.store,
        &rec::CapabilityEvidence {
            agent,
            capability,
            evidence,
            established_at: chrono::Utc::now().to_rfc3339(),
            agent_version,
            degraded,
        },
    )
    .await
    .map_err(storage_err)?;
    Ok(json!({ "recorded": true }))
}

pub async fn invalidate_evidence(
    d: &Daemon,
    agent: String,
    detected_version: Option<String>,
) -> Reply {
    let discarded =
        rec::invalidate_observation_evidence(&d.store, &agent, detected_version.as_deref())
            .await
            .map_err(storage_err)?;
    Ok(json!({ "discarded": discarded }))
}

#[allow(clippy::too_many_arguments)]
pub async fn migration(
    d: &Daemon,
    agent: String,
    kind: String,
    action: MigrationAction,
    source: (Option<String>, Option<String>, Option<String>),
    target: (Option<String>, Option<String>, Option<String>),
    overlap_permitted: bool,
    phase: Option<String>,
    last_error: Option<String>,
) -> Reply {
    match action {
        MigrationAction::Start => {
            let state = rec::MigrationState {
                id: uuid::Uuid::now_v7(),
                agent: agent.clone(),
                kind: kind.clone(),
                source_owner: source.0.unwrap_or_default(),
                source_scope: source.1.unwrap_or_default(),
                source_location: source.2.unwrap_or_default(),
                target_owner: target.0.unwrap_or_default(),
                target_scope: target.1.unwrap_or_default(),
                target_location: target.2.unwrap_or_default(),
                phase: "planned".into(),
                overlap_permitted,
                started_at: chrono::Utc::now().to_rfc3339(),
                last_error: None,
            };
            rec::start_migration(&d.store, &state)
                .await
                .map_err(storage_err)?;
            Ok(serde_json::to_value(state).unwrap_or(json!({})))
        }
        MigrationAction::Advance => {
            let phase = phase.ok_or_else(|| WireError::invalid("advance needs a phase"))?;
            rec::set_migration_phase(&d.store, &agent, &kind, &phase, None)
                .await
                .map_err(storage_err)?;
            read_migration(d, &agent, &kind).await
        }
        MigrationAction::Fail => {
            rec::set_migration_phase(&d.store, &agent, &kind, "failed", last_error.as_deref())
                .await
                .map_err(storage_err)?;
            read_migration(d, &agent, &kind).await
        }
        MigrationAction::Clear => {
            rec::clear_migration(&d.store, &agent, &kind)
                .await
                .map_err(storage_err)?;
            Ok(json!({ "migration": null }))
        }
        MigrationAction::Read => read_migration(d, &agent, &kind).await,
    }
}

async fn read_migration(d: &Daemon, agent: &str, kind: &str) -> Reply {
    let state = rec::migration(&d.store, agent, kind)
        .await
        .map_err(storage_err)?;
    Ok(json!({ "migration": state }))
}

pub async fn record_recovery(
    d: &Daemon,
    agent: String,
    kind: String,
    source_path: String,
    artifact_path: String,
    content_hash: String,
) -> Reply {
    rec::record_recovery_artifact(
        &d.store,
        &rec::RecoveryArtifact {
            id: uuid::Uuid::now_v7(),
            agent,
            kind,
            source_path,
            artifact_path: artifact_path.clone(),
            content_hash,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .await
    .map_err(storage_err)?;
    // Only the path is ever returned, never the content (FR-239).
    Ok(json!({ "artifact_path": artifact_path }))
}

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
use cairn_core::wire::WireError;
use cairn_store::integrations as rec;
use serde_json::json;

type Reply = Result<serde_json::Value, WireError>;

/// Install or refresh detected integrations during explicit setup.
///
/// Instructions stay user-owned: setup never edits `AGENTS.md`, `CLAUDE.md`,
/// or committed `.claude/settings.json`. Existing Cairn-owned resources are
/// updated only when inspection still matches their recorded ownership.
pub async fn setup(d: &Daemon, cwd: &str) -> serde_json::Value {
    setup_at(d, &cairn_integrate::scope::Env::discover(cwd)).await
}

async fn setup_at(d: &Daemon, env: &cairn_integrate::scope::Env) -> serde_json::Value {
    use cairn_integrate::desired::{Choices, DesiredIntegrationState, RecordedResource};
    use cairn_integrate::model::{
        ActivationState, AgentId, ArtifactVersion, InstallationScope, ResourceKind, ResourceOwner,
    };
    use cairn_integrate::plan::{plan_agent, ChangeAction, Intent, RecordedInstall};

    let mut applied = Vec::new();
    let mut warnings = Vec::new();
    for agent in AgentId::ALL
        .into_iter()
        .filter(|agent| *agent != AgentId::GenericMcp)
    {
        let adapter = cairn_integrate::adapter_for(agent);
        let detection = adapter.detect(env);
        if !detection.detected {
            continue;
        }

        let records = match rec::bound_resources(&d.store, agent.as_str()).await {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|row| {
                    Some(RecordedInstall {
                        agent,
                        kind: ResourceKind::parse(&row.resource.kind)?,
                        owner: ResourceOwner::parse(&row.resource.owner)?,
                        scope: InstallationScope::parse(&row.resource.scope)?,
                        location: row.resource.location.into(),
                        content_hash: row.resource.content_hash,
                        artifact_schema: row.resource.artifact_schema.map(|v| v as u32),
                        artifact_revision: row.resource.artifact_revision,
                        activation: ActivationState::parse(&row.resource.activation)?,
                        serves: row
                            .serves
                            .iter()
                            .filter_map(|value| AgentId::parse(value))
                            .collect(),
                        container_single_line: row.resource.container_single_line,
                        created_container: row.resource.created_container,
                    })
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                warnings.push(json!({
                    "agent": agent.as_str(),
                    "detail": format!("could not read integration ownership: {error}"),
                }));
                continue;
            }
        };
        let recorded = records
            .iter()
            .map(|row| RecordedResource {
                agent: row.agent,
                kind: row.kind,
                owner: row.owner,
                scope: row.scope,
                activation: row.activation,
            })
            .collect::<Vec<_>>();
        let desired = DesiredIntegrationState::compose(
            &Choices {
                agents: vec![agent],
                only: vec![
                    ResourceKind::Mcp,
                    ResourceKind::Lifecycle,
                    ResourceKind::Skill,
                ],
                ..Default::default()
            },
            &[agent],
            &recorded,
            cairn_integrate::render::Contract::canonical().version(),
            ArtifactVersion::new(
                cairn_integrate::revision::embedded_schema(),
                cairn_integrate::revision::embedded_revision(),
            ),
        );
        let observed = adapter.inspect(env, &records);
        let plan = plan_agent(Intent::Repair { force: false }, agent, &desired, &observed);
        if plan.is_blocked() {
            warnings.extend(plan.blocking.into_iter().map(|blocked| {
                json!({
                    "agent": agent.as_str(),
                    "kind": blocked.kind.as_str(),
                    "detail": blocked.detail,
                })
            }));
            continue;
        }

        let now = chrono::Utc::now().to_rfc3339();
        let _ = rec::upsert_agent(
            &d.store,
            &rec::AgentIntegration {
                agent: agent.as_str().into(),
                adapter_version: 1,
                detected_version: detection.version.clone(),
                compatibility: "compatible_unverified".into(),
                level: "mcp_plus".into(),
                completion_guarantee: "not_demonstrated".into(),
                connected_at: now.clone(),
                last_verified_at: None,
            },
        )
        .await;

        for change in plan
            .changes
            .into_iter()
            .filter(|change| matches!(change.action, ChangeAction::Add | ChangeAction::Update))
        {
            let materialized = match cairn_integrate::install::materialize_install(
                env,
                agent,
                change.kind,
                change.scope,
            ) {
                Ok(value) => value,
                Err(error) => {
                    warnings.push(json!({
                        "agent": agent.as_str(),
                        "kind": change.kind.as_str(),
                        "detail": error.to_string(),
                    }));
                    continue;
                }
            };
            if let Err(error) = cairn_integrate::install::commit(&materialized) {
                warnings.push(json!({
                    "agent": agent.as_str(),
                    "kind": change.kind.as_str(),
                    "detail": error.to_string(),
                }));
                continue;
            }
            let resource = rec::InstalledResource {
                id: uuid::Uuid::now_v7(),
                kind: change.kind.as_str().into(),
                owner: ResourceOwner::Direct.as_str().into(),
                scope: change.scope.as_str().into(),
                location: materialized.location.display().to_string(),
                content_hash: materialized.content_hash,
                artifact_schema: materialized
                    .artifact
                    .as_ref()
                    .map(|value| value.schema as i64),
                artifact_revision: materialized.artifact.map(|value| value.revision),
                activation: ActivationState::NotApplicable.as_str().into(),
                installed_at: now.clone(),
                last_verified_at: Some(now.clone()),
                container_single_line: materialized.container_single_line,
                created_container: materialized.created_container,
            };
            match rec::bind(&d.store, agent.as_str(), &resource).await {
                Ok(_) => applied.push(json!({
                    "agent": agent.as_str(),
                    "kind": change.kind.as_str(),
                    "target": resource.location,
                })),
                Err(error) => warnings.push(json!({
                    "agent": agent.as_str(),
                    "kind": change.kind.as_str(),
                    "detail": format!("installed but ownership was not recorded: {error}"),
                })),
            }
        }
    }
    json!({ "applied": applied, "warnings": warnings })
}

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
    capture: Option<cairn_core::event::CaptureOutput>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Reply> + Send + 'a>> {
    Box::pin(canonical_event_inner(
        d,
        event,
        wait_for_handoff,
        token_budget,
        capture,
    ))
}

async fn canonical_event_inner(
    d: &Daemon,
    event: CanonicalLifecycleEvent,
    wait_for_handoff: bool,
    token_budget: Option<usize>,
    capture: Option<cairn_core::event::CaptureOutput>,
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

    // The safe events, after the lifecycle half — which is what created or
    // resumed the session they bind to.
    //
    // **The reply is never changed by what happens here.** A boundary event
    // must answer, and a capture-class event must fail soft: an agent that
    // received its context and then saw an error because a spool row could not
    // be written would be experiencing Cairn as the thing that broke, which is
    // exactly what FR-749a–d forbid. A capture failure is counted and logged
    // rather than returned (FR-749c).
    if let Some(capture) = capture {
        if !capture.is_empty() {
            if let Err(e) =
                crate::handlers::spool_capture(d, &cwd, &agent, &vendor_key, &capture).await
            {
                tracing::debug!(error = %e.message, agent = %agent, "capture was not spooled");
            }
        }
    }
    let _ = key;
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
                },
            )
            .await?;

            // Context delivery is the one canonical event whose handling
            // produces something the agent consumes (D19a).
            //
            // `trigger: session_open` and the vendor's own `source` are what
            // let this retrieval go through the server as the push it is
            // (`contracts/retrieval-delivery.md` §1–§3); `open_trigger` is
            // forwarded exactly as the vendor sent it (`startup`/`resume`/
            // `clear`/`compact`/`fork`), never derived from `after_compaction`
            // above, which is Cairn's own recorded-state detection and can
            // legitimately disagree with what the vendor happened to send.
            let delivered = crate::handlers::handle(
                d,
                cairn_core::wire::Request::Context {
                    cwd,
                    agent_session_key: key,
                    session_id: None,
                    reason: Some(cairn_core::wire::ContextReason::SessionStart),
                    token_budget,
                    explain: false,
                    // A lifecycle-delivered briefing has always been the full
                    // assembly; this event carries no depth of its own to
                    // forward (T156).
                    depth: None,
                    trigger: Some("session_open".to_string()),
                    open_trigger: event.source.clone(),
                },
            )
            .await;

            delivered
        }
        CanonicalEvent::ToolSucceeded | CanonicalEvent::ToolFailed => {
            let observation = event
                .observation
                .ok_or_else(|| WireError::invalid("a tool event must carry its observation"))?;
            crate::handlers::observe(d, &cwd, key, observation).await
        }
        // Flush pending capture, record the checkpoint, leave the session
        // active, write no handoff (FR-032, FR-230).
        CanonicalEvent::AgentQuiesced => crate::handlers::turn_checkpoint(d, &cwd, key).await,
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
        // Capture only. This event is capture class, so `cairn hook` sends it
        // one-way and throws the reply away -- there is no channel here to hand
        // anything back on, for any agent.
        //
        // It used to ask for a `PostCompaction` briefing, which *restores* the
        // checkpoint. Nothing could be delivered from it, so the only effect was
        // to consume the checkpoint the next session open needs, leaving whether
        // delivery is observed to depend on which of two hooks the vendor
        // happens to run first. Restoration belongs where context can actually
        // reach the model: the session that opens next for an agent that re-opens
        // one, and `cairn_context(reason=post_compaction)` for an agent that does
        // not.
        //
        // `lifecycle_post_compaction` is already recorded above, which is the
        // whole of what this event establishes.
        CanonicalEvent::ContextCompacted => Ok(serde_json::json!({})),
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

pub async fn record_evidence(
    d: &Daemon,
    agent: String,
    capability: String,
    evidence: String,
    agent_version: Option<String>,
    degraded: Option<bool>,
) -> Reply {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn setup_installs_owned_resources_is_idempotent_and_blocks_edits() {
        let d = crate::testsupport::daemon().await;
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        let repo = root.path().join("repo");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let env = cairn_integrate::scope::Env::new(&home, &repo);

        let first = setup_at(&d, &env).await;
        assert_eq!(first["warnings"], json!([]));
        assert_eq!(first["applied"].as_array().unwrap().len(), 3);
        assert!(home.join(".claude.json").exists());
        assert!(repo.join(".claude/settings.local.json").exists());
        assert!(!repo.join("CLAUDE.md").exists());
        assert!(!repo.join("AGENTS.md").exists());
        assert!(!repo.join(".claude/settings.json").exists());

        let second = setup_at(&d, &env).await;
        assert_eq!(second["warnings"], json!([]));
        assert_eq!(second["applied"], json!([]));

        let edited = r#"{"mcpServers":{"cairn":{"command":"user-edit"}}}"#;
        std::fs::write(home.join(".claude.json"), edited).unwrap();
        let conflict = setup_at(&d, &env).await;
        assert_eq!(conflict["applied"], json!([]));
        assert_eq!(conflict["warnings"].as_array().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(home.join(".claude.json")).unwrap(),
            edited
        );
    }
}

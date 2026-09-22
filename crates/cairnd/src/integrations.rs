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

            // A session that opens with an unrestored compaction checkpoint is
            // the *same* session coming back from a compaction, not a new one.
            // That is the first boundary the model reads afterwards, and it is
            // the only place Cairn can hand the checkpoint back without being
            // asked -- `context_compacted` is capture class and returns before
            // anything is emitted.
            //
            // Detected from Cairn's own recorded state rather than a vendor
            // string, so it holds for any agent that re-opens a session after
            // compacting. `source` is consulted only as corroboration where the
            // vendor supplies it: Claude Code sends `compact`, and the others
            // send nothing at all.
            let after_compaction = post_compaction_reopen(d, &cwd, key.as_deref(), &event).await;
            let reason = if after_compaction {
                cairn_core::wire::ContextReason::PostCompaction
            } else {
                cairn_core::wire::ContextReason::SessionStart
            };

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
                    reason: Some(reason),
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

            // `context_after_compaction` is delivery, not capture, so it is
            // established only where a checkpoint was actually restored *into*
            // a briefing the agent consumes. A compaction Cairn merely heard
            // about establishes `lifecycle_post_compaction` and nothing more --
            // which is the whole point of them being two capabilities.
            if after_compaction {
                if let Ok(payload) = &delivered {
                    if payload.get("checkpoint").is_some() {
                        write_evidence(d, &event.agent, "context_after_compaction").await;
                    }
                }
            }
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
        CanonicalEvent::AgentQuiesced => {
            crate::handlers::turn_checkpoint(d, &cwd, key).await
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

/// Whether this session-open is the same session returning from a compaction.
///
/// True when Cairn holds a `context_compacting` checkpoint for the session that
/// has never been restored. That is a fact about Cairn's own records, so it does
/// not depend on a vendor naming the boundary; where a vendor does name it --
/// Claude Code's `SessionStart` source is `compact` -- it agrees, and is used
/// only as corroboration.
///
/// Deliberately conservative: a checkpoint already restored is not restored
/// twice, and no checkpoint at all means an ordinary session start.
async fn post_compaction_reopen(
    d: &Daemon,
    cwd: &str,
    key: Option<&str>,
    event: &CanonicalLifecycleEvent,
) -> bool {
    // A vendor that names the boundary is believed immediately.
    let named = event
        .source
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("compact"));

    let Ok(r) = d.resolve(cwd).await else {
        return named;
    };
    let Some(key) = key else { return named };
    let session = match cairn_store::repo::session_by_key(&d.store, r.project.id, key).await {
        Ok(Some(s)) => s,
        _ => return named,
    };
    match cairn_store::continuity::latest(&d.store, session.id).await {
        // An **unrestored** compaction checkpoint is the signal, and the
        // `restore_count` is what makes a second session open harmless: once the
        // first has restored it, a duplicate finds it consumed and delivers
        // nothing again. A vendor naming the boundary is deliberately *not* ORed
        // in here -- an agent that re-emits `SessionStart` twice would then
        // restore twice, and `restore_count` would stop meaning "delivered once".
        Ok(Some(c)) => {
            c.trigger == cairn_core::domain::CheckpointTrigger::ContextCompacting
                && c.restore_count == 0
        }
        // Only where Cairn has no checkpoint to reason about at all does the
        // vendor's own naming decide. Nothing can be restored in that case, so
        // it costs a briefing reason and no more.
        _ => named,
    }
}

/// One capability-evidence row, recorded as an observation.
async fn write_evidence(d: &Daemon, agent: &str, capability: &str) {
    let version = rec::agent(&d.store, agent)
        .await
        .ok()
        .flatten()
        .and_then(|a| a.detected_version);
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

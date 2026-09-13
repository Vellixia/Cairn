//! The agent integration layer.
//!
//! One boundary sits between Cairn's domain and every coding agent it talks
//! to. Above it live vendor payloads, vendor configuration formats and vendor
//! version quirks; below it live sessions, observations, memory and handoffs,
//! which know that agents exist only as provenance.
//!
//! This crate owns:
//!
//! - the [`adapter`] trait and the four agent adapters in [`agents`];
//! - the CC Switch [`managers`] manager, which is never an agent;
//! - lifecycle normalization into `cairn_core::lifecycle`;
//! - the [`desired`] integration state — the single statement of intent;
//! - [`capability`] profiles, evidence and level derivation;
//! - source-preserving configuration inspection and mutation ([`edit`],
//!   [`apply`]);
//! - ownership [`markers`] and canonical hashing;
//! - [`scope`] resolution;
//! - the usage-contract [`render`]ings and the Skill [`revision`] algorithm.
//!
//! It deliberately has no SQLite dependency and no runtime requirement: the
//! fixture corpus and every recorded vendor payload must be testable without a
//! daemon, a socket or a temporary Git repository. Persistence stays in
//! `cairn-store` behind daemon requests, preserving Feature 001's
//! single-writer rule (D18).

pub mod adapter;
pub mod agents;
pub mod apply;
pub mod capability;
pub mod desired;
pub mod edit;
pub mod install;
pub mod managers;
pub mod markers;
pub mod model;
pub mod plan;
pub mod render;
pub mod revision;
pub mod scope;

pub use adapter::{AgentAdapter, Detection, IntegrationManager, Observed, RawPayload};
pub use capability::CapabilityProfile;
pub use model::{
    AgentId, ArtifactVersion, HealthCondition, InstallationScope, IntegrationLevel, ManagerId,
    ResourceKind, ResourceOwner,
};

/// The MCP entry Cairn installs, in every agent's format.
///
/// Deterministic and secret-free by construction: there is no field here that
/// could carry a credential (FR-131, FR-162, SC-135).
pub fn mcp_entry() -> serde_json::Value {
    serde_json::json!({ "command": "cairn", "args": ["mcp"] })
}

/// Cairn's MCP entry in **OpenCode's** schema.
///
/// OpenCode does not accept the `command` + `args` shape every other client
/// uses. It requires a tagged union -- `type: "local" | "remote"` -- with the
/// whole invocation in one `command` array and an explicit `enabled`. Writing
/// the generic shape did not merely fail to register Cairn: OpenCode rejects
/// the entire configuration file over one bad server entry, so
/// `cairn connect opencode` left OpenCode unable to start at all with
/// `Missing key mcp.cairn.enabled`. An integration must never be able to break
/// the tool it integrates with.
pub fn mcp_entry_opencode() -> serde_json::Value {
    serde_json::json!({
        "type": "local",
        "command": ["cairn", "mcp"],
        "enabled": true,
    })
}

/// Cairn's canonical MCP entry for `agent`.
pub fn mcp_entry_for(agent: model::AgentId) -> serde_json::Value {
    if agent == model::AgentId::Opencode {
        mcp_entry_opencode()
    } else {
        mcp_entry()
    }
}

/// The reserved MCP server name. Ownership is this name plus a recorded
/// canonical hash — never a search for the word "cairn" (FR-139).
pub const MCP_SERVER_NAME: &str = "cairn";

/// Return every agent adapter.
pub fn adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(agents::claude_code::ClaudeCode),
        Box::new(agents::codex::Codex),
        Box::new(agents::opencode::OpenCode),
        Box::new(agents::generic_mcp::GenericMcp),
    ]
}

/// Look up one adapter.
pub fn adapter_for(agent: AgentId) -> Box<dyn AgentAdapter> {
    match agent {
        AgentId::ClaudeCode => Box::new(agents::claude_code::ClaudeCode),
        AgentId::Codex => Box::new(agents::codex::Codex),
        AgentId::Opencode => Box::new(agents::opencode::OpenCode),
        AgentId::GenericMcp => Box::new(agents::generic_mcp::GenericMcp),
    }
}

/// Normalize one vendor event into the canonical vocabulary.
///
/// This is the function the hook entry point calls, and the only part of this
/// crate on the capture path. It is pure: no I/O, no editors, no embedded
/// assets — the cost of linking the rest is binary size, not work.
pub fn normalize(
    agent: AgentId,
    event: &str,
    payload: &RawPayload,
) -> Option<cairn_core::lifecycle::CanonicalLifecycleEvent> {
    adapter_for(agent).normalize(event, payload)
}

/// Which deadline class a vendor event belongs to, from its name alone.
///
/// The hook has to answer this *before* it reads stdin: the capture fast path
/// runs without an async runtime, and a boundary event needs a reply, so the
/// two cannot both consume the payload. Deriving it from the adapter rather
/// than a second table is what keeps the answer single-sourced — a synthetic
/// payload carrying only a routing key is enough, because the class is
/// determined by the event, never by its contents.
///
/// `None` means the adapter declines that event entirely (FR-115).
/// Feature 005 capture for one vendor event, through that agent's adapter.
///
/// Mirrors [`normalize`] and is called beside it, not instead of it: one drives
/// the lifecycle the daemon already depends on, the other produces the safe
/// events the server consolidates.
pub fn capture(
    agent: AgentId,
    event: &str,
    payload: &RawPayload,
    env: &agents::CaptureEnv<'_>,
) -> agents::CaptureOutput {
    adapter_for(agent).capture(event, payload, env)
}

/// Whether this agent's event carries transient prompt or assistant text.
pub fn carries_semantic_material(agent: AgentId, event: &str) -> bool {
    adapter_for(agent).carries_semantic_material(event)
}

pub fn event_class(agent: AgentId, event: &str) -> Option<cairn_core::lifecycle::CanonicalEvent> {
    let probe = serde_json::json!({
        "session_id": "class-probe",
        "sessionID": "class-probe",
        "thread_id": "class-probe",
    });
    normalize(agent, event, &RawPayload::new(probe, ".")).map(|e| e.event)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OpenCode rejects the whole configuration file over one malformed server
    /// entry, so the generic `command` + `args` shape did not merely fail to
    /// register Cairn -- it left OpenCode unable to start with `Missing key
    /// mcp.cairn.enabled`. An integration must not be able to break the tool it
    /// integrates with.
    #[test]
    fn the_opencode_mcp_entry_matches_opencodes_own_schema() {
        let e = mcp_entry_opencode();
        assert_eq!(e["type"], "local");
        assert_eq!(e["enabled"], true);
        // The whole invocation lives in one array; there is no `args`.
        assert_eq!(e["command"], serde_json::json!(["cairn", "mcp"]));
        assert!(e.get("args").is_none());
        let text = e.to_string();
        for word in ["token", "key", "secret", "password"] {
            assert!(!text.to_lowercase().contains(word));
        }
    }

    /// The shapes are per-agent, and picking by agent is the whole point.
    #[test]
    fn only_opencode_gets_the_opencode_shape() {
        use model::AgentId;
        assert_eq!(mcp_entry_for(AgentId::Opencode), mcp_entry_opencode());
        for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::GenericMcp] {
            assert_eq!(mcp_entry_for(agent), mcp_entry(), "{agent}");
        }
    }

    #[test]
    fn the_mcp_entry_is_deterministic_and_carries_no_secret() {
        assert_eq!(mcp_entry(), mcp_entry());
        let text = mcp_entry().to_string();
        assert_eq!(text, r#"{"args":["mcp"],"command":"cairn"}"#);
        for word in ["token", "key", "secret", "password"] {
            assert!(!text.to_lowercase().contains(word));
        }
    }

    #[test]
    fn the_deadline_class_is_answerable_without_the_payload() {
        // The hook depends on this: it must know whether to take the
        // no-runtime capture path before it reads stdin.
        use cairn_core::lifecycle::CanonicalEvent;
        assert_eq!(
            event_class(AgentId::ClaudeCode, "SessionStart"),
            Some(CanonicalEvent::SessionOpened)
        );
        assert_eq!(
            event_class(AgentId::ClaudeCode, "Stop"),
            Some(CanonicalEvent::AgentQuiesced)
        );
        assert_eq!(event_class(AgentId::ClaudeCode, "PreToolUse"), None);
        // Codex's one tool registration is capture class either way, so the
        // class does not depend on the payload it will later carry.
        assert!(!event_class(AgentId::Codex, "PostToolUse")
            .unwrap()
            .is_boundary_class());
        assert_eq!(
            event_class(AgentId::Opencode, "session.idle"),
            Some(CanonicalEvent::AgentQuiesced)
        );
        assert!(event_class(AgentId::Opencode, "session.created")
            .unwrap()
            .is_boundary_class());
    }

    #[test]
    fn every_agent_has_an_adapter() {
        assert_eq!(adapters().len(), AgentId::ALL.len());
        for a in AgentId::ALL {
            assert_eq!(adapter_for(a).id(), a);
        }
    }
}

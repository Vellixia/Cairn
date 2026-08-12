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

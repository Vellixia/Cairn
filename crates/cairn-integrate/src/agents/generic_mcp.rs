//! The generic MCP adapter (FR-102, SC-107).
//!
//! For an agent Cairn has no native adapter for. It emits no lifecycle events,
//! Cairn writes none of its configuration, and its level is `MCP_ONLY` with
//! automatic lifecycle and automatic capture named as unavailable.
//!
//! This is the honest floor of the product: the tools work, and Cairn says
//! plainly what the developer is not getting.

use super::*;
use crate::adapter::{AgentAdapter, Detection, RawPayload};
use crate::model::AgentId;
use crate::plan::RecordedInstall;
use crate::scope::Env;
use cairn_core::lifecycle::CanonicalLifecycleEvent;

pub struct GenericMcp;

impl AgentAdapter for GenericMcp {
    fn id(&self) -> AgentId {
        AgentId::GenericMcp
    }

    /// Always available: the generic path is a capability of Cairn, not a
    /// program to find on disk.
    fn detect(&self, _env: &Env) -> Detection {
        Detection::found(None, None)
    }

    /// Cairn writes nothing for a generic client — the developer pastes the
    /// exported block (FR-131) — so there is nothing to inspect.
    fn inspect(&self, _env: &Env, _record: &[RecordedInstall]) -> Vec<Observed> {
        Vec::new()
    }

    fn registered_events(&self) -> &'static [&'static str] {
        &[]
    }

    /// Emits no lifecycle events, from any event name, with any payload.
    fn normalize(&self, _event: &str, _payload: &RawPayload) -> Option<CanonicalLifecycleEvent> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Availability, Capability, CapabilityProfile};
    use serde_json::json;

    #[test]
    fn no_event_is_ever_produced() {
        let a = GenericMcp;
        for ev in [
            "SessionStart",
            "PostToolUse",
            "Stop",
            "SessionEnd",
            "session.idle",
            "anything",
        ] {
            assert!(a
                .normalize(ev, &RawPayload::new(json!({"session_id": "s"}), "/repo"))
                .is_none());
        }
        assert!(a.registered_events().is_empty());
    }

    #[test]
    fn every_lifecycle_capability_is_absent() {
        // SC-107: the report names automatic lifecycle and automatic capture
        // as unavailable.
        let p = CapabilityProfile::base(AgentId::GenericMcp);
        for c in Capability::ALL.iter().filter(|c| c.is_lifecycle()) {
            assert_eq!(
                p.get(*c).availability,
                Availability::Absent,
                "{c} is claimed for a generic client"
            );
        }
        assert_eq!(
            p.get(Capability::McpUserScope).availability,
            Availability::Guaranteed
        );
    }

    #[test]
    fn cairn_writes_nothing_for_a_generic_client() {
        let a = GenericMcp;
        let env = Env::new("/home/dev", "/repo");
        assert!(a.inspect(&env, &[]).is_empty());
    }
}

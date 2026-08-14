//! The canonical lifecycle vocabulary (FR-112, FR-113, FR-114).
//!
//! Seven events, and Cairn's session, capture and handoff behavior depends on
//! nothing else. No vendor event name, payload shape, or ordering assumption
//! reaches the daemon: an adapter translates into this type, and the daemon
//! sees only this type.
//!
//! The set is the minimum that expresses Feature 001's established behavior
//! across the supported agents. Feature 002 adds no canonical event that no
//! supported agent signals (FR-113).

use crate::wire::ObservationInput;
use serde::{Deserialize, Serialize};

/// One of Cairn's own lifecycle boundaries.
///
/// `AgentQuiesced` is deliberately weaker than "the agent finished answering".
/// It asserts only that the agent has stopped working and is waiting, which is
/// the strongest claim every supported agent actually establishes — one signals
/// that it finished responding, another that its turn stopped, and a third only
/// that the session became idle, which can follow an error as readily as an
/// answer (FR-230, D21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalEvent {
    /// Starts or resumes a session; the context-delivery point.
    SessionOpened,
    /// A tool call the vendor reported as successful.
    ToolSucceeded,
    /// A tool call whose payload *establishes* failure. Never inferred.
    ToolFailed,
    /// The agent stopped working and is waiting. Checkpoint only.
    AgentQuiesced,
    /// Compaction is about to happen: produces a durable handoff.
    ContextCompacting,
    /// Compaction finished: the session stays usable, no second handoff.
    ContextCompacted,
    /// The session actually terminated.
    SessionClosed,
}

impl CanonicalEvent {
    /// Every canonical event, in vocabulary order.
    pub const ALL: [CanonicalEvent; 7] = [
        CanonicalEvent::SessionOpened,
        CanonicalEvent::ToolSucceeded,
        CanonicalEvent::ToolFailed,
        CanonicalEvent::AgentQuiesced,
        CanonicalEvent::ContextCompacting,
        CanonicalEvent::ContextCompacted,
        CanonicalEvent::SessionClosed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CanonicalEvent::SessionOpened => "session_opened",
            CanonicalEvent::ToolSucceeded => "tool_succeeded",
            CanonicalEvent::ToolFailed => "tool_failed",
            CanonicalEvent::AgentQuiesced => "agent_quiesced",
            CanonicalEvent::ContextCompacting => "context_compacting",
            CanonicalEvent::ContextCompacted => "context_compacted",
            CanonicalEvent::SessionClosed => "session_closed",
        }
    }

    /// Boundary/context class events get the longer deadline; the rest are
    /// capture class, where a missed deadline drops the event and is not a
    /// failure (`contracts/lifecycle.md`).
    pub fn is_boundary_class(self) -> bool {
        matches!(
            self,
            CanonicalEvent::SessionOpened
                | CanonicalEvent::ContextCompacting
                | CanonicalEvent::SessionClosed
        )
    }

    /// True where the event produces a durable handoff.
    ///
    /// `AgentQuiesced` and `ContextCompacted` deliberately do not (FR-114,
    /// FR-119, FR-230).
    pub fn produces_durable_handoff(self) -> bool {
        matches!(
            self,
            CanonicalEvent::ContextCompacting | CanonicalEvent::SessionClosed
        )
    }
}

/// A canonical event plus the identity of the agent session that produced it.
///
/// The identity is not optional: concurrent sessions are routed by it, and an
/// event that cannot name its session cannot be routed (FR-010, FR-118).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalLifecycleEvent {
    pub event: CanonicalEvent,
    /// Provenance only. Never a memory scope (FR-189).
    pub agent: String,
    /// The vendor's own session identifier.
    pub agent_session_key: String,
    pub cwd: String,
    /// `session_opened` only: startup | resume | clear | compact | fork.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Compaction events only: manual | auto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// `session_closed` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Tool events only. `agent_quiesced` never carries one (FR-231).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ObservationInput>,
}

impl CanonicalLifecycleEvent {
    /// Construct an event carrying no observation.
    pub fn new(
        event: CanonicalEvent,
        agent: impl Into<String>,
        agent_session_key: impl Into<String>,
        cwd: impl Into<String>,
    ) -> Self {
        Self {
            event,
            agent: agent.into(),
            agent_session_key: agent_session_key.into(),
            cwd: cwd.into(),
            source: None,
            trigger: None,
            reason: None,
            observation: None,
        }
    }

    pub fn with_source(mut self, source: Option<String>) -> Self {
        self.source = source;
        self
    }

    pub fn with_trigger(mut self, trigger: Option<String>) -> Self {
        self.trigger = trigger;
        self
    }

    pub fn with_reason(mut self, reason: Option<String>) -> Self {
        self.reason = reason;
        self
    }

    pub fn with_observation(mut self, observation: ObservationInput) -> Self {
        self.observation = Some(observation);
        self
    }

    /// The invariant every adapter output must satisfy before it reaches the
    /// daemon: only tool events carry observations, and quiescence never does
    /// (FR-231, `data-model.md` §CanonicalLifecycleEvent).
    pub fn is_well_formed(&self) -> bool {
        if self.agent_session_key.is_empty() {
            return false;
        }
        match self.event {
            CanonicalEvent::ToolSucceeded | CanonicalEvent::ToolFailed => {
                self.observation.is_some()
            }
            _ => self.observation.is_none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_is_exactly_seven_events() {
        assert_eq!(CanonicalEvent::ALL.len(), 7);
        let mut names: Vec<&str> = CanonicalEvent::ALL.iter().map(|e| e.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 7, "an event name is duplicated");
    }

    #[test]
    fn only_two_events_produce_a_durable_handoff() {
        let producing: Vec<_> = CanonicalEvent::ALL
            .iter()
            .filter(|e| e.produces_durable_handoff())
            .map(|e| e.as_str())
            .collect();
        assert_eq!(producing, vec!["context_compacting", "session_closed"]);
    }

    #[test]
    fn quiescence_and_post_compaction_write_no_handoff() {
        // FR-114, FR-119, FR-230: both leave the session usable and produce no
        // durable record of their own.
        assert!(!CanonicalEvent::AgentQuiesced.produces_durable_handoff());
        assert!(!CanonicalEvent::ContextCompacted.produces_durable_handoff());
    }

    #[test]
    fn quiescence_may_not_carry_an_observation() {
        let mut e = CanonicalLifecycleEvent::new(
            CanonicalEvent::AgentQuiesced,
            "opencode",
            "s-1",
            "/tmp/repo",
        );
        assert!(e.is_well_formed());
        e.observation = Some(ObservationInput {
            kind: crate::domain::ObservationType::Discovery,
            path: None,
            command: None,
            exit_code: None,
            outcome: None,
            summary: "x".into(),
            details: None,
            vendor_tool: None,
        });
        assert!(
            !e.is_well_formed(),
            "quiescence must never carry an outcome"
        );
    }

    #[test]
    fn an_event_without_a_session_key_is_not_routable() {
        let e =
            CanonicalLifecycleEvent::new(CanonicalEvent::SessionOpened, "codex", "", "/tmp/repo");
        assert!(!e.is_well_formed());
    }
}

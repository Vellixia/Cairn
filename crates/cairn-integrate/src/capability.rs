//! The capability model and the level derivation (FR-107–FR-111, FR-241,
//! FR-242, FR-245, D19, D19a).
//!
//! Two dimensions, and both are load-bearing.
//!
//! **Availability** is what the vendor's own documented behavior provides.
//! It is static data per adapter, refined by detection — never a probe, never
//! inferred from a similarly named capability (FR-108).
//!
//! **Confidence** is what Cairn has actually established *on this
//! installation*. It never raises a level. It withholds FULL while any
//! FULL-required capability is merely expected (FR-245), which is the hole
//! that gating only the completion guarantee left open: a vendor update that
//! removed tool capture would have kept static availability `guaranteed`, left
//! the missing capability `expected`, and nothing would have consulted it.

use crate::model::{ActivationState, AgentId, IntegrationLevel, ResourceKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The integration surfaces and lifecycle signals a profile describes
/// (FR-107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    McpUserScope,
    McpProjectScope,
    InstructionsProject,
    SkillUser,
    SkillProject,
    LifecycleSessionOpen,
    LifecycleToolSuccess,
    LifecycleToolFailure,
    LifecycleQuiesce,
    LifecyclePreCompaction,
    LifecyclePostCompaction,
    LifecycleSessionClose,
    ContextAtSessionOpen,
    StableSessionIdentifier,
}

impl Capability {
    pub const ALL: [Capability; 14] = [
        Capability::McpUserScope,
        Capability::McpProjectScope,
        Capability::InstructionsProject,
        Capability::SkillUser,
        Capability::SkillProject,
        Capability::LifecycleSessionOpen,
        Capability::LifecycleToolSuccess,
        Capability::LifecycleToolFailure,
        Capability::LifecycleQuiesce,
        Capability::LifecyclePreCompaction,
        Capability::LifecyclePostCompaction,
        Capability::LifecycleSessionClose,
        Capability::ContextAtSessionOpen,
        Capability::StableSessionIdentifier,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Capability::McpUserScope => "mcp_user_scope",
            Capability::McpProjectScope => "mcp_project_scope",
            Capability::InstructionsProject => "instructions_project",
            Capability::SkillUser => "skill_user",
            Capability::SkillProject => "skill_project",
            Capability::LifecycleSessionOpen => "lifecycle_session_open",
            Capability::LifecycleToolSuccess => "lifecycle_tool_success",
            Capability::LifecycleToolFailure => "lifecycle_tool_failure",
            Capability::LifecycleQuiesce => "lifecycle_quiesce",
            Capability::LifecyclePreCompaction => "lifecycle_pre_compaction",
            Capability::LifecyclePostCompaction => "lifecycle_post_compaction",
            Capability::LifecycleSessionClose => "lifecycle_session_close",
            Capability::ContextAtSessionOpen => "context_at_session_open",
            Capability::StableSessionIdentifier => "stable_session_identifier",
        }
    }

    pub fn parse(s: &str) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.as_str() == s)
    }

    /// True for the lifecycle signals, which are what a trust gate suspends.
    pub fn is_lifecycle(self) -> bool {
        matches!(
            self,
            Capability::LifecycleSessionOpen
                | Capability::LifecycleToolSuccess
                | Capability::LifecycleToolFailure
                | Capability::LifecycleQuiesce
                | Capability::LifecyclePreCompaction
                | Capability::LifecyclePostCompaction
                | Capability::LifecycleSessionClose
        )
    }

    /// The capability one canonical event demonstrates.
    ///
    /// Every lifecycle capability corresponds to exactly one canonical event
    /// and vice versa, which is what lets a fixture assert "the profile claims
    /// this, and here is the payload that proves it" without a second table.
    pub fn for_event(event: cairn_core::lifecycle::CanonicalEvent) -> Capability {
        use cairn_core::lifecycle::CanonicalEvent as E;
        match event {
            E::SessionOpened => Capability::LifecycleSessionOpen,
            E::ToolSucceeded => Capability::LifecycleToolSuccess,
            E::ToolFailed => Capability::LifecycleToolFailure,
            E::AgentQuiesced => Capability::LifecycleQuiesce,
            E::ContextCompacting => Capability::LifecyclePreCompaction,
            E::ContextCompacted => Capability::LifecyclePostCompaction,
            E::SessionClosed => Capability::LifecycleSessionClose,
        }
    }

    /// What Cairn is waiting to see before it will call this established.
    ///
    /// Phrased as the observation rather than the capability, because that is
    /// what a developer can act on: run an ordinary session (FR-245).
    pub fn awaited_behavior(self) -> &'static str {
        match self {
            Capability::LifecycleSessionOpen => "a first session opened on this installation",
            Capability::LifecycleToolSuccess => "a first tool call captured here",
            Capability::LifecycleQuiesce => "a first turn checkpoint here",
            Capability::LifecycleSessionClose => "a first session closed here",
            Capability::ContextAtSessionOpen => "context delivered at a session start here",
            Capability::StableSessionIdentifier => {
                "two events carrying the agent's own session identifier"
            }
            _ => "a first observation of this behavior here",
        }
    }

    /// The plain-language behavior a developer loses when this is absent.
    /// Never a numeric or unlabeled score (FR-111).
    pub fn missing_behavior(self) -> &'static str {
        match self {
            Capability::McpUserScope => "Cairn's tools registered once for every repository",
            Capability::McpProjectScope => "Cairn's tools registered for this repository only",
            Capability::InstructionsProject => {
                "the Cairn usage contract in the agent's instructions"
            }
            Capability::SkillUser => "the Cairn Skill installed for this user",
            Capability::SkillProject => "the Cairn Skill installed for this repository",
            Capability::LifecycleSessionOpen => "automatic session start",
            Capability::LifecycleToolSuccess => "automatic capture of tool calls",
            Capability::LifecycleToolFailure => "automatic capture of tool failures",
            Capability::LifecycleQuiesce => "turn checkpoints when the agent stops working",
            Capability::LifecyclePreCompaction => "a durable handoff before compaction",
            Capability::LifecyclePostCompaction => "context re-delivery after compaction",
            Capability::LifecycleSessionClose => "automatic session completion",
            Capability::ContextAtSessionOpen => "project context delivered at session start",
            Capability::StableSessionIdentifier => "a stable session identity from the agent",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the vendor's documented behavior provides (FR-241).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// The agent's documented behavior always provides it.
    Guaranteed,
    /// The agent provides it only when a particular payload or configuration
    /// makes it determinable. Never counts towards FULL, and is always
    /// reported with what it depends on.
    Conditional,
    Absent,
    /// Installed, but the agent will not run it until the user trusts it.
    PendingActivation,
}

impl Availability {
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::Guaranteed => "guaranteed",
            Availability::Conditional => "conditional",
            Availability::Absent => "absent",
            Availability::PendingActivation => "pending_activation",
        }
    }
}

/// What Cairn has established here, versus what it merely expects (FR-242).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Cairn holds an evidence row for it on this installation.
    Verified,
    /// The vendor documents it; Cairn has not established it here.
    Expected,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Verified => "verified",
            Confidence::Expected => "expected",
        }
    }
}

/// How a capability was established (`data-model.md` §CapabilityEvidence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// A fact about a resource Cairn itself wrote, read back. Version
    /// independent: a version change re-derives it rather than discarding it.
    Introspection,
    /// A runtime behavior observed producing its canonical event. Version
    /// bound: what a previous build did is not evidence about this one.
    Observation,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::Introspection => "introspection",
            EvidenceKind::Observation => "observation",
        }
    }
    pub fn parse(s: &str) -> Option<EvidenceKind> {
        match s {
            "introspection" => Some(EvidenceKind::Introspection),
            "observation" => Some(EvidenceKind::Observation),
            _ => None,
        }
    }
    /// Introspection proves a fact about Cairn's own artifact, so a vendor
    /// version change does not invalidate it (FR-245).
    pub fn is_version_independent(self) -> bool {
        self == EvidenceKind::Introspection
    }
}

/// One established fact, as persisted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub capability: String,
    pub kind: EvidenceKind,
    pub agent_version: Option<String>,
    /// `context_at_session_open` only: whether the establishing delivery
    /// carried a degraded briefing (D19a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<bool>,
}

/// Evidence keys for the three configuration resources FULL requires.
///
/// The pseudo-code in `data-model.md` §Level derivation names these `mcp`,
/// `instructions` and `skill`; they share one keyspace with the runtime
/// capability names, which do not collide with them.
pub fn config_evidence_key(kind: ResourceKind) -> &'static str {
    kind.as_str()
}

/// Whether the completion guarantee has been demonstrated (FR-207).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionGuarantee {
    Demonstrated,
    NotDemonstrated,
    PendingActivation,
}

impl CompletionGuarantee {
    pub fn as_str(self) -> &'static str {
        match self {
            CompletionGuarantee::Demonstrated => "demonstrated",
            CompletionGuarantee::NotDemonstrated => "not_demonstrated",
            CompletionGuarantee::PendingActivation => "pending_activation",
        }
    }
}

/// One capability's two dimensions, plus what a conditional one depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityState {
    pub availability: Availability,
    pub confidence: Confidence,
    /// Mandatory where availability is `conditional` (FR-241).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<String>,
}

impl CapabilityState {
    fn new(availability: Availability) -> Self {
        Self {
            availability,
            confidence: Confidence::Expected,
            depends_on: None,
        }
    }
    fn conditional(depends_on: &str) -> Self {
        Self {
            availability: Availability::Conditional,
            confidence: Confidence::Expected,
            depends_on: Some(depends_on.to_string()),
        }
    }
    /// `established` from the derivation rule: guaranteed **and** verified.
    /// Conditional never satisfies it; expected never satisfies it.
    pub fn established(&self) -> bool {
        self.availability == Availability::Guaranteed && self.confidence == Confidence::Verified
    }
}

/// Per agent, which surfaces and signals that agent actually provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityProfile {
    pub agent: AgentId,
    pub capabilities: BTreeMap<Capability, CapabilityState>,
    /// Whether lifecycle handlers require a user trust step inside the agent
    /// before they run (FR-107, D24).
    pub handlers_require_trust: bool,
    /// Introspection evidence per configuration resource kind.
    pub config_verified: BTreeMap<ResourceKind, bool>,
}

impl CapabilityProfile {
    /// The static profile for one agent, before any refinement.
    ///
    /// These are vendor facts recorded from official sources (D30–D33), not
    /// probes. Two entries are `conditional` and both are load-bearing —
    /// see `contracts/lifecycle.md` §OpenCode.
    pub fn base(agent: AgentId) -> Self {
        use Availability::*;
        use Capability::*;
        let mut c: BTreeMap<Capability, CapabilityState> = BTreeMap::new();
        macro_rules! set {
            ($k:expr, $a:expr) => {
                c.insert($k, CapabilityState::new($a))
            };
        }
        match agent {
            AgentId::ClaudeCode | AgentId::Codex => {
                for k in Capability::ALL {
                    set!(k, Guaranteed);
                }
            }
            AgentId::Opencode => {
                for k in Capability::ALL {
                    set!(k, Guaranteed);
                }
                // `tool.execute.after` carries no outcome flag, and a tool
                // that throws may not reach the hook at all. Provable failures
                // do exist, so this is conditional rather than absent —
                // discarding them would be as dishonest as inventing them.
                c.insert(
                    LifecycleToolFailure,
                    CapabilityState::conditional(
                        "OpenCode's tool output unambiguously establishing a failure",
                    ),
                );
                c.insert(
                    LifecyclePreCompaction,
                    CapabilityState::conditional(
                        "the installed OpenCode exposing experimental.session.compacting",
                    ),
                );
                // The one genuine absence: OpenCode signals no session end at
                // all (FR-115, SC-131).
                set!(LifecycleSessionClose, Absent);
            }
            AgentId::GenericMcp => {
                for k in Capability::ALL {
                    set!(k, Absent);
                }
                set!(McpUserScope, Guaranteed);
            }
        }
        Self {
            agent,
            capabilities: c,
            handlers_require_trust: agent == AgentId::Codex,
            config_verified: BTreeMap::new(),
        }
    }

    pub fn get(&self, c: Capability) -> &CapabilityState {
        self.capabilities
            .get(&c)
            .expect("every capability is present in every profile")
    }

    /// Suspend the lifecycle capabilities of a trust-gated agent until the
    /// user has activated them (FR-209, D24).
    ///
    /// An agent whose handlers are installed but not yet trusted is never
    /// reported at the level those handlers would provide once active.
    pub fn apply_activation(&mut self, activation: ActivationState) {
        if !self.handlers_require_trust {
            return;
        }
        if activation.needs_user_action() {
            for c in Capability::ALL.iter().filter(|c| c.is_lifecycle()) {
                if let Some(state) = self.capabilities.get_mut(c) {
                    if state.availability != Availability::Absent {
                        state.availability = Availability::PendingActivation;
                        state.confidence = Confidence::Expected;
                    }
                }
            }
        }
    }

    /// Raise confidence to `verified` for every capability that holds
    /// evidence, and record which configuration resources are verified.
    ///
    /// Confidence never raises a level; it only stops FULL being claimed on
    /// documentation alone (FR-242).
    pub fn apply_evidence(&mut self, evidence: &[Evidence]) {
        for e in evidence {
            if let Some(cap) = Capability::parse(&e.capability) {
                if let Some(state) = self.capabilities.get_mut(&cap) {
                    state.confidence = Confidence::Verified;
                }
            } else if let Some(kind) = ResourceKind::parse(&e.capability) {
                self.config_verified.insert(kind, true);
            }
        }
    }

    /// The configuration resources FULL requires, for this agent.
    pub fn full_required_config(&self) -> Vec<ResourceKind> {
        let mut v = vec![ResourceKind::Mcp, ResourceKind::Instructions];
        if self.agent.supports_skills() {
            v.push(ResourceKind::Skill);
        }
        if self.agent == AgentId::GenericMcp {
            v = vec![ResourceKind::Mcp];
        }
        v
    }

    /// The runtime capabilities FULL requires
    /// (`data-model.md` §Level derivation).
    pub const FULL_REQUIRED_RUNTIME: [Capability; 6] = [
        Capability::LifecycleSessionOpen,
        Capability::LifecycleToolSuccess,
        Capability::LifecycleQuiesce,
        Capability::LifecycleSessionClose,
        Capability::ContextAtSessionOpen,
        Capability::StableSessionIdentifier,
    ];

    /// Capabilities FULL requires that are not yet established, named in plain
    /// language for `awaited_behaviors` (FR-245).
    pub fn awaited(&self) -> Vec<String> {
        let mut out = Vec::new();
        for kind in self.full_required_config() {
            if !self.config_verified.get(&kind).copied().unwrap_or(false) {
                out.push(format!(
                    "the {kind} resource read back from this installation"
                ));
            }
        }
        for c in Self::FULL_REQUIRED_RUNTIME {
            let state = self.get(c);
            if state.availability == Availability::Guaranteed
                && state.confidence == Confidence::Expected
            {
                out.push(c.awaited_behavior().to_string());
            }
        }
        out
    }

    /// Capabilities the vendor documents but Cairn has not established here.
    /// Mandatory whenever any capability is expected (FR-188, FR-242).
    pub fn unverified(&self) -> Vec<String> {
        Capability::ALL
            .iter()
            .filter(|c| {
                let s = self.get(**c);
                s.availability != Availability::Absent && s.confidence == Confidence::Expected
            })
            .map(|c| c.as_str().to_string())
            .collect()
    }

    /// The behaviors the developer does not get. Mandatory below FULL
    /// (FR-111).
    pub fn missing_behaviors(&self) -> Vec<String> {
        Capability::ALL
            .iter()
            .filter(|c| self.get(**c).availability == Availability::Absent)
            .map(|c| c.missing_behavior().to_string())
            .collect()
    }

    /// The conditional entries, each with what it depends on (FR-241).
    pub fn conditional_behaviors(&self) -> Vec<String> {
        Capability::ALL
            .iter()
            .filter_map(|c| {
                let s = self.get(*c);
                (s.availability == Availability::Conditional).then(|| {
                    format!(
                        "{} only where {}",
                        c.missing_behavior(),
                        s.depends_on
                            .as_deref()
                            .unwrap_or("the payload establishes it")
                    )
                })
            })
            .collect()
    }

    /// Lifecycle coverage, split three ways. All three lists are mandatory and
    /// are what make an honest level auditable (FR-168).
    pub fn lifecycle_coverage(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let (mut g, mut c, mut a) = (Vec::new(), Vec::new(), Vec::new());
        for cap in Capability::ALL.iter().filter(|c| c.is_lifecycle()) {
            let name = cap.as_str().to_string();
            match self.get(*cap).availability {
                Availability::Guaranteed => g.push(name),
                Availability::Conditional => c.push(name),
                Availability::Absent => a.push(name),
                Availability::PendingActivation => c.push(name),
            }
        }
        (g, c, a)
    }
}

/// Everything level derivation needs beyond the profile itself.
#[derive(Debug, Clone, Default)]
pub struct LevelInputs {
    /// Whether the Cairn MCP entry is installed and reachable for this agent.
    pub mcp_present: bool,
    /// Whether the managed instruction block is installed.
    pub instructions_present: bool,
    /// Whether the Cairn Skill is installed.
    pub skill_present: bool,
    /// Whether the agent is detected at all.
    pub detected: bool,
    /// Whether any safe integration exists for this version (FR-187).
    pub safe_to_integrate: bool,
    /// A boundary that acknowledged but has not produced its handoff yet.
    /// Until it does, the completion guarantee is not satisfied for it
    /// (FR-240 clause 4).
    pub boundary_owed: bool,
    /// The trust state of a trust-gated agent.
    pub activation: ActivationState,
    /// Whether measurement has shown session-end work fits the vendor's own
    /// handler budget (FR-208, SC-128). `None` where the agent imposes none.
    pub close_within_budget: Option<bool>,
}

impl LevelInputs {
    pub fn detected_and_safe() -> Self {
        Self {
            detected: true,
            safe_to_integrate: true,
            activation: ActivationState::NotApplicable,
            ..Default::default()
        }
    }
}

/// The computed outcome. Nothing here is hardcoded by agent name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelOutcome {
    pub level: IntegrationLevel,
    pub completion_guarantee: CompletionGuarantee,
    pub missing_behaviors: Vec<String>,
    pub awaited_behaviors: Vec<String>,
    pub unverified_behaviors: Vec<String>,
}

/// Derive the completion guarantee (FR-207, FR-229, FR-240).
///
/// **Cairn's inactivity timeout and daemon-start reconciliation never set
/// it.** They are recovery from silence: they may backstop a boundary that was
/// missed, and they never constitute one. There is deliberately no input to
/// this function through which they could.
pub fn completion_guarantee(
    profile: &CapabilityProfile,
    inputs: &LevelInputs,
) -> CompletionGuarantee {
    let close = profile.get(Capability::LifecycleSessionClose);
    if close.availability == Availability::PendingActivation {
        return CompletionGuarantee::PendingActivation;
    }
    // A boundary that acknowledged but has not yet produced its handoff reads
    // as owed, not complete (FR-240 clause 4).
    if inputs.boundary_owed {
        return CompletionGuarantee::NotDemonstrated;
    }
    // Where the agent imposes a handler deadline, the adapter must be *shown*
    // by measurement to fit inside it, or the guarantee is not claimed
    // (FR-208).
    if inputs.close_within_budget == Some(false) {
        return CompletionGuarantee::NotDemonstrated;
    }
    if close.established() {
        CompletionGuarantee::Demonstrated
    } else {
        CompletionGuarantee::NotDemonstrated
    }
}

/// Derive the integration level (FR-109, FR-245).
pub fn derive_level(profile: &CapabilityProfile, inputs: &LevelInputs) -> LevelOutcome {
    let guarantee = completion_guarantee(profile, inputs);
    let missing_behaviors = profile.missing_behaviors();
    let awaited_behaviors = profile.awaited();
    let unverified_behaviors = profile.unverified();

    let level = if !inputs.detected || !inputs.safe_to_integrate {
        IntegrationLevel::Unsupported
    } else if !inputs.mcp_present {
        // Nothing is claimed without the tool surface.
        IntegrationLevel::Unsupported
    } else {
        let config_established = profile
            .full_required_config()
            .into_iter()
            .all(|k| profile.config_verified.get(&k).copied().unwrap_or(false));
        let runtime_established = CapabilityProfile::FULL_REQUIRED_RUNTIME
            .iter()
            .all(|c| profile.get(*c).established());

        if config_established
            && runtime_established
            && guarantee == CompletionGuarantee::Demonstrated
        {
            IntegrationLevel::Full
        } else {
            let any_lifecycle = Capability::ALL
                .iter()
                .any(|c| c.is_lifecycle() && profile.get(*c).availability != Availability::Absent);
            if inputs.instructions_present || inputs.skill_present || any_lifecycle {
                IntegrationLevel::McpPlus
            } else {
                IntegrationLevel::McpOnly
            }
        }
    };

    LevelOutcome {
        level,
        completion_guarantee: guarantee,
        missing_behaviors,
        awaited_behaviors,
        unverified_behaviors,
    }
}

/// Classify a detected agent version (FR-185, FR-186, FR-187).
///
/// Unknown is *not* an error: a version newer than any Cairn has verified is
/// compatible-but-unverified and is used. Only a version Cairn positively
/// knows is incompatible is reported unsupported.
pub fn classify_version(
    agent: AgentId,
    version: Option<&str>,
) -> (crate::model::CompatibilityClassification, Option<String>) {
    use crate::model::CompatibilityClassification as C;
    let Some(v) = version else {
        return (C::CompatibleUnverified, None);
    };
    if let Some(reason) = known_incompatible(agent, v) {
        return (C::Unsupported, Some(reason));
    }
    if verified_versions(agent).contains(&v) {
        return (C::Verified, None);
    }
    (C::CompatibleUnverified, None)
}

/// Versions Cairn has actually verified against. Deliberately a short, closed
/// list — everything else is compatible-but-unverified, not broken.
fn verified_versions(agent: AgentId) -> &'static [&'static str] {
    match agent {
        AgentId::ClaudeCode => &["2.1.220"],
        AgentId::Codex => &["0.58.0"],
        AgentId::Opencode => &["1.4.2"],
        AgentId::GenericMcp => &[],
    }
}

/// Versions Cairn positively knows are incompatible, with what is
/// incompatible about them (FR-187). Empty is the honest default.
fn known_incompatible(_agent: AgentId, _version: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn established_profile(agent: AgentId) -> CapabilityProfile {
        let mut p = CapabilityProfile::base(agent);
        let mut evidence: Vec<Evidence> = CapabilityProfile::FULL_REQUIRED_RUNTIME
            .iter()
            .map(|c| Evidence {
                capability: c.as_str().to_string(),
                kind: EvidenceKind::Observation,
                agent_version: Some("1.0.0".into()),
                degraded: None,
            })
            .collect();
        for k in p.full_required_config() {
            evidence.push(Evidence {
                capability: k.as_str().to_string(),
                kind: EvidenceKind::Introspection,
                agent_version: Some("1.0.0".into()),
                degraded: None,
            });
        }
        p.apply_evidence(&evidence);
        p
    }

    fn full_inputs() -> LevelInputs {
        LevelInputs {
            mcp_present: true,
            instructions_present: true,
            skill_present: true,
            ..LevelInputs::detected_and_safe()
        }
    }

    #[test]
    fn full_requires_every_full_required_capability_established() {
        let p = established_profile(AgentId::ClaudeCode);
        assert_eq!(
            derive_level(&p, &full_inputs()).level,
            IntegrationLevel::Full
        );
    }

    #[test]
    fn an_expected_capability_withholds_full_without_making_it_unsupported() {
        // FR-245: lapsed evidence is never `unsupported`.
        let mut p = established_profile(AgentId::ClaudeCode);
        p.capabilities
            .get_mut(&Capability::LifecycleToolSuccess)
            .unwrap()
            .confidence = Confidence::Expected;
        let out = derive_level(&p, &full_inputs());
        assert_eq!(out.level, IntegrationLevel::McpPlus);
        assert!(
            !out.awaited_behaviors.is_empty(),
            "must name what is awaited"
        );
    }

    #[test]
    fn a_conditional_capability_never_counts_towards_full() {
        // FR-241: conditional is neither present nor absent, and never FULL.
        let mut p = established_profile(AgentId::ClaudeCode);
        let state = p
            .capabilities
            .get_mut(&Capability::LifecycleSessionClose)
            .unwrap();
        state.availability = Availability::Conditional;
        state.confidence = Confidence::Verified;
        assert!(!state.established());
        assert_ne!(
            derive_level(&p, &full_inputs()).level,
            IntegrationLevel::Full
        );
    }

    #[test]
    fn opencode_is_below_full_under_current_vendor_behavior() {
        // Derived, not hardcoded: the absent session-close capability is what
        // does it (SC-131).
        let p = established_profile(AgentId::Opencode);
        let out = derive_level(&p, &full_inputs());
        assert_eq!(out.level, IntegrationLevel::McpPlus);
        assert_eq!(
            out.completion_guarantee,
            CompletionGuarantee::NotDemonstrated
        );
        assert!(out
            .missing_behaviors
            .iter()
            .any(|b| b.contains("automatic session completion")));
    }

    #[test]
    fn recovery_from_silence_cannot_produce_full() {
        // FR-229, SC-131: there is deliberately no input through which the
        // idle reaper or daemon-start reconciliation could reach this.
        let p = established_profile(AgentId::Opencode);
        for owed in [true, false] {
            let inputs = LevelInputs {
                boundary_owed: owed,
                ..full_inputs()
            };
            assert_ne!(derive_level(&p, &inputs).level, IntegrationLevel::Full);
        }
    }

    #[test]
    fn an_owed_boundary_withholds_the_completion_guarantee() {
        // FR-240 clause 4.
        let p = established_profile(AgentId::ClaudeCode);
        let inputs = LevelInputs {
            boundary_owed: true,
            ..full_inputs()
        };
        let out = derive_level(&p, &inputs);
        assert_eq!(
            out.completion_guarantee,
            CompletionGuarantee::NotDemonstrated
        );
        assert_ne!(out.level, IntegrationLevel::Full);
    }

    #[test]
    fn an_untrusted_codex_is_pending_activation_not_full() {
        // FR-209, D24.
        let mut p = established_profile(AgentId::Codex);
        p.apply_activation(ActivationState::PendingUserTrust);
        let out = derive_level(&p, &full_inputs());
        assert_eq!(
            out.completion_guarantee,
            CompletionGuarantee::PendingActivation
        );
        assert_ne!(out.level, IntegrationLevel::Full);
    }

    #[test]
    fn a_close_that_misses_the_vendor_budget_is_not_full() {
        // FR-208, SC-128: measurement, not assertion.
        let p = established_profile(AgentId::Codex);
        let inputs = LevelInputs {
            close_within_budget: Some(false),
            ..full_inputs()
        };
        assert_ne!(derive_level(&p, &inputs).level, IntegrationLevel::Full);
    }

    #[test]
    fn generic_mcp_is_mcp_only_and_names_what_is_unavailable() {
        // SC-107.
        let mut p = CapabilityProfile::base(AgentId::GenericMcp);
        p.apply_evidence(&[Evidence {
            capability: "mcp".into(),
            kind: EvidenceKind::Introspection,
            agent_version: None,
            degraded: None,
        }]);
        let inputs = LevelInputs {
            mcp_present: true,
            ..LevelInputs::detected_and_safe()
        };
        let out = derive_level(&p, &inputs);
        assert_eq!(out.level, IntegrationLevel::McpOnly);
        assert!(out
            .missing_behaviors
            .iter()
            .any(|b| b.contains("automatic capture of tool calls")));
        assert!(out
            .missing_behaviors
            .iter()
            .any(|b| b.contains("automatic session start")));
    }

    #[test]
    fn an_absent_capability_names_a_behavior_not_a_score() {
        for c in Capability::ALL {
            let b = c.missing_behavior();
            assert!(!b.is_empty());
            assert!(
                !b.chars().any(|ch| ch.is_ascii_digit()),
                "{c} reads like a score"
            );
        }
    }

    #[test]
    fn unknown_versions_integrate_and_only_known_bad_ones_are_unsupported() {
        use crate::model::CompatibilityClassification as C;
        // FR-186, SC-123.
        assert_eq!(
            classify_version(AgentId::ClaudeCode, Some("99.0.0")).0,
            C::CompatibleUnverified
        );
        assert_eq!(
            classify_version(AgentId::ClaudeCode, Some("2.1.220")).0,
            C::Verified
        );
        assert_eq!(
            classify_version(AgentId::Codex, None).0,
            C::CompatibleUnverified
        );
    }

    #[test]
    fn introspection_evidence_survives_a_version_change_and_observation_does_not() {
        // FR-245, SC-138 — the rule the store applies, asserted on the type.
        assert!(EvidenceKind::Introspection.is_version_independent());
        assert!(!EvidenceKind::Observation.is_version_independent());
    }

    #[test]
    fn every_conditional_entry_names_what_it_depends_on() {
        // FR-241: never reported as present, always with its condition.
        let p = CapabilityProfile::base(AgentId::Opencode);
        for c in Capability::ALL {
            let s = p.get(c);
            if s.availability == Availability::Conditional {
                assert!(s.depends_on.is_some(), "{c} is conditional on nothing");
            }
        }
        assert_eq!(p.conditional_behaviors().len(), 2);
    }
}

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

/// A vendor's own deadline for its session-boundary handler.
///
/// A vendor fact, held in the profile rather than read from an agent name at
/// derivation time. Its presence is what makes FR-208's measurement gate
/// apply: an agent that imposes a budget is not granted the completion
/// guarantee until measurement shows Cairn's session-end work fits inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandlerBudget {
    pub default_ms: u64,
    pub max_ms: u64,
}

/// Whether this build has *demonstrated* that its session-end work fits the
/// agent's own handler budget (FR-208, SC-128).
///
/// This is a claim about this build, and `tests/tests/perf_session_close.rs`
/// is its proof: the benchmark measures at least 100 real session-end
/// boundaries against the budget below and fails the build if the claim here
/// is not true of it. A running Cairn cannot re-derive it — one developer's
/// machine does not produce 100 boundaries — so the honest place for it is a
/// constant a test is obliged to keep true.
pub fn budget_demonstrated(agent: AgentId) -> bool {
    match agent {
        // Demonstrated by `perf_session_close`, which asserts this line.
        AgentId::Codex => true,
        // No budget imposed; nothing to demonstrate.
        AgentId::ClaudeCode | AgentId::Opencode | AgentId::GenericMcp => true,
    }
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
    /// The vendor's own session-end handler deadline, where it imposes one
    /// (FR-208).
    pub close_budget: Option<HandlerBudget>,
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
            AgentId::ClaudeCode => {
                for k in Capability::ALL {
                    set!(k, Guaranteed);
                }
                // The `PostCompact` hook fires, and that is not the same fact.
                // This capability is *context re-delivery* after compaction,
                // and Claude Code's hook cannot re-deliver: `additionalContext`
                // is unsupported for `PostCompact`, whose output the vendor
                // documents as "shows stderr to user only". Cairn is told the
                // compaction happened and has no way to hand the checkpoint
                // back through that channel.
                //
                // Recorded as `conditional` rather than `absent` because the
                // signal is real and Cairn does act on it — it restores the
                // checkpoint, so `cairn_context(reason=post_compaction)`
                // answers immediately. What is not guaranteed is that the agent
                // is *told* without asking, which is precisely the difference
                // between `automatic` and `agent_initiated` (FR-426).
                c.insert(
                    LifecyclePostCompaction,
                    CapabilityState::conditional(
                        "Claude Code's PostCompact hook can return context to the session",
                    ),
                );
            }
            AgentId::Codex => {
                for k in Capability::ALL {
                    set!(k, Guaranteed);
                }
                // Codex registers `PostCompact` and Cairn is called on it, but
                // this capability is *context re-delivery* after compaction,
                // and Cairn does not re-deliver on that path for any agent:
                // `ContextCompacted` is capture class, so `cairn hook` sends it
                // one-way and returns without ever reaching `emit_context`
                // (`crates/cairn/src/hook.rs`, where `delivers_context` is
                // `SessionOpened` alone).
                //
                // What the hook does do is restore the checkpoint, so
                // `cairn_context(reason=post_compaction)` answers immediately.
                // That is the same shape as Claude Code above: the signal is
                // real and acted on, and what is *not* guaranteed is that the
                // agent is told without asking -- precisely the difference
                // between `automatic` and `agent_initiated` (FR-426).
                //
                // This was `guaranteed` until it was read against the delivery
                // path rather than the hook registration. It had never been
                // driven live (T148, deferred to #42), and the identical
                // `automatic` claim for Claude Code was disproved the moment a
                // real compaction was run. Under-promising is permitted here;
                // over-claiming is the defect FR-426 forbids. If T148 ever
                // shows Codex genuinely re-delivering, this becomes
                // `guaranteed` on that evidence (F12).
                c.insert(
                    LifecyclePostCompaction,
                    CapabilityState::conditional(
                        "Codex's PostCompact hook can return context to the session",
                    ),
                );
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
                        "OpenCode's tool output unambiguously establishes a failure",
                    ),
                );
                c.insert(
                    LifecyclePreCompaction,
                    CapabilityState::conditional(
                        "the installed OpenCode exposes experimental.session.compacting",
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
            // `SESSION_END_DEFAULT_TIMEOUT_SEC = 1`,
            // `SESSION_END_MAX_TIMEOUT_SEC = 3` (D31). Cairn's session-end
            // work has to fit inside this, and be shown to (FR-208, SC-128).
            close_budget: (agent == AgentId::Codex).then_some(HandlerBudget {
                default_ms: 1_000,
                max_ms: 3_000,
            }),
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
    ///
    /// Lifecycle first, because that is what a developer notices: "no
    /// automatic session start" is the fact that explains why nothing is being
    /// captured, and where Cairn's tools happen to be registered is not.
    pub fn missing_behaviors(&self) -> Vec<String> {
        let absent = |lifecycle: bool| {
            Capability::ALL
                .iter()
                .filter(move |c| c.is_lifecycle() == lifecycle)
                .filter(|c| self.get(**c).availability == Availability::Absent)
                .map(|c| c.missing_behavior().to_string())
        };
        absent(true).chain(absent(false)).collect()
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

impl CapabilityProfile {
    /// What Cairn can honestly promise this agent about compression-safe
    /// continuity (FR-426, FR-427, D57).
    ///
    /// ```text
    /// pre     post                  mode
    /// ------  --------------------  ----------------------
    /// present present               automatic
    /// present absent / conditional  agent_initiated
    /// absent  any                   unavailable_automatic
    /// ```
    ///
    /// A **derived read** over the capability profile Feature 002 already
    /// maintains. No new canonical event and no new capability: a mode that
    /// needed its own signal would be a second source of truth about the same
    /// thing, and would drift from it.
    ///
    /// Cairn never reports a rehydration guarantee an adapter cannot provide.
    /// Claiming `automatic` for an agent whose post-compaction hook is
    /// experimental would be exactly the false promise US6 #4 is about.
    pub fn continuity_mode(&self) -> cairn_core::domain::ContinuityMode {
        let pre = self.get(Capability::LifecyclePreCompaction);
        let post = self.get(Capability::LifecyclePostCompaction);

        use cairn_core::domain::ContinuityMode;

        // Nothing warns Cairn at all: the agent must write its own checkpoint
        // before compacting, because there is no moment Cairn can do it.
        if matches!(
            pre.availability,
            Availability::Absent | Availability::PendingActivation
        ) {
            return ContinuityMode::UnavailableAutomatic;
        }

        // `automatic` is a promise that Cairn is called back on both sides, and
        // it is only keepable when **both** are guaranteed.
        //
        // A conditional pre-compaction is the case this originally got wrong.
        // OpenCode's warning depends on the installed build exposing
        // `experimental.session.compacting`, so on a build without it Cairn is
        // never told compaction is coming and the checkpoint is never written —
        // while the agent had been told continuity was automatic and did
        // nothing. Conditional and unavailable mean the same thing to an agent
        // that must act: it cannot rely on being called, so it must ask
        // (FR-426).
        match (pre.availability, post.availability) {
            (Availability::Guaranteed, Availability::Guaranteed) => ContinuityMode::Automatic,
            _ => ContinuityMode::AgentInitiated,
        }
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
    /// How this agent's sessions actually reach a terminal state, whenever
    /// that is not "the agent said so".
    ///
    /// Mandatory wherever the completion guarantee is not demonstrated. FR-229
    /// requires the report to say that sessions are closed by inactivity
    /// rather than completed, and FR-209 requires the action to be stated —
    /// both are this field.
    pub completion_note: Option<String>,
}

/// The plain-language account of how sessions terminate here (FR-207, FR-209,
/// FR-229).
fn completion_note(
    profile: &CapabilityProfile,
    inputs: &LevelInputs,
    guarantee: CompletionGuarantee,
) -> Option<String> {
    if guarantee == CompletionGuarantee::Demonstrated {
        return None;
    }
    let close = profile.get(Capability::LifecycleSessionClose);
    Some(match close.availability {
        // FR-209: what actually works now, and the action required.
        Availability::PendingActivation => "this agent's lifecycle handlers are installed but \
             not yet trusted, so sessions are not completed automatically; trust them inside \
             the agent to activate them"
            .to_string(),
        // FR-229: the honest account of a safety net. Reported wherever the
        // agent signals no termination, whatever the reason.
        Availability::Absent | Availability::Conditional => {
            "this agent signals no session end, so sessions here are closed by Cairn's \
             inactivity timeout rather than completed"
                .to_string()
        }
        Availability::Guaranteed => {
            if profile.close_budget.is_some() && inputs.close_within_budget != Some(true) {
                // FR-208: measurement, not assertion.
                "Cairn's session-end work has not been shown to fit this agent's own handler \
                 budget, so the completion guarantee is not claimed"
                    .to_string()
            } else if inputs.boundary_owed {
                "a session boundary here is still owed its handoff".to_string()
            } else {
                "no session has closed here yet, so automatic completion is not established"
                    .to_string()
            }
        }
    })
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
    // by measurement to fit inside it. Absence of a contradicting measurement
    // is not a demonstration, so an unmeasured budget withholds the guarantee
    // exactly as a failed one does (FR-208).
    if profile.close_budget.is_some() && inputs.close_within_budget != Some(true) {
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
    let mut missing_behaviors = profile.missing_behaviors();
    let awaited_behaviors = profile.awaited();
    let unverified_behaviors = profile.unverified();

    // FR-207 and SC-127: an agent without a demonstrated mechanism names
    // automatic session completion as the missing behavior. Not merely
    // awaited — this installation's configuration cannot produce it as it
    // stands, which is a different statement from "it has not happened yet".
    let close = profile.get(Capability::LifecycleSessionClose);
    let unproducible = close.availability != Availability::Guaranteed
        || (profile.close_budget.is_some() && inputs.close_within_budget != Some(true));
    let names_completion = Capability::LifecycleSessionClose.missing_behavior();
    if guarantee != CompletionGuarantee::Demonstrated
        && unproducible
        && !missing_behaviors.iter().any(|b| b == names_completion)
    {
        missing_behaviors.push(names_completion.to_string());
    }

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
        completion_note: completion_note(profile, inputs, guarantee),
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
    classify_against(KNOWN_INCOMPATIBLE, agent, version)
}

/// `classify_version` against a given incompatibility table.
///
/// Split out so the unsupported path is testable while the shipped table
/// stays empty — which is the honest state, not an untested one.
fn classify_against(
    table: &[(AgentId, &str, &str)],
    agent: AgentId,
    version: Option<&str>,
) -> (crate::model::CompatibilityClassification, Option<String>) {
    use crate::model::CompatibilityClassification as C;
    let Some(v) = version else {
        return (C::CompatibleUnverified, None);
    };
    if let Some(reason) = lookup(table, agent, v) {
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

/// Versions Cairn positively knows are incompatible, and what is incompatible
/// about them (FR-187).
///
/// Empty is the honest default and the expected steady state: a version Cairn
/// has not tried is not a version Cairn knows is broken. An entry belongs here
/// only when a specific incompatibility has been observed, and it carries the
/// explanation the report shows the developer.
const KNOWN_INCOMPATIBLE: &[(AgentId, &str, &str)] = &[];

fn lookup(table: &[(AgentId, &str, &str)], agent: AgentId, version: &str) -> Option<String> {
    table
        .iter()
        .find(|(a, v, _)| *a == agent && *v == version)
        .map(|(_, _, reason)| reason.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn established_profile(agent: AgentId) -> CapabilityProfile {
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

    // ------------------------------------------------- T066 — Codex gate ---

    /// FR-208: an agent that imposes a handler deadline is not granted the
    /// completion guarantee on the strength of no measurement at all.
    #[test]
    fn an_unmeasured_handler_budget_withholds_the_guarantee() {
        let p = established_profile(AgentId::Codex);
        assert!(
            p.close_budget.is_some(),
            "Codex imposes a session-end handler deadline (D31)"
        );
        for measured in [None, Some(false)] {
            let inputs = LevelInputs {
                close_within_budget: measured,
                ..full_inputs()
            };
            let out = derive_level(&p, &inputs);
            assert_eq!(
                out.completion_guarantee,
                CompletionGuarantee::NotDemonstrated,
                "an unmeasured budget was treated as a demonstrated one"
            );
            assert_ne!(out.level, IntegrationLevel::Full);
            assert!(
                out.missing_behaviors
                    .iter()
                    .any(|b| b.contains("automatic session completion")),
                "the report did not name what is missing: {:?}",
                out.missing_behaviors
            );
            assert!(out
                .completion_note
                .as_deref()
                .unwrap_or_default()
                .contains("handler budget"));
        }
    }

    /// The whole Codex gate: trust active, the budget demonstrated, and every
    /// FULL-required capability established. All three, or below FULL
    /// (FR-207, FR-208, FR-209, SC-127, SC-128).
    #[test]
    fn codex_reaches_full_only_with_trust_budget_and_every_capability() {
        let demonstrated = || LevelInputs {
            close_within_budget: Some(true),
            ..full_inputs()
        };

        // All three: FULL.
        let p = established_profile(AgentId::Codex);
        let out = derive_level(&p, &demonstrated());
        assert_eq!(out.level, IntegrationLevel::Full);
        assert_eq!(out.completion_guarantee, CompletionGuarantee::Demonstrated);
        assert_eq!(out.completion_note, None);

        // Trust withdrawn.
        let mut untrusted = established_profile(AgentId::Codex);
        untrusted.apply_activation(ActivationState::PendingUserTrust);
        let out = derive_level(&untrusted, &demonstrated());
        assert_ne!(out.level, IntegrationLevel::Full);
        assert_eq!(
            out.completion_guarantee,
            CompletionGuarantee::PendingActivation
        );
        assert!(
            out.missing_behaviors
                .iter()
                .any(|b| b.contains("automatic session completion")),
            "a pending-trust Codex did not name the behavior it is not providing"
        );
        // FR-209: the required action is stated, not merely the state.
        assert!(out
            .completion_note
            .as_deref()
            .unwrap_or_default()
            .contains("trust"));

        // One FULL-required capability never observed.
        let mut unobserved = established_profile(AgentId::Codex);
        unobserved
            .capabilities
            .get_mut(&Capability::LifecycleQuiesce)
            .unwrap()
            .confidence = Confidence::Expected;
        let out = derive_level(&unobserved, &demonstrated());
        assert_ne!(out.level, IntegrationLevel::Full);
        assert!(!out.awaited_behaviors.is_empty());
    }

    /// The gate is a property of the profile, not of the agent's name.
    #[test]
    fn only_an_agent_that_imposes_a_budget_is_gated_on_one() {
        for agent in [AgentId::ClaudeCode, AgentId::Opencode] {
            assert!(
                CapabilityProfile::base(agent).close_budget.is_none(),
                "{agent:?} imposes no documented session-end deadline"
            );
        }
        // Claude Code, with no budget and nothing measured, still reaches
        // FULL — the gate applies where the vendor imposes one and nowhere
        // else.
        let p = established_profile(AgentId::ClaudeCode);
        let inputs = LevelInputs {
            close_within_budget: None,
            ..full_inputs()
        };
        assert_eq!(derive_level(&p, &inputs).level, IntegrationLevel::Full);
    }

    // -------------------------------------------- T075/T076 — OpenCode ----

    /// FR-229, SC-131: the report says sessions are closed by inactivity
    /// rather than completed, in the developer's own terms.
    #[test]
    fn an_agent_with_no_termination_signal_says_how_its_sessions_end() {
        let p = established_profile(AgentId::Opencode);
        let out = derive_level(&p, &full_inputs());
        let note = out.completion_note.expect("a note is mandatory below FULL");
        assert!(note.contains("inactivity"), "{note}");
        assert!(
            note.contains("rather than completed") || note.contains("not completed"),
            "the note does not distinguish a timeout from a completion: {note}"
        );
    }

    /// FR-207, US4 #8: a future mechanism that establishes actual termination
    /// promotes OpenCode to FULL on the strength of the demonstrated
    /// capability alone. Nothing here is unlocked by an agent name or a
    /// vendor event name.
    #[test]
    fn a_demonstrated_termination_mechanism_promotes_opencode_to_full() {
        let mut p = established_profile(AgentId::Opencode);
        assert_ne!(
            derive_level(&p, &full_inputs()).level,
            IntegrationLevel::Full,
            "OpenCode is below FULL under current vendor behavior"
        );

        // The only change: some mechanism — a new vendor event, a supervisor,
        // anything — now positively establishes that a session terminated,
        // and has been observed doing it here.
        let close = p
            .capabilities
            .get_mut(&Capability::LifecycleSessionClose)
            .unwrap();
        close.availability = Availability::Guaranteed;
        close.confidence = Confidence::Verified;

        let out = derive_level(&p, &full_inputs());
        assert_eq!(
            out.level,
            IntegrationLevel::Full,
            "a demonstrated capability did not promote the agent"
        );
        assert_eq!(out.completion_guarantee, CompletionGuarantee::Demonstrated);
        assert_eq!(out.completion_note, None);
    }

    /// And the derivation contains no vendor event name to special-case.
    #[test]
    fn no_vendor_event_name_appears_in_the_derivation() {
        let source = include_str!("capability.rs");
        // The derivation is everything from the level inputs onward; the
        // profile above it records vendor facts and legitimately names the
        // hooks it depends on.
        let derivation = source
            .split("pub struct LevelInputs")
            .nth(1)
            .expect("the derivation")
            .split("#[cfg(test)]")
            .next()
            .expect("before the tests");
        for name in [
            "SessionEnd",
            "session.idle",
            "session.deleted",
            "SessionStart",
            "session.created",
            "PostToolUse",
            "tool.execute.after",
            "Stop",
            "PreCompact",
        ] {
            assert!(
                !derivation.contains(name),
                "the level derivation special-cases the vendor event `{name}`"
            );
        }
    }

    // ------------------------------------------ T117 — evidence lifecycle --

    /// SC-138: a FULL-required runtime capability that has never been observed
    /// keeps the level below FULL and is named as awaited — not as missing,
    /// which would be a different and wrong claim.
    #[test]
    fn an_unobserved_capability_is_awaited_not_missing() {
        let mut p = established_profile(AgentId::ClaudeCode);
        p.capabilities
            .get_mut(&Capability::LifecycleSessionOpen)
            .unwrap()
            .confidence = Confidence::Expected;
        let out = derive_level(&p, &full_inputs());
        assert_ne!(out.level, IntegrationLevel::Full);
        assert!(
            out.awaited_behaviors
                .iter()
                .any(|b| b.contains("first session opened")),
            "{:?}",
            out.awaited_behaviors
        );
        assert!(
            !out.missing_behaviors
                .iter()
                .any(|b| b.contains("automatic session start")),
            "a capability the agent has is not missing, only unobserved"
        );
    }

    /// SC-138: applying evidence establishes exactly the capability it names
    /// and nothing else.
    #[test]
    fn evidence_establishes_only_what_it_names() {
        let mut p = CapabilityProfile::base(AgentId::ClaudeCode);
        p.apply_evidence(&[Evidence {
            capability: "lifecycle_tool_success".into(),
            kind: EvidenceKind::Observation,
            agent_version: Some("2.1.220".into()),
            degraded: None,
        }]);
        assert!(p.get(Capability::LifecycleToolSuccess).established());
        for c in Capability::ALL {
            if c == Capability::LifecycleToolSuccess {
                continue;
            }
            assert!(
                !p.get(c).established(),
                "{c} was established by another capability's evidence"
            );
        }
    }

    /// A capability that is absent stays absent however much evidence arrives.
    /// Evidence raises confidence; it never invents availability (FR-242).
    #[test]
    fn evidence_never_creates_a_capability_the_agent_does_not_have() {
        let mut p = CapabilityProfile::base(AgentId::Opencode);
        p.apply_evidence(&[Evidence {
            capability: "lifecycle_session_close".into(),
            kind: EvidenceKind::Observation,
            agent_version: None,
            degraded: None,
        }]);
        assert_eq!(
            p.get(Capability::LifecycleSessionClose).availability,
            Availability::Absent
        );
        assert!(!p.get(Capability::LifecycleSessionClose).established());
    }
}

/// T118 — versions Cairn has not seen, and versions it knows are broken
/// (FR-185, FR-186, FR-187, FR-188, SC-123, US7 #7).
///
/// The failure this guards against is the one every integration eventually
/// makes: pinning to version strings, so that the day after a vendor releases
/// an update the tool refuses to work with an agent that is in fact fine.
/// Cairn's rule is the opposite — unknown is usable, and only a positively
/// known incompatibility is refused.
#[cfg(test)]
mod compatibility {
    use super::tests::established_profile;
    use super::*;
    use crate::model::CompatibilityClassification as C;

    #[test]
    fn an_unseen_version_is_usable_rather_than_refused() {
        // A version newer than anything Cairn has verified.
        for agent in [AgentId::ClaudeCode, AgentId::Codex, AgentId::Opencode] {
            let (class, why) = classify_version(agent, Some("99.99.99"));
            assert_eq!(
                class,
                C::CompatibleUnverified,
                "{agent:?} refused a version it merely has not seen"
            );
            assert_eq!(
                why, None,
                "there is nothing to explain about an unseen version"
            );

            // And it integrates: nothing about the classification withholds
            // the level (FR-186).
            let inputs = LevelInputs {
                mcp_present: true,
                instructions_present: true,
                skill_present: true,
                close_within_budget: Some(true),
                safe_to_integrate: class != C::Unsupported,
                ..LevelInputs::detected_and_safe()
            };
            let mut p = CapabilityProfile::base(agent);
            let evidence: Vec<Evidence> = CapabilityProfile::FULL_REQUIRED_RUNTIME
                .iter()
                .map(|c| Evidence {
                    capability: c.as_str().to_string(),
                    kind: EvidenceKind::Observation,
                    agent_version: Some("99.99.99".into()),
                    degraded: None,
                })
                .chain(p.full_required_config().into_iter().map(|k| Evidence {
                    capability: k.as_str().to_string(),
                    kind: EvidenceKind::Introspection,
                    agent_version: None,
                    degraded: None,
                }))
                .collect();
            p.apply_evidence(&evidence);
            assert_ne!(
                derive_level(&p, &inputs).level,
                IntegrationLevel::Unsupported,
                "{agent:?} at an unverified version was not integrated at all"
            );
        }
    }

    #[test]
    fn the_shipped_incompatibility_table_is_empty() {
        // The honest steady state. An entry here is a claim that a specific
        // version is broken, and Cairn makes none.
        assert!(
            KNOWN_INCOMPATIBLE.is_empty(),
            "an incompatibility claim was added without a recorded reason"
        );
    }

    #[test]
    fn only_a_known_incompatibility_is_unsupported_and_it_says_what() {
        let table = [(
            AgentId::Codex,
            "0.40.0",
            "hook payloads before 0.41 carry no session identifier, so events cannot be routed",
        )];
        let (class, why) = classify_against(&table, AgentId::Codex, Some("0.40.0"));
        assert_eq!(class, C::Unsupported);
        let why = why.expect("an unsupported version states what is incompatible");
        assert!(why.contains("session identifier"), "{why}");

        // The neighbours are unaffected: one bad version is not a floor.
        assert_eq!(
            classify_against(&table, AgentId::Codex, Some("0.39.0")).0,
            C::CompatibleUnverified
        );
        assert_eq!(
            classify_against(&table, AgentId::Codex, Some("0.41.0")).0,
            C::CompatibleUnverified
        );
        // And it is per agent, not per version string.
        assert_eq!(
            classify_against(&table, AgentId::ClaudeCode, Some("0.40.0")).0,
            C::CompatibleUnverified
        );
    }

    #[test]
    fn a_removed_vendor_event_lowers_the_level_and_keeps_working() {
        // US7 #7: degradation is by capability detection, not by version
        // matching. The agent updates, an event Cairn relied on disappears,
        // its observation evidence is discarded with the version change, and
        // the integration keeps working at a level that says so.
        let mut p = established_profile(AgentId::ClaudeCode);
        let full = LevelInputs {
            close_within_budget: Some(true),
            ..LevelInputs {
                mcp_present: true,
                instructions_present: true,
                skill_present: true,
                ..LevelInputs::detected_and_safe()
            }
        };
        assert_eq!(derive_level(&p, &full).level, IntegrationLevel::Full);

        // The vendor removed the event behind this capability.
        let state = p
            .capabilities
            .get_mut(&Capability::LifecycleQuiesce)
            .unwrap();
        state.availability = Availability::Absent;

        let out = derive_level(&p, &full);
        assert_eq!(
            out.level,
            IntegrationLevel::McpPlus,
            "a removed vendor event broke the integration instead of lowering it"
        );
        assert!(
            out.missing_behaviors
                .iter()
                .any(|b| b.contains("turn checkpoints")),
            "the lost behavior was not named: {:?}",
            out.missing_behaviors
        );
        // The version string never entered into it.
        assert_eq!(
            classify_version(AgentId::ClaudeCode, Some("2.1.220")).0,
            C::Verified,
            "the version is still the one Cairn verified; only the capability changed"
        );
    }

    #[test]
    fn a_version_change_discards_observation_evidence_and_keeps_introspection() {
        // FR-245, SC-138: the store applies this rule, and this is what it
        // means for the level. Introspection is a fact about Cairn's own
        // artifact; observation is a fact about a build that is now gone.
        let mut p = CapabilityProfile::base(AgentId::ClaudeCode);
        let config: Vec<Evidence> = p
            .full_required_config()
            .into_iter()
            .map(|k| Evidence {
                capability: k.as_str().to_string(),
                kind: EvidenceKind::Introspection,
                agent_version: Some("2.1.220".into()),
                degraded: None,
            })
            .collect();
        p.apply_evidence(&config);

        // Runtime evidence from the previous version is simply not applied:
        // the store discarded it, so the profile never sees it.
        let inputs = LevelInputs {
            mcp_present: true,
            instructions_present: true,
            skill_present: true,
            close_within_budget: Some(true),
            ..LevelInputs::detected_and_safe()
        };
        let out = derive_level(&p, &inputs);
        assert_ne!(out.level, IntegrationLevel::Full);
        assert!(
            !out.awaited_behaviors.is_empty(),
            "the level dropped without saying what it is waiting for"
        );
        // The configuration half survived, so nothing is reinstalled.
        for kind in p.full_required_config() {
            assert_eq!(p.config_verified.get(&kind), Some(&true));
        }
        assert!(
            !out.awaited_behaviors.iter().any(|b| b.contains("resource")),
            "introspection evidence was discarded along with observation: {:?}",
            out.awaited_behaviors
        );
    }
}

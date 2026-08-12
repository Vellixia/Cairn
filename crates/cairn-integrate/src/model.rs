//! The enumerations the integration layer is built from (`data-model.md`).
//!
//! These are data, not behavior. Every one of them is a closed set: a health
//! condition is never free text (FR-167), an owner is exactly one of three
//! things (FR-145), and a level is computed rather than stored as intent
//! (FR-109).

use serde::{Deserialize, Serialize};
use std::fmt;

/// A coding agent Cairn has a native adapter for, plus the generic fallback.
///
/// Feature 002 adds no agent beyond these, including applications a detected
/// integration manager happens to support (FR-102, FR-106).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    ClaudeCode,
    Codex,
    Opencode,
    GenericMcp,
}

impl AgentId {
    pub const ALL: [AgentId; 4] = [
        AgentId::ClaudeCode,
        AgentId::Codex,
        AgentId::Opencode,
        AgentId::GenericMcp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "claude-code",
            AgentId::Codex => "codex",
            AgentId::Opencode => "opencode",
            AgentId::GenericMcp => "generic-mcp",
        }
    }

    pub fn parse(s: &str) -> Option<AgentId> {
        AgentId::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// Whether this agent supports Skills at all. The generic MCP path does
    /// not, which is why FULL's Skill requirement is conditional on it
    /// (`data-model.md` §Level derivation).
    pub fn supports_skills(self) -> bool {
        !matches!(self, AgentId::GenericMcp)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An integration manager. Exactly one in this feature (FR-103).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManagerId {
    CcSwitch,
}

impl ManagerId {
    pub fn as_str(self) -> &'static str {
        match self {
            ManagerId::CcSwitch => "cc-switch",
        }
    }
    pub fn parse(s: &str) -> Option<ManagerId> {
        (s == "cc-switch").then_some(ManagerId::CcSwitch)
    }
}

impl fmt::Display for ManagerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The four things Cairn installs per agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Mcp,
    Lifecycle,
    Instructions,
    Skill,
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 4] = [
        ResourceKind::Mcp,
        ResourceKind::Lifecycle,
        ResourceKind::Instructions,
        ResourceKind::Skill,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Mcp => "mcp",
            ResourceKind::Lifecycle => "lifecycle",
            ResourceKind::Instructions => "instructions",
            ResourceKind::Skill => "skill",
        }
    }

    pub fn parse(s: &str) -> Option<ResourceKind> {
        ResourceKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Only these two are manager-distributable in this feature (D33).
    pub fn manager_distributable(self) -> bool {
        matches!(self, ResourceKind::Mcp | ResourceKind::Skill)
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Exactly one recorded owner per logical resource (FR-145).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOwner {
    /// Cairn installed it and Cairn maintains it.
    Direct,
    /// A manager distributes it; Cairn verifies but never removes it (FR-149).
    Manager,
    /// It exists, Cairn did not install it, Cairn does not manage it. Never
    /// adopted, never deleted (FR-150).
    External,
}

impl ResourceOwner {
    pub fn as_str(self) -> &'static str {
        match self {
            ResourceOwner::Direct => "direct",
            ResourceOwner::Manager => "manager",
            ResourceOwner::External => "external",
        }
    }
    pub fn parse(s: &str) -> Option<ResourceOwner> {
        match s {
            "direct" => Some(ResourceOwner::Direct),
            "manager" => Some(ResourceOwner::Manager),
            "external" => Some(ResourceOwner::External),
            // The CLI names the manager rather than the category.
            "cc-switch" => Some(ResourceOwner::Manager),
            _ => None,
        }
    }
}

impl fmt::Display for ResourceOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a Cairn-owned resource lives, recorded alongside its owner and never
/// inferred from where a file happens to be found (FR-220).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationScope {
    /// Inside the repository, in a file normally committed.
    ProjectShared,
    /// Inside the repository, in a file the agent treats as developer-local.
    ProjectLocal,
    /// Per-user, outside any repository.
    User,
}

impl InstallationScope {
    pub fn as_str(self) -> &'static str {
        match self {
            InstallationScope::ProjectShared => "project_shared",
            InstallationScope::ProjectLocal => "project_local",
            InstallationScope::User => "user",
        }
    }
    pub fn parse(s: &str) -> Option<InstallationScope> {
        match s {
            "project_shared" => Some(InstallationScope::ProjectShared),
            "project_local" => Some(InstallationScope::ProjectLocal),
            "user" => Some(InstallationScope::User),
            _ => None,
        }
    }
    /// True where writing here adds a file a collaborator would receive.
    pub fn is_committed(self) -> bool {
        self == InstallationScope::ProjectShared
    }
}

impl fmt::Display for InstallationScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a trust-gated handler is actually going to run (FR-209, D24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationState {
    #[default]
    NotApplicable,
    PendingUserTrust,
    Active,
    /// The handler was trusted and Cairn's upgrade reset that trust.
    Invalidated,
}

impl ActivationState {
    pub fn as_str(self) -> &'static str {
        match self {
            ActivationState::NotApplicable => "not_applicable",
            ActivationState::PendingUserTrust => "pending_user_trust",
            ActivationState::Active => "active",
            ActivationState::Invalidated => "invalidated",
        }
    }
    pub fn parse(s: &str) -> Option<ActivationState> {
        match s {
            "not_applicable" => Some(ActivationState::NotApplicable),
            "pending_user_trust" => Some(ActivationState::PendingUserTrust),
            "active" => Some(ActivationState::Active),
            "invalidated" => Some(ActivationState::Invalidated),
            _ => None,
        }
    }
    /// True where the handler will not run until the user acts.
    pub fn needs_user_action(self) -> bool {
        matches!(
            self,
            ActivationState::PendingUserTrust | ActivationState::Invalidated
        )
    }
}

/// The honest summary of a connected agent. Computed, never stored as intent
/// (FR-109).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationLevel {
    Unsupported,
    McpOnly,
    McpPlus,
    Full,
}

impl IntegrationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            IntegrationLevel::Full => "full",
            IntegrationLevel::McpPlus => "mcp_plus",
            IntegrationLevel::McpOnly => "mcp_only",
            IntegrationLevel::Unsupported => "unsupported",
        }
    }
    pub fn parse(s: &str) -> Option<IntegrationLevel> {
        match s {
            "full" => Some(IntegrationLevel::Full),
            "mcp_plus" => Some(IntegrationLevel::McpPlus),
            "mcp_only" => Some(IntegrationLevel::McpOnly),
            "unsupported" => Some(IntegrationLevel::Unsupported),
            _ => None,
        }
    }
    /// The display form. Never printed as a bare word where a capability is
    /// missing — the caller appends the naming clause (FR-110, FR-111).
    pub fn display(self) -> &'static str {
        match self {
            IntegrationLevel::Full => "FULL",
            IntegrationLevel::McpPlus => "MCP_PLUS",
            IntegrationLevel::McpOnly => "MCP_ONLY",
            IntegrationLevel::Unsupported => "UNSUPPORTED",
        }
    }
}

impl fmt::Display for IntegrationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display())
    }
}

/// Per agent, how much Cairn actually knows about this version (FR-185).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityClassification {
    Verified,
    /// The default for any version Cairn has not pinned as broken (FR-186).
    CompatibleUnverified,
    Unsupported,
}

impl CompatibilityClassification {
    pub fn as_str(self) -> &'static str {
        match self {
            CompatibilityClassification::Verified => "verified",
            CompatibilityClassification::CompatibleUnverified => "compatible_unverified",
            CompatibilityClassification::Unsupported => "unsupported",
        }
    }
    pub fn parse(s: &str) -> Option<CompatibilityClassification> {
        match s {
            "verified" => Some(CompatibilityClassification::Verified),
            "compatible_unverified" => Some(CompatibilityClassification::CompatibleUnverified),
            "unsupported" => Some(CompatibilityClassification::Unsupported),
            _ => None,
        }
    }
}

/// The closed set of per-resource conditions diagnostics may report (FR-167).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCondition {
    Healthy,
    Missing,
    Modified,
    Outdated,
    Duplicated,
    ConflictingOwner,
    MalformedConfig,
    DamagedMarkers,
    InstalledNotActivated,
    Migrating,
    ManagerActionRequired,
    /// Healthy, and serving more than one agent (FR-243).
    Shared,
    Unknown,
}

impl HealthCondition {
    pub fn as_str(self) -> &'static str {
        match self {
            HealthCondition::Healthy => "healthy",
            HealthCondition::Missing => "missing",
            HealthCondition::Modified => "modified",
            HealthCondition::Outdated => "outdated",
            HealthCondition::Duplicated => "duplicated",
            HealthCondition::ConflictingOwner => "conflicting_owner",
            HealthCondition::MalformedConfig => "malformed_config",
            HealthCondition::DamagedMarkers => "damaged_markers",
            HealthCondition::InstalledNotActivated => "installed_not_activated",
            HealthCondition::Migrating => "migrating",
            HealthCondition::ManagerActionRequired => "manager_action_required",
            HealthCondition::Shared => "shared",
            HealthCondition::Unknown => "unknown",
        }
    }

    /// Doctor exits 0 only where every condition is one of these (FR-170).
    pub fn is_acceptable(self) -> bool {
        matches!(
            self,
            HealthCondition::Healthy | HealthCondition::Shared | HealthCondition::Unknown
        )
    }

    /// Whether a default repair may act on this condition, or must only report
    /// it (`contracts/integration-health.md` §Condition semantics).
    pub fn repairable_by_default(self) -> bool {
        matches!(
            self,
            HealthCondition::Missing | HealthCondition::Outdated | HealthCondition::Duplicated
        )
    }

    /// Whether `--force` may act on it. A damaged marker never qualifies:
    /// forcing past one would mean guessing which text was Cairn's (FR-221).
    pub fn repairable_by_force(self) -> bool {
        self.repairable_by_default() || self == HealthCondition::Modified
    }
}

impl fmt::Display for HealthCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Schema plus content revision for a versioned artifact (D26).
///
/// Deliberately not tied to Cairn's package version: a patch release that
/// changes neither the contract nor the Skill rewrites nothing, which on Codex
/// also means it does not invalidate hook trust (D24).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub schema: u32,
    pub revision: String,
}

impl ArtifactVersion {
    pub fn new(schema: u32, revision: impl Into<String>) -> Self {
        Self {
            schema,
            revision: revision.into(),
        }
    }
}

impl fmt::Display for ArtifactVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "schema {} revision {}", self.schema, self.revision)
    }
}

/// The canonical hash used for ownership and semantic comparison.
///
/// First 12 hex characters of the SHA-256 of a deterministic normalization:
/// trailing whitespace stripped per line, exactly one trailing newline
/// (`data-model.md` §Conventions, D25).
pub fn canonical_hash(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(normalize_text(text).as_bytes());
    hex::encode(h.finalize())[..12].to_string()
}

/// The normalization `canonical_hash` applies. Exposed because semantic
/// comparison needs the same rule (FR-223).
pub fn normalize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.replace("\r\n", "\n").lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    if out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_enum_round_trips_through_its_string_form() {
        for a in AgentId::ALL {
            assert_eq!(AgentId::parse(a.as_str()), Some(a));
        }
        for k in ResourceKind::ALL {
            assert_eq!(ResourceKind::parse(k.as_str()), Some(k));
        }
        for o in [
            ResourceOwner::Direct,
            ResourceOwner::Manager,
            ResourceOwner::External,
        ] {
            assert_eq!(ResourceOwner::parse(o.as_str()), Some(o));
        }
        for s in [
            InstallationScope::ProjectShared,
            InstallationScope::ProjectLocal,
            InstallationScope::User,
        ] {
            assert_eq!(InstallationScope::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn formatting_only_differences_hash_alike() {
        // FR-223: reflow and trailing whitespace are not an edit.
        assert_eq!(canonical_hash("a\nb\n"), canonical_hash("a  \r\nb\n\n\n"));
        assert_ne!(canonical_hash("a\nb\n"), canonical_hash("a\nc\n"));
    }

    #[test]
    fn a_damaged_marker_is_never_forceable() {
        // FR-221: forcing past one would mean guessing which text was Cairn's.
        assert!(!HealthCondition::DamagedMarkers.repairable_by_force());
        assert!(!HealthCondition::ConflictingOwner.repairable_by_force());
        assert!(!HealthCondition::MalformedConfig.repairable_by_force());
        assert!(HealthCondition::Modified.repairable_by_force());
        assert!(!HealthCondition::Modified.repairable_by_default());
    }

    #[test]
    fn only_mcp_and_skill_are_manager_distributable() {
        assert!(ResourceKind::Mcp.manager_distributable());
        assert!(ResourceKind::Skill.manager_distributable());
        assert!(!ResourceKind::Lifecycle.manager_distributable());
        assert!(!ResourceKind::Instructions.manager_distributable());
    }
}

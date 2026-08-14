//! The adapter boundary (FR-101, FR-102, plan.md §Adapter shape, D19).
//!
//! Five operations, four implementors. Atomic writing, marker handling,
//! canonical hashing, change classification, semantic comparison and
//! verification are shared code the adapters call — so a bug in atomicity is
//! fixed once, not four times.
//!
//! `normalize` returning `None` is the *normal* way an adapter declines an
//! event it does not map (FR-115). An adapter never emits a canonical event
//! the agent has not actually signalled: where an agent provides no signal,
//! that event simply does not occur for that agent, the gap shows up in the
//! capability profile, and Cairn's existing deterministic boundaries govern
//! the outcome.
//!
//! The manager is a different type with a different shape, because it has no
//! lifecycle, no instructions and no removal (FR-101).

use crate::capability::CapabilityProfile;
use crate::model::{
    ActivationState, AgentId, HealthCondition, InstallationScope, ManagerId, ResourceKind,
    ResourceOwner,
};
use crate::scope::Env;
use cairn_core::lifecycle::CanonicalLifecycleEvent;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What detection found. Never modifies anything and never needs the network
/// (FR-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detection {
    pub detected: bool,
    /// The agent's own version string where it exposes one obtainable without
    /// authentication (FR-104).
    pub version: Option<String>,
    /// Where the evidence of installation was found, for diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<String>,
}

impl Detection {
    pub fn absent() -> Detection {
        Detection {
            detected: false,
            version: None,
            evidence_path: None,
        }
    }
    pub fn found(version: Option<String>, path: Option<PathBuf>) -> Detection {
        Detection {
            detected: true,
            version,
            evidence_path: path.map(|p| p.display().to_string()),
        }
    }
}

/// One resource as it actually exists on this machine, from inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub kind: ResourceKind,
    pub condition: HealthCondition,
    pub owner: ResourceOwner,
    pub scope: InstallationScope,
    pub location: Option<PathBuf>,
    pub activation: ActivationState,
    /// The artifact version found installed, where the resource carries one.
    pub installed_schema: Option<u32>,
    pub installed_revision: Option<String>,
    /// What names the problem. Never quotes user configuration beyond what
    /// identifies it, and never contains a credential (FR-171).
    pub detail: Option<String>,
    /// A command the developer can run, or the manual sequence.
    pub remedy: Option<String>,
}

impl Observed {
    pub fn new(kind: ResourceKind, condition: HealthCondition) -> Observed {
        Observed {
            kind,
            condition,
            owner: ResourceOwner::Direct,
            scope: InstallationScope::User,
            location: None,
            activation: ActivationState::NotApplicable,
            installed_schema: None,
            installed_revision: None,
            detail: None,
            remedy: None,
        }
    }
    pub fn at(mut self, scope: InstallationScope, location: Option<PathBuf>) -> Observed {
        self.scope = scope;
        self.location = location;
        self
    }
    pub fn owned_by(mut self, owner: ResourceOwner) -> Observed {
        self.owner = owner;
        self
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Observed {
        self.detail = Some(detail.into());
        self
    }
    pub fn remedy(mut self, remedy: impl Into<String>) -> Observed {
        self.remedy = Some(remedy.into());
        self
    }
    pub fn version(mut self, schema: u32, revision: impl Into<String>) -> Observed {
        self.installed_schema = Some(schema);
        self.installed_revision = Some(revision.into());
        self
    }
}

/// A raw vendor payload, before normalization.
///
/// Deliberately opaque: the shape belongs to the vendor, and nothing outside
/// the adapter may depend on it (FR-112).
#[derive(Debug, Clone, Default)]
pub struct RawPayload {
    pub json: serde_json::Value,
    pub cwd: String,
}

impl RawPayload {
    pub fn new(json: serde_json::Value, cwd: impl Into<String>) -> RawPayload {
        RawPayload {
            json,
            cwd: cwd.into(),
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        self.json.get(key).and_then(|v| v.as_str())
    }
    pub fn value(&self, key: &str) -> Option<&serde_json::Value> {
        self.json.get(key)
    }
}

/// The translation boundary for one coding agent.
///
/// Owns that agent's configuration surfaces and lifecycle vocabulary. Owns no
/// Cairn semantics.
pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;

    /// Is it installed, and at what version?
    fn detect(&self, env: &Env) -> Detection;

    /// Static vendor facts, refined by detection (FR-107, FR-108).
    fn capabilities(&self, detection: &Detection) -> CapabilityProfile {
        let _ = detection;
        CapabilityProfile::base(self.id())
    }

    /// What exists on this machine right now.
    fn inspect(&self, env: &Env, record: &[crate::plan::RecordedInstall]) -> Vec<Observed>;

    /// Translate one vendor event. `None` means the adapter declines it, which
    /// is the normal case for every event Cairn does not map (FR-115).
    fn normalize(&self, event: &str, payload: &RawPayload) -> Option<CanonicalLifecycleEvent>;

    /// The vendor events this adapter registers. Cairn registers only what its
    /// canonical lifecycle needs and reports the rest as unused (US2 #6).
    fn registered_events(&self) -> &'static [&'static str];
}

/// The structured outcome when a manager step cannot be automated (FR-233).
///
/// The operation that returns this **has not completed**. Cairn never reports
/// success on the strength of having asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerActionRequired {
    pub manager: ManagerId,
    pub resource_kind: ResourceKind,
    pub applications: Vec<String>,
    /// `import` | `remove`.
    pub action: String,
    /// `deep_link` | `manual_ui`.
    pub method: String,
    /// Present only for `import`, and only when it carries no secret. Removal
    /// has no documented link, so it is null — never a fabricated one.
    pub uri: Option<String>,
    pub instructions: String,
    pub verify_with: String,
    /// `awaiting_user` | `verified` | `not_performed`.
    pub status: String,
}

/// One manager-held resource and the applications it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerBinding {
    pub kind: ResourceKind,
    pub app: String,
    pub condition: HealthCondition,
    pub detail: Option<String>,
    pub remedy: Option<String>,
}

/// A tool that distributes Cairn resources to agents on the developer's
/// behalf. Never an agent: it produces no sessions, no observations and no
/// lifecycle events (FR-101).
pub trait IntegrationManager: Send + Sync {
    fn id(&self) -> ManagerId;

    fn detect(&self, env: &Env) -> Detection;

    /// Which applications this manager can distribute to.
    fn target_apps(&self) -> &'static [&'static str];

    /// Which Cairn resources it can distribute.
    fn distributable(&self) -> &'static [ResourceKind];

    /// Inspect the target applications' **own** configuration to see what the
    /// manager actually installed (FR-234).
    fn inspect_bindings(
        &self,
        env: &Env,
        apps: &[String],
        kind: ResourceKind,
    ) -> Vec<ManagerBinding>;

    /// Build the manager's documented import request.
    fn import_uri(&self, kind: ResourceKind, apps: &[String]) -> Result<String, ImportRefusal>;
}

/// Why an import request cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportRefusal {
    /// The build's embedded Skill revision has no published branch, and
    /// emitting an unpublished ref would make the manager silently install
    /// `main` (D29).
    UnpublishedSkillRef { revision: String, manual: String },
    /// This manager does not distribute this resource kind.
    NotDistributable(ResourceKind),
}

impl std::fmt::Display for ImportRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportRefusal::UnpublishedSkillRef { revision, .. } => write!(
                f,
                "this build's Skill revision {revision} has no published skill-release branch"
            ),
            ImportRefusal::NotDistributable(k) => {
                write!(f, "{k} is not distributable through this manager")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_defaults_to_absent_rather_than_assumed() {
        let d = Detection::absent();
        assert!(!d.detected);
        assert!(d.version.is_none());
    }

    #[test]
    fn a_manager_action_carries_no_fabricated_uri_for_removal() {
        // FR-233: removal has no documented link, so it is null.
        let a = ManagerActionRequired {
            manager: ManagerId::CcSwitch,
            resource_kind: ResourceKind::Mcp,
            applications: vec!["codex".into()],
            action: "remove".into(),
            method: "manual_ui".into(),
            uri: None,
            instructions: "In CC Switch: …".into(),
            verify_with: "cairn doctor codex".into(),
            status: "awaiting_user".into(),
        };
        assert!(a.uri.is_none());
        assert_eq!(a.status, "awaiting_user");
    }
}

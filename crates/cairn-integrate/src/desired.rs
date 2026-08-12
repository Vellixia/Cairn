//! The single canonical statement of intent (FR-201, FR-202, FR-226, D26).
//!
//! Guided onboarding, preview, diagnostics, repair, ownership migration and
//! manager distribution all read from this one model rather than each deriving
//! its own view of intent. That is the whole point: two views of "what the
//! developer wants" is how an integration layer starts contradicting itself.
//!
//! Three invariants are structural rather than checked:
//!
//! - **Path-free.** No field can hold a machine-specific absolute path.
//!   Locations are named by scope and resolved from the scope matrix at apply
//!   time.
//! - **Secret-free.** No field can hold a token, credential or key.
//! - **Deterministic.** Keys are emitted in a fixed order and lists are sorted
//!   by a stable key, so identical inputs serialize byte-identically on any
//!   machine (SC-135).
//!
//! Feature 002 writes no project-local desired-state file and treats no
//! committed file as authoritative for integration intent (FR-202). Manifest
//! drift, merge semantics and application-on-clone are explicitly out of scope
//! (FR-227) — this model exists so a later feature can add them without
//! changing adapter or reconciliation semantics.

use crate::model::{
    ActivationState, AgentId, ArtifactVersion, InstallationScope, IntegrationLevel, ManagerId,
    ResourceKind, ResourceOwner,
};
use serde::{Deserialize, Serialize};

/// The desired-state schema version (D26).
pub const DESIRED_STATE_SCHEMA: u32 = 1;

/// What the developer wants, for one resource of one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredResource {
    pub kind: ResourceKind,
    /// Exactly one owner (FR-145).
    pub owner: ResourceOwner,
    pub scope: InstallationScope,
    /// Schema plus revision for versioned artifacts; null for the MCP entry
    /// and lifecycle handlers, which carry `adapter_version`.
    pub desired_version: Option<ArtifactVersion>,
    /// States the intent that a trust-gated handler should end up running
    /// (FR-209).
    pub desired_activation: ActivationState,
}

/// What the developer wants for one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredAgent {
    pub agent: AgentId,
    /// False means "disconnect this agent".
    pub enabled: bool,
    /// What the developer asked for. The achieved level is computed and never
    /// taken from here (FR-109).
    pub requested_level: Option<IntegrationLevel>,
    pub resources: Vec<DesiredResource>,
}

/// What the developer wants from an integration manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredManager {
    pub manager: ManagerId,
    /// The manager's own application identifiers.
    pub target_apps: Vec<String>,
    /// Only `mcp` and `skill` are manager-distributable in this feature.
    pub resources: Vec<ResourceKind>,
}

/// The single canonical statement of intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredIntegrationState {
    pub schema: u32,
    pub agents: Vec<DesiredAgent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager: Option<DesiredManager>,
}

/// What the developer asked for on the command line, before it is composed
/// with the local record and detection.
#[derive(Debug, Clone, Default)]
pub struct Choices {
    /// Empty means "everything detected" (`--auto`).
    pub agents: Vec<AgentId>,
    pub shared: bool,
    /// Per-resource scope overrides, e.g. `--scope mcp=project_shared`.
    pub scope_overrides: Vec<(ResourceKind, InstallationScope)>,
    /// Distribute these resource kinds through a manager instead of directly.
    pub via_manager: Option<ManagerId>,
    pub manager_resources: Vec<ResourceKind>,
    pub manager_apps: Vec<String>,
    /// Agents being disconnected rather than connected.
    pub disable: Vec<AgentId>,
    /// Restrict the operation to these resource kinds (`--only`).
    pub only: Vec<ResourceKind>,
}

/// What the machine already holds, for one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedResource {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub owner: ResourceOwner,
    pub scope: InstallationScope,
    pub activation: ActivationState,
}

impl DesiredIntegrationState {
    /// Compose intent from the developer's explicit choices, Cairn's local
    /// integration record, and the configuration actually detected on the
    /// machine (FR-202).
    ///
    /// The record is what keeps Feature 001's resources where they already
    /// are: an adopted project-scoped resource stays project-scoped, and
    /// moving one is an explicit migration rather than a silent relocation
    /// (FR-217).
    pub fn compose(
        choices: &Choices,
        detected: &[AgentId],
        recorded: &[RecordedResource],
        contract_version: ArtifactVersion,
        skill_version: ArtifactVersion,
    ) -> DesiredIntegrationState {
        let mut selected: Vec<AgentId> = if choices.agents.is_empty() {
            detected.to_vec()
        } else {
            choices.agents.clone()
        };
        for a in &choices.disable {
            if !selected.contains(a) {
                selected.push(*a);
            }
        }
        selected.sort();
        selected.dedup();

        let mut agents: Vec<DesiredAgent> = Vec::new();
        for agent in selected {
            let mut resources: Vec<DesiredResource> = Vec::new();
            for kind in crate::scope::kinds_for(agent) {
                if !choices.only.is_empty() && !choices.only.contains(&kind) {
                    continue;
                }
                let recorded_here = recorded.iter().find(|r| r.agent == agent && r.kind == kind);

                let owner = match (&choices.via_manager, recorded_here) {
                    // A manager the developer chose owns what it can
                    // distribute; lifecycle and instructions stay direct
                    // (FR-165).
                    (Some(_), _) if choices.manager_resources.contains(&kind) => {
                        ResourceOwner::Manager
                    }
                    (_, Some(r)) => r.owner,
                    _ => ResourceOwner::Direct,
                };

                let override_scope = choices
                    .scope_overrides
                    .iter()
                    .find(|(k, _)| *k == kind)
                    .map(|(_, s)| *s);
                // An already-recorded resource keeps its actual scope unless
                // the developer explicitly asks otherwise (FR-217).
                let scope = match (override_scope, recorded_here) {
                    (Some(s), _) => s,
                    (None, Some(r)) => r.scope,
                    (None, None) => crate::scope::resolve_scope(agent, kind, choices.shared, None)
                        .unwrap_or(InstallationScope::User),
                };

                let desired_version = match kind {
                    ResourceKind::Instructions => Some(contract_version.clone()),
                    ResourceKind::Skill => Some(skill_version.clone()),
                    _ => None,
                };
                let desired_activation = match (kind, agent) {
                    (ResourceKind::Lifecycle, AgentId::Codex) => ActivationState::Active,
                    _ => ActivationState::NotApplicable,
                };

                resources.push(DesiredResource {
                    kind,
                    owner,
                    scope,
                    desired_version,
                    desired_activation,
                });
            }
            resources.sort_by_key(|r| r.kind);
            agents.push(DesiredAgent {
                agent,
                enabled: !choices.disable.contains(&agent),
                requested_level: None,
                resources,
            });
        }
        agents.sort_by_key(|a| a.agent);

        let manager = choices.via_manager.map(|manager| {
            let mut target_apps = if choices.manager_apps.is_empty() {
                agents
                    .iter()
                    .filter_map(|a| crate::scope::manager_app_for(a.agent))
                    .map(str::to_string)
                    .collect()
            } else {
                choices.manager_apps.clone()
            };
            target_apps.sort();
            target_apps.dedup();
            let mut resources: Vec<ResourceKind> = choices
                .manager_resources
                .iter()
                .copied()
                .filter(|k| k.manager_distributable())
                .collect();
            resources.sort();
            resources.dedup();
            DesiredManager {
                manager,
                target_apps,
                resources,
            }
        });

        DesiredIntegrationState {
            schema: DESIRED_STATE_SCHEMA,
            agents,
            manager,
        }
    }

    /// Deterministic serialization. Identical inputs produce byte-identical
    /// output across runs and machines (SC-135).
    pub fn serialize(&self) -> String {
        // `serde_json::to_string_pretty` emits struct fields in declaration
        // order and never reorders; the sorting above is what makes the lists
        // stable, and no map is ever emitted.
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Every `(agent, kind)` appears exactly once.
    pub fn well_formed(&self) -> bool {
        let mut seen = std::collections::BTreeSet::new();
        for a in &self.agents {
            for r in &a.resources {
                if !seen.insert((a.agent, r.kind)) {
                    return false;
                }
            }
        }
        true
    }

    /// Look up one resource's intent.
    pub fn resource(&self, agent: AgentId, kind: ResourceKind) -> Option<&DesiredResource> {
        self.agents
            .iter()
            .find(|a| a.agent == agent)?
            .resources
            .iter()
            .find(|r| r.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions() -> (ArtifactVersion, ArtifactVersion) {
        (
            ArtifactVersion::new(1, "8f2b19c40a7d"),
            ArtifactVersion::new(1, "c07d4419b2ae"),
        )
    }

    fn compose(choices: &Choices, recorded: &[RecordedResource]) -> DesiredIntegrationState {
        let (c, s) = versions();
        DesiredIntegrationState::compose(
            choices,
            &[AgentId::ClaudeCode, AgentId::Codex],
            recorded,
            c,
            s,
        )
    }

    #[test]
    fn determinism() {
        // SC-135: identical inputs serialize byte-identically.
        let choices = Choices {
            agents: vec![AgentId::Codex, AgentId::ClaudeCode],
            ..Default::default()
        };
        let a = compose(&choices, &[]).serialize();
        // Reversing the developer's input order must not change the output.
        let reversed = Choices {
            agents: vec![AgentId::ClaudeCode, AgentId::Codex],
            ..Default::default()
        };
        let b = compose(&reversed, &[]).serialize();
        assert_eq!(a, b);
        assert_eq!(a, compose(&choices, &[]).serialize());
    }

    #[test]
    fn single_consumer() {
        // Every (agent, kind) appears exactly once (FR-201).
        let s = compose(&Choices::default(), &[]);
        assert!(s.well_formed());
        assert_eq!(s.schema, DESIRED_STATE_SCHEMA);
    }

    #[test]
    fn no_secret_and_no_absolute_path_can_appear() {
        // Secret-free and path-free by construction: there is no field that
        // could hold one, which is stronger than filtering.
        let s = compose(&Choices::default(), &[]).serialize();
        assert!(!s.contains('/'), "a path leaked into desired state: {s}");
        for word in ["token", "secret", "key", "password", "authorization"] {
            assert!(!s.to_lowercase().contains(word), "{word} appears in {s}");
        }
    }

    #[test]
    fn a_recorded_scope_is_kept_rather_than_relocated() {
        // FR-217: Feature 001 wrote lifecycle to committed project scope;
        // Feature 002's default is project_local, and adopting must not move
        // it.
        let recorded = [RecordedResource {
            agent: AgentId::ClaudeCode,
            kind: ResourceKind::Lifecycle,
            owner: ResourceOwner::Direct,
            scope: InstallationScope::ProjectShared,
            activation: ActivationState::NotApplicable,
        }];
        let s = compose(&Choices::default(), &recorded);
        assert_eq!(
            s.resource(AgentId::ClaudeCode, ResourceKind::Lifecycle)
                .unwrap()
                .scope,
            InstallationScope::ProjectShared
        );
    }

    #[test]
    fn an_explicit_override_still_wins() {
        let choices = Choices {
            scope_overrides: vec![(ResourceKind::Mcp, InstallationScope::ProjectShared)],
            ..Default::default()
        };
        let s = compose(&choices, &[]);
        assert_eq!(
            s.resource(AgentId::Codex, ResourceKind::Mcp).unwrap().scope,
            InstallationScope::ProjectShared
        );
    }

    #[test]
    fn a_manager_owns_only_what_it_can_distribute() {
        // FR-165: the split is shown, with lifecycle and instructions direct.
        let choices = Choices {
            via_manager: Some(ManagerId::CcSwitch),
            manager_resources: vec![
                ResourceKind::Mcp,
                ResourceKind::Skill,
                ResourceKind::Lifecycle,
            ],
            ..Default::default()
        };
        let s = compose(&choices, &[]);
        let m = s.manager.as_ref().unwrap();
        assert_eq!(m.resources, vec![ResourceKind::Mcp, ResourceKind::Skill]);
        assert_eq!(
            s.resource(AgentId::Codex, ResourceKind::Mcp).unwrap().owner,
            ResourceOwner::Manager
        );
        assert_eq!(
            s.resource(AgentId::Codex, ResourceKind::Instructions)
                .unwrap()
                .owner,
            ResourceOwner::Direct
        );
    }

    #[test]
    fn disabling_an_agent_is_expressed_in_the_same_model() {
        let choices = Choices {
            disable: vec![AgentId::Codex],
            ..Default::default()
        };
        let s = compose(&choices, &[]);
        assert!(
            !s.agents
                .iter()
                .find(|a| a.agent == AgentId::Codex)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn only_restricts_the_resource_kinds_considered() {
        let choices = Choices {
            only: vec![ResourceKind::Mcp],
            ..Default::default()
        };
        let s = compose(&choices, &[]);
        for a in &s.agents {
            assert_eq!(a.resources.len(), 1);
            assert_eq!(a.resources[0].kind, ResourceKind::Mcp);
        }
    }

    #[test]
    fn codex_lifecycle_states_the_intent_to_be_active() {
        // FR-209: the desired activation is a statement of intent, not a
        // claim that it is running.
        let s = compose(&Choices::default(), &[]);
        assert_eq!(
            s.resource(AgentId::Codex, ResourceKind::Lifecycle)
                .unwrap()
                .desired_activation,
            ActivationState::Active
        );
        assert_eq!(
            s.resource(AgentId::ClaudeCode, ResourceKind::Mcp)
                .unwrap()
                .desired_activation,
            ActivationState::NotApplicable
        );
    }

    #[test]
    fn the_achieved_level_is_never_taken_from_intent() {
        let s = compose(&Choices::default(), &[]);
        assert!(s.agents.iter().all(|a| a.requested_level.is_none()));
    }
}

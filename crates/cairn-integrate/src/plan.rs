//! Setup's integration change plan.
//!
//! Computing a plan performs **zero writes**, including no temporary files
//! (SC-118).

use crate::adapter::Observed;
use crate::desired::DesiredIntegrationState;
use crate::model::{
    ActivationState, AgentId, HealthCondition, InstallationScope, ResourceKind, ResourceOwner,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What the local record holds about one physical resource, plus who depends
/// on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedInstall {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub owner: ResourceOwner,
    pub scope: InstallationScope,
    pub location: PathBuf,
    /// Canonical hash of exactly what Cairn wrote. Null for manager-owned
    /// resources: Cairn did not write the bytes and does not own them.
    pub content_hash: Option<String>,
    pub artifact_schema: Option<u32>,
    pub artifact_revision: Option<String>,
    pub activation: ActivationState,
    /// Every agent bound to this resource. More than one is the shared case
    /// (FR-243).
    pub serves: Vec<AgentId>,
    /// Whether the container Cairn wrote into was on a single line before it
    /// did. One bit about Cairn's own edit, not a copy of the developer's file
    /// (FR-156, FR-238) — it is what lets removal restore the layout exactly.
    pub container_single_line: bool,
    /// Whether Cairn created the enclosing key, so pruning removes only what
    /// Cairn added and never an empty container the developer wrote.
    pub created_container: bool,
}

/// How a planned change is classified (FR-160).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAction {
    Add,
    Update,
    Unchanged,
    Conflict,
}

impl ChangeAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeAction::Add => "add",
            ChangeAction::Update => "update",
            ChangeAction::Unchanged => "unchanged",
            ChangeAction::Conflict => "conflict",
        }
    }
    pub fn writes(self) -> bool {
        matches!(self, ChangeAction::Add | ChangeAction::Update)
    }
}

/// One planned change, with the resource and file named (FR-160).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedChange {
    pub action: ChangeAction,
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub owner: ResourceOwner,
    pub scope: InstallationScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub detail: String,
}

/// A conflict that stops the operation, with the manual sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocking {
    pub agent: AgentId,
    pub kind: ResourceKind,
    pub condition: HealthCondition,
    pub detail: String,
    /// What the developer does instead. Never empty.
    pub manual_sequence: Vec<String>,
}

/// Something the developer must do after the change lands, inside the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostApplyAction {
    pub kind: String,
    pub agent: AgentId,
    pub instruction: String,
    pub verify_with: String,
}

/// The computed set of changes an operation would make.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationChangePlan {
    pub changes: Vec<PlannedChange>,
    /// The blast radius, stated explicitly. Mandatory (FR-161).
    pub untouched: Vec<String>,
    pub blocking: Vec<Blocking>,
    pub post_apply_actions: Vec<PostApplyAction>,
}

impl IntegrationChangePlan {
    /// True where applying this plan would write nothing.
    pub fn is_noop(&self) -> bool {
        !self.changes.iter().any(|c| c.action.writes())
    }

    /// True where a human decision is needed before anything can be applied.
    pub fn is_blocked(&self) -> bool {
        !self.blocking.is_empty()
    }

    pub fn changes_for(&self, agent: AgentId, kind: ResourceKind) -> Option<&PlannedChange> {
        self.changes
            .iter()
            .find(|c| c.agent == agent && c.kind == kind)
    }
}

/// Compute the plan for one agent from desired state and observation.
///
/// The rules, in one place:
///
/// - A resource that is absent and wanted is `add`.
/// - A resource that is present, owned by Cairn, and semantically equal to
///   canonical is `unchanged` — formatting is not a change (FR-223).
/// - A resource that is present and behind this build is `update`.
/// - A resource under an owner other than the record says, or shadowed by a
///   higher-precedence location, is `conflict` and blocks (FR-146, D38).
/// - A resource a developer hand-edited is `conflict`; setup never overwrites
///   it (FR-177, FR-221).
pub fn plan_agent(
    agent: AgentId,
    desired: &DesiredIntegrationState,
    observed: &[Observed],
) -> IntegrationChangePlan {
    let mut plan = IntegrationChangePlan::default();
    let Some(want) = desired.agents.iter().find(|a| a.agent == agent) else {
        return plan;
    };

    for wanted in &want.resources {
        let seen = observed.iter().find(|o| o.kind == wanted.kind);
        let target = seen.and_then(|o| o.location.clone()).map(display_path);

        let change = match seen.map(|o| o.condition) {
            // Asking a manager to distribute something Cairn already owns
            // directly would leave the developer with two copies of the same
            // resource and no way to tell which one is live. Both owners are
            // named and nothing is written (FR-146, FR-219, SC-112).
            _ if wanted.owner == ResourceOwner::Manager
                && seen.map(|o| o.owner) == Some(ResourceOwner::Direct)
                && seen.map(|o| o.condition.is_acceptable()).unwrap_or(false) =>
            {
                plan.blocking.push(Blocking {
                    agent,
                    kind: wanted.kind,
                    condition: HealthCondition::ConflictingOwner,
                    detail: format!(
                        "Cairn already owns {} for {agent} directly, and you asked an \
                         integration manager to distribute it as well",
                        wanted.kind
                    ),
                    manual_sequence: manual_sequence_for(
                        agent,
                        wanted.kind,
                        HealthCondition::ConflictingOwner,
                    ),
                });
                PlannedChange {
                    action: ChangeAction::Conflict,
                    agent,
                    kind: wanted.kind,
                    owner: ResourceOwner::Direct,
                    scope: wanted.scope,
                    target,
                    detail: "owned directly by Cairn and requested from a manager; \
                             migrate to one owner rather than keeping two copies"
                        .into(),
                }
            }

            // A manager owns it: Cairn verifies rather than writing (FR-234).
            _ if wanted.owner == ResourceOwner::Manager => PlannedChange {
                action: ChangeAction::Unchanged,
                agent,
                kind: wanted.kind,
                owner: ResourceOwner::Manager,
                scope: wanted.scope,
                target,
                detail: "distributed by the manager; Cairn verifies but does not write it".into(),
            },

            None | Some(HealthCondition::Missing) => PlannedChange {
                action: ChangeAction::Add,
                agent,
                kind: wanted.kind,
                owner: wanted.owner,
                scope: wanted.scope,
                target,
                detail: match &wanted.desired_version {
                    Some(v) => format!("install {v}"),
                    None => "install".into(),
                },
            },

            Some(HealthCondition::Healthy) | Some(HealthCondition::Shared) => PlannedChange {
                action: ChangeAction::Unchanged,
                agent,
                kind: wanted.kind,
                owner: wanted.owner,
                scope: wanted.scope,
                target,
                detail: match &wanted.desired_version {
                    Some(v) => format!("already at {v}"),
                    None => "already installed".into(),
                },
            },

            Some(HealthCondition::Outdated) | Some(HealthCondition::Duplicated) => PlannedChange {
                action: ChangeAction::Update,
                agent,
                kind: wanted.kind,
                owner: wanted.owner,
                scope: wanted.scope,
                target,
                detail: seen
                    .and_then(|o| o.detail.clone())
                    .unwrap_or_else(|| "bring to the current version".into()),
            },

            Some(HealthCondition::Modified) => {
                plan.blocking.push(Blocking {
                    agent,
                    kind: wanted.kind,
                    condition: HealthCondition::Modified,
                    detail: "a Cairn-managed resource was edited by hand".into(),
                    manual_sequence: vec![
                        "resolve the edited resource manually; Cairn will not overwrite it".into(),
                        "cairn setup".into(),
                    ],
                });
                PlannedChange {
                    action: ChangeAction::Conflict,
                    agent,
                    kind: wanted.kind,
                    owner: wanted.owner,
                    scope: wanted.scope,
                    target,
                    detail: "edited by hand; setup changes nothing".into(),
                }
            }

            Some(HealthCondition::InstalledNotActivated) => {
                plan.post_apply_actions.push(PostApplyAction {
                    kind: "activation".into(),
                    agent,
                    instruction: seen
                        .and_then(|o| o.remedy.clone())
                        .unwrap_or_else(|| "activate the handlers inside the agent".into()),
                    verify_with: "cairn setup".into(),
                });
                PlannedChange {
                    action: ChangeAction::Unchanged,
                    agent,
                    kind: wanted.kind,
                    owner: wanted.owner,
                    scope: wanted.scope,
                    target,
                    detail: "installed, awaiting activation inside the agent".into(),
                }
            }

            Some(HealthCondition::Migrating) => {
                plan.blocking.push(Blocking {
                    agent,
                    kind: wanted.kind,
                    condition: HealthCondition::Migrating,
                    detail: "an ownership migration for this resource is in progress".into(),
                    manual_sequence: vec![
                        "resolve the incomplete ownership change manually".into(),
                        "cairn setup".into(),
                    ],
                });
                PlannedChange {
                    action: ChangeAction::Conflict,
                    agent,
                    kind: wanted.kind,
                    owner: wanted.owner,
                    scope: wanted.scope,
                    target,
                    detail: "migrating".into(),
                }
            }

            Some(condition) => {
                // conflicting_owner, malformed_config, damaged_markers,
                // manager_action_required, unknown — every one of them needs a
                // human decision, and repair explains rather than guessing
                // (FR-174).
                plan.blocking.push(Blocking {
                    agent,
                    kind: wanted.kind,
                    condition,
                    detail: seen
                        .and_then(|o| o.detail.clone())
                        .unwrap_or_else(|| condition.to_string()),
                    manual_sequence: manual_sequence_for(agent, wanted.kind, condition),
                });
                PlannedChange {
                    action: ChangeAction::Conflict,
                    agent,
                    kind: wanted.kind,
                    owner: seen.map(|o| o.owner).unwrap_or(wanted.owner),
                    scope: wanted.scope,
                    target,
                    detail: condition.to_string(),
                }
            }
        };
        plan.changes.push(change);
    }

    plan.untouched = untouched_for(agent);
    plan
}

/// The blast radius, named per agent. Mandatory in every plan (FR-161).
fn untouched_for(agent: AgentId) -> Vec<String> {
    let mut v = vec![
        "every other MCP server".to_string(),
        "every other lifecycle handler and plugin".to_string(),
        "all content outside the cairn:managed markers".to_string(),
        "every provider, credential and model setting".to_string(),
    ];
    if agent == AgentId::Opencode {
        v.push("every other OpenCode plugin file".to_string());
    }
    v
}

fn manual_sequence_for(
    agent: AgentId,
    kind: ResourceKind,
    condition: HealthCondition,
) -> Vec<String> {
    match condition {
        HealthCondition::ConflictingOwner => vec![
            format!("choose one owner for {agent} {kind} and remove the conflicting copy"),
            "cairn setup".into(),
        ],
        HealthCondition::MalformedConfig => vec![
            "fix the configuration file by hand; Cairn will not rewrite a file it cannot parse"
                .into(),
            "cairn setup".into(),
        ],
        HealthCondition::DamagedMarkers => vec![
            "restore or remove the damaged cairn:managed markers by hand".into(),
            "cairn setup".into(),
        ],
        HealthCondition::ManagerActionRequired => vec![
            "complete the step inside the manager".into(),
            "cairn setup".into(),
        ],
        _ => vec!["cairn setup".into()],
    }
}

fn display_path(p: PathBuf) -> String {
    p.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desired::{Choices, DesiredIntegrationState};
    use crate::model::ArtifactVersion;

    fn desired() -> DesiredIntegrationState {
        DesiredIntegrationState::compose(
            &Choices {
                agents: vec![AgentId::ClaudeCode],
                ..Default::default()
            },
            &[AgentId::ClaudeCode],
            &[],
            ArtifactVersion::new(1, "aaaaaaaaaaaa"),
            ArtifactVersion::new(1, "bbbbbbbbbbbb"),
        )
    }

    #[test]
    fn an_absent_resource_is_an_add() {
        let plan = plan_agent(AgentId::ClaudeCode, &desired(), &[]);
        assert!(plan.changes.iter().all(|c| c.action == ChangeAction::Add));
        assert!(!plan.is_noop());
    }

    #[test]
    fn a_healthy_resource_is_unchanged() {
        // FR-157, SC-102.
        let observed: Vec<Observed> = ResourceKind::ALL
            .iter()
            .map(|k| Observed::new(*k, HealthCondition::Healthy))
            .collect();
        let plan = plan_agent(AgentId::ClaudeCode, &desired(), &observed);
        assert!(plan.is_noop());
        assert!(plan
            .changes
            .iter()
            .all(|c| c.action == ChangeAction::Unchanged));
    }

    #[test]
    fn a_hand_edit_blocks_setup() {
        // FR-177, FR-221, SC-130.
        let observed = vec![Observed::new(
            ResourceKind::Instructions,
            HealthCondition::Modified,
        )];
        let plan = plan_agent(AgentId::ClaudeCode, &desired(), &observed);
        assert!(plan.is_blocked());
        assert_eq!(
            plan.changes_for(AgentId::ClaudeCode, ResourceKind::Instructions)
                .unwrap()
                .action,
            ChangeAction::Conflict
        );
    }

    #[test]
    fn a_damaged_marker_blocks_setup() {
        let observed = vec![Observed::new(
            ResourceKind::Instructions,
            HealthCondition::DamagedMarkers,
        )];
        let plan = plan_agent(AgentId::ClaudeCode, &desired(), &observed);
        assert!(plan.is_blocked());
        assert_eq!(
            plan.changes_for(AgentId::ClaudeCode, ResourceKind::Instructions)
                .unwrap()
                .action,
            ChangeAction::Conflict
        );
    }

    #[test]
    fn every_blocking_entry_carries_a_manual_sequence() {
        // FR-174: explain the conflict and the options, change nothing.
        for condition in [
            HealthCondition::ConflictingOwner,
            HealthCondition::MalformedConfig,
            HealthCondition::DamagedMarkers,
            HealthCondition::Modified,
        ] {
            let observed = vec![Observed::new(ResourceKind::Instructions, condition)];
            let plan = plan_agent(AgentId::ClaudeCode, &desired(), &observed);
            assert!(plan.is_blocked(), "{condition} did not block");
            assert!(plan.blocking.iter().all(|b| !b.manual_sequence.is_empty()));
        }
    }

    #[test]
    fn every_plan_names_its_blast_radius() {
        // FR-161: `untouched` is mandatory.
        let plan = plan_agent(AgentId::ClaudeCode, &desired(), &[]);
        assert!(!plan.untouched.is_empty());
        assert!(plan
            .untouched
            .iter()
            .any(|u| u.contains("outside the cairn:managed markers")));
    }

    /// Ask a manager to distribute `mcp` for an agent whose `mcp` Cairn
    /// already owns directly.
    fn via_manager() -> DesiredIntegrationState {
        DesiredIntegrationState::compose(
            &Choices {
                agents: vec![AgentId::ClaudeCode],
                via_manager: Some(crate::model::ManagerId::CcSwitch),
                manager_resources: vec![ResourceKind::Mcp],
                manager_apps: vec!["claude".into()],
                ..Default::default()
            },
            &[AgentId::ClaudeCode],
            &[],
            ArtifactVersion::new(1, "aaaaaaaaaaaa"),
            ArtifactVersion::new(1, "bbbbbbbbbbbb"),
        )
    }

    #[test]
    fn two_owners_for_one_resource_is_a_conflict_rather_than_a_second_copy() {
        // FR-146, FR-219, SC-112: a manager import on top of a resource Cairn
        // already owns directly would leave two copies of the same entry and
        // no way to tell which one is live.
        let observed = vec![Observed::new(ResourceKind::Mcp, HealthCondition::Healthy)
            .owned_by(ResourceOwner::Direct)];
        let plan = plan_agent(AgentId::ClaudeCode, &via_manager(), &observed);

        assert!(plan.is_blocked(), "a second owner was accepted silently");
        let mcp = plan
            .changes_for(AgentId::ClaudeCode, ResourceKind::Mcp)
            .expect("an mcp change");
        assert_eq!(mcp.action, ChangeAction::Conflict);
        assert!(
            !mcp.action.writes(),
            "a conflicting owner still wrote something"
        );

        let blocking = plan
            .blocking
            .iter()
            .find(|b| b.kind == ResourceKind::Mcp)
            .expect("a blocking entry");
        assert_eq!(blocking.condition, HealthCondition::ConflictingOwner);
        // Both owners are named, and the way out is stated.
        assert!(blocking.detail.contains("directly"), "{}", blocking.detail);
        assert!(blocking.detail.contains("manager"), "{}", blocking.detail);
        assert!(blocking
            .manual_sequence
            .iter()
            .any(|s| s.contains("choose one owner")));
    }

    #[test]
    fn a_manager_owned_resource_cairn_does_not_hold_is_verified_not_written() {
        // The other half: with nothing of Cairn's own in place, the manager
        // owns it and Cairn writes nothing (FR-234).
        let plan = plan_agent(AgentId::ClaudeCode, &via_manager(), &[]);
        let mcp = plan
            .changes_for(AgentId::ClaudeCode, ResourceKind::Mcp)
            .expect("an mcp change");
        assert_eq!(mcp.action, ChangeAction::Unchanged);
        assert_eq!(mcp.owner, ResourceOwner::Manager);
        assert!(!mcp.action.writes());
    }

    #[test]
    fn an_inactive_handler_produces_a_post_apply_action_not_a_write() {
        // FR-209: report the step, do not claim the level.
        let observed = vec![Observed::new(
            ResourceKind::Lifecycle,
            HealthCondition::InstalledNotActivated,
        )
        .remedy("run `codex hooks trust`")];
        let plan = plan_agent(AgentId::Codex, &desired_codex(), &observed);
        assert_eq!(
            plan.changes_for(AgentId::Codex, ResourceKind::Lifecycle)
                .unwrap()
                .action,
            ChangeAction::Unchanged
        );
        assert_eq!(plan.post_apply_actions.len(), 1);
        assert!(plan.post_apply_actions[0].instruction.contains("trust"));
    }

    fn desired_codex() -> DesiredIntegrationState {
        DesiredIntegrationState::compose(
            &Choices {
                agents: vec![AgentId::Codex],
                ..Default::default()
            },
            &[AgentId::Codex],
            &[],
            ArtifactVersion::new(1, "aaaaaaaaaaaa"),
            ArtifactVersion::new(1, "bbbbbbbbbbbb"),
        )
    }
}

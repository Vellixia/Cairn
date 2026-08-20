//! The integration command surface (`contracts/integration-cli.md`).
//!
//! Every command here rides Feature 001's envelope and exit codes, and every
//! one of them follows the same sequence: inspect current state, compute the
//! intended change, validate it, apply it, verify the result (FR-151). Only
//! the last two steps touch the filesystem, which is what makes `--dry-run`
//! inert by construction rather than by care (SC-118).
//!
//! Configuration operations **fail loudly**. None of them is fail-soft; only
//! the hook path is, and it is unchanged (FR-193, FR-196).

use crate::client;
use crate::Output;
use cairn_core::wire::{codes, MigrationAction, Request, WireError};
use cairn_integrate::adapter::{IntegrationManager, ManagerActionRequired, Observed};
use cairn_integrate::apply::RecoveryWrite;
use cairn_integrate::capability::{self, CapabilityProfile, Evidence, EvidenceKind, LevelInputs};
use cairn_integrate::desired::{Choices, DesiredIntegrationState, RecordedResource};
use cairn_integrate::install;
use cairn_integrate::managers::cc_switch::{self, CcSwitch};
use cairn_integrate::model::{
    ActivationState, AgentId, CompatibilityClassification, HealthCondition, InstallationScope,
    IntegrationLevel, ManagerId, ResourceKind, ResourceOwner,
};
use cairn_integrate::plan::{
    plan_agent, ChangeAction, IntegrationChangePlan, Intent, RecordedInstall,
};
use cairn_integrate::scope::{self, Env};
use cairn_integrate::{adapter_for, render, revision};
use serde_json::{json, Value};
use std::path::PathBuf;

/// What the developer asked for on the command line.
#[derive(Debug, Default, Clone)]
pub struct Options {
    pub agent: Option<AgentId>,
    /// Detect everything installed and propose a plan covering all of it.
    /// Bare `cairn connect` means the same thing (FR-163).
    pub auto: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub shared: bool,
    pub scopes: Vec<(ResourceKind, InstallationScope)>,
    pub via: Option<ManagerId>,
    pub apps: Vec<String>,
    pub only: Vec<ResourceKind>,
    pub force: bool,
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into())
}

fn env() -> Env {
    Env::discover(cwd())
}

fn err(code: &str, message: impl Into<String>) -> WireError {
    WireError::new(code, message)
}

/// The local record, as the planner wants it.
struct Snapshot {
    installs: Vec<RecordedInstall>,
    evidence: std::collections::BTreeMap<AgentId, Vec<Evidence>>,
    migrations: Vec<Value>,
}

impl Snapshot {
    fn for_agent(&self, agent: AgentId) -> Vec<RecordedInstall> {
        self.installs
            .iter()
            .filter(|r| r.agent == agent || r.serves.contains(&agent))
            .cloned()
            .collect()
    }
    fn recorded_resources(&self) -> Vec<RecordedResource> {
        self.installs
            .iter()
            .map(|r| RecordedResource {
                agent: r.agent,
                kind: r.kind,
                owner: r.owner,
                scope: r.scope,
                activation: r.activation,
            })
            .collect()
    }
    fn migration_for(&self, agent: AgentId, kind: ResourceKind) -> Option<&Value> {
        self.migrations
            .iter()
            .find(|m| m["agent"] == agent.as_str() && m["kind"] == kind.as_str())
    }
}

async fn snapshot() -> Result<Snapshot, WireError> {
    let value = client::send(&Request::IntegrationSnapshot { cwd: cwd() }).await?;
    let mut installs = Vec::new();
    let mut evidence = std::collections::BTreeMap::new();

    for entry in value["agents"].as_array().cloned().unwrap_or_default() {
        let Some(agent) = entry["agent"].as_str().and_then(AgentId::parse) else {
            continue;
        };
        for r in entry["resources"].as_array().cloned().unwrap_or_default() {
            let Some(kind) = r["kind"].as_str().and_then(ResourceKind::parse) else {
                continue;
            };
            installs.push(RecordedInstall {
                agent,
                kind,
                owner: r["owner"]
                    .as_str()
                    .and_then(ResourceOwner::parse)
                    .unwrap_or(ResourceOwner::Direct),
                scope: r["scope"]
                    .as_str()
                    .and_then(InstallationScope::parse)
                    .unwrap_or(InstallationScope::User),
                location: PathBuf::from(r["location"].as_str().unwrap_or_default()),
                content_hash: r["content_hash"].as_str().map(str::to_string),
                artifact_schema: r["artifact_schema"].as_i64().map(|v| v as u32),
                artifact_revision: r["artifact_revision"].as_str().map(str::to_string),
                activation: r["activation"]
                    .as_str()
                    .and_then(ActivationState::parse)
                    .unwrap_or(ActivationState::NotApplicable),
                serves: r["serves"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|a| a.as_str().and_then(AgentId::parse))
                    .collect(),
                container_single_line: r["container_single_line"].as_bool().unwrap_or(false),
                created_container: r["created_container"].as_bool().unwrap_or(false),
            });
        }
        evidence.insert(
            agent,
            entry["evidence"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|e| {
                    Some(Evidence {
                        capability: e["capability"].as_str()?.to_string(),
                        kind: EvidenceKind::parse(e["evidence"].as_str()?)?,
                        agent_version: e["agent_version"].as_str().map(str::to_string),
                        degraded: e["degraded"].as_bool(),
                    })
                })
                .collect(),
        );
    }
    Ok(Snapshot {
        installs,
        evidence,
        migrations: value["migrations"].as_array().cloned().unwrap_or_default(),
    })
}

/// How many boundaries are currently owed a handoff.
///
/// A boundary that acknowledged but has not produced its handoff reads as
/// owed, not complete, so it withholds the completion guarantee (FR-240
/// clause 4).
async fn boundary_owed() -> bool {
    match client::send(&Request::Status { cwd: cwd() }).await {
        Ok(v) => v["sessions_awaiting_handoff"].as_i64().unwrap_or(0) > 0,
        Err(_) => false,
    }
}

/// One agent's detected state, capability profile and computed level.
struct AgentState {
    agent: AgentId,
    /// Whether Cairn has installed anything for this agent yet.
    connected: bool,
    detection: cairn_integrate::Detection,
    profile: CapabilityProfile,
    compatibility: CompatibilityClassification,
    incompatible_because: Option<String>,
    outcome: capability::LevelOutcome,
    observed: Vec<Observed>,
}

async fn assess(
    env: &Env,
    snap: &Snapshot,
    owed: bool,
    agent: AgentId,
) -> Result<AgentState, WireError> {
    let adapter = adapter_for(agent);
    let detection = adapter.detect(env);
    let (compatibility, incompatible_because) =
        capability::classify_version(agent, detection.version.as_deref());

    let record = snap.for_agent(agent);
    let observed = adapter.inspect(env, &record);

    let mut profile = adapter.capabilities(&detection);
    // A trust-gated agent whose handlers are not active is never reported at
    // the level those handlers would provide once they are (FR-209).
    let activation = observed
        .iter()
        .find(|o| o.kind == ResourceKind::Lifecycle)
        .map(|o| o.activation)
        .unwrap_or(ActivationState::NotApplicable);
    profile.apply_activation(activation);
    if let Some(e) = snap.evidence.get(&agent) {
        profile.apply_evidence(e);
    }

    let present = |kind: ResourceKind| {
        observed
            .iter()
            .any(|o| o.kind == kind && o.condition.is_acceptable())
    };
    let inputs = LevelInputs {
        mcp_present: present(ResourceKind::Mcp) || agent == AgentId::GenericMcp,
        instructions_present: present(ResourceKind::Instructions),
        skill_present: present(ResourceKind::Skill),
        detected: detection.detected,
        safe_to_integrate: compatibility != CompatibilityClassification::Unsupported,
        boundary_owed: owed,
        activation,
        // The demonstration FR-208 requires is the release-evidence benchmark
        // (`perf_session_close`, SC-128), not something a running Cairn can
        // re-measure: one developer's machine does not produce 100 boundaries.
        // The claim is a constant that benchmark is obliged to keep true, and
        // an agent that imposes no budget has nothing to demonstrate.
        close_within_budget: Some(capability::budget_demonstrated(agent)),
    };
    let outcome = capability::derive_level(&profile, &inputs);
    let connected = snap.installs.iter().any(|r| r.agent == agent);
    Ok(AgentState {
        agent,
        connected,
        detection,
        profile,
        compatibility,
        incompatible_because,
        outcome,
        observed,
    })
}

// ---------------------------------------------------------------- agents ---

/// `cairn agents` — detection only. Makes no change, requires no network
/// (FR-105).
pub async fn agents() -> Result<Output, WireError> {
    let env = env();
    let snap = snapshot().await.unwrap_or(Snapshot {
        installs: vec![],
        evidence: Default::default(),
        migrations: vec![],
    });
    let owed = boundary_owed().await;

    let mut rows = Vec::new();
    let mut text = String::new();
    for agent in AgentId::ALL {
        let state = assess(&env, &snap, owed, agent).await?;
        if !state.detection.detected {
            continue;
        }
        text.push_str(&format!(
            "{:<13} {:<9} {:<22} {:<21} {}\n",
            agent.as_str(),
            state.detection.version.as_deref().unwrap_or("-"),
            state.compatibility.as_str().replace('_', "-"),
            // Derived from this agent's capability profile, never from a table
            // someone maintains: a mode that over-claims is a defect, and a
            // table is exactly how one comes to over-claim (FR-426, FR-427).
            state.profile.continuity_mode().as_str().replace('_', "-"),
            level_line(&state)
        ));
        rows.push(agent_json(&state));
    }

    let manager = CcSwitch;
    let detection = manager.detect(&env);
    let manager_json = if detection.detected {
        text.push_str(&format!(
            "{:<13} {:<9} {:<22} manager    (mcp, skill → {})\n",
            "cc-switch",
            detection.version.as_deref().unwrap_or("-"),
            "compatible-unverified",
            manager.target_apps().join(", ")
        ));
        json!({
            "manager": ManagerId::CcSwitch.as_str(),
            "detected": true,
            "version": detection.version,
            "compatibility": "compatible_unverified",
            "distributable_resources": manager.distributable().iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            "target_apps": manager.target_apps(),
        })
    } else {
        Value::Null
    };
    if text.is_empty() {
        text.push_str("No supported agent or integration manager detected here.\n");
    }
    Ok(Output::with(
        json!({ "agents": rows, "manager": manager_json }),
        text,
    ))
}

/// A level is never printed as a bare word where a capability is missing: the
/// parenthetical naming the missing behavior is part of the contract (FR-110,
/// FR-111).
///
/// An agent Cairn has not connected yet is reported as exactly that. Calling
/// it UNSUPPORTED would be a claim about the agent rather than about Cairn's
/// own state, and FR-109 reserves that word for a version that cannot be
/// integrated safely.
fn level_line(state: &AgentState) -> String {
    // The generic path is never "connected": Cairn installs nothing for a
    // client it has no adapter for, and the developer pastes the exported
    // block themselves (FR-131). Telling them to run `cairn connect` would be
    // advice for a command that has nothing to do here.
    if !state.connected && state.agent != AgentId::GenericMcp {
        return "not connected   (run `cairn connect`)".into();
    }
    let mut s = state.outcome.level.display().to_string();
    if state.outcome.level != IntegrationLevel::Full {
        if let Some(first) = state.outcome.missing_behaviors.first() {
            // The behaviors read as noun phrases, some with a leading article.
            // "no the Cairn usage contract" is not a sentence.
            let first = first.strip_prefix("the ").unwrap_or(first);
            s.push_str(&format!("   (no {first})"));
        } else if let Some(first) = state.outcome.awaited_behaviors.first() {
            s.push_str(&format!("   (awaiting {first})"));
        }
    }
    s
}

/// What a continuity mode means, in the words a developer needs.
///
/// The mode is derived; this is only its rendering. It says what the agent will
/// do rather than naming a state, because "agent_initiated" tells someone
/// nothing about whether their work survives a compaction.
fn continuity_note(mode: cairn_core::domain::ContinuityMode) -> &'static str {
    use cairn_core::domain::ContinuityMode::*;
    match mode {
        Automatic => "automatic — Cairn is called back before and after compaction",
        AgentInitiated => {
            "agent_initiated — Cairn is warned before compaction but not called back after; \
             the agent must ask for context with reason=post_compaction"
        }
        UnavailableAutomatic => {
            "unavailable_automatic — this agent reports no compaction event; write a \
             checkpoint with cairn_session action=checkpoint before you compact"
        }
    }
}

fn agent_json(state: &AgentState) -> Value {
    let (guaranteed, conditional, absent) = state.profile.lifecycle_coverage();
    let mut caps = serde_json::Map::new();
    for c in capability::Capability::ALL {
        let s = state.profile.get(c);
        let mut entry = json!({
            "availability": s.availability.as_str(),
            "confidence": s.confidence.as_str(),
        });
        if let Some(dep) = &s.depends_on {
            entry["depends_on"] = json!(dep);
        }
        caps.insert(c.as_str().to_string(), entry);
    }
    caps.insert(
        "handlers_require_trust".into(),
        json!(if state.profile.handlers_require_trust {
            "yes"
        } else {
            "no"
        }),
    );
    let mut v = json!({
        "agent": state.agent.as_str(),
        "detected": state.detection.detected,
        "connected": state.connected,
        // What this agent can honestly promise about surviving compaction
        // (FR-426). `automatic` means Cairn is called back; `agent_initiated`
        // means the agent must ask; `unavailable_automatic` means neither.
        "continuity_mode": state.profile.continuity_mode().as_str(),
        "version": state.detection.version,
        "compatibility": state.compatibility.as_str(),
        "level": state.outcome.level.as_str(),
        "completion_guarantee": state.outcome.completion_guarantee.as_str(),
        "capabilities": caps,
        "lifecycle_coverage": {
            "guaranteed": guaranteed,
            "conditional": conditional,
            "absent": absent,
        },
        // Mandatory below FULL, and mandatory whenever anything is expected
        // (FR-111, FR-188, FR-245).
        "missing_behaviors": state.outcome.missing_behaviors,
        "unverified_behaviors": state.outcome.unverified_behaviors,
        "awaited_behaviors": state.outcome.awaited_behaviors,
        "conditional_behaviors": state.profile.conditional_behaviors(),
    });
    // How sessions here actually reach a terminal state, wherever that is not
    // "the agent said so" (FR-207, FR-209, FR-229).
    if let Some(note) = &state.outcome.completion_note {
        v["session_completion"] = json!(note);
    }
    if let Some(why) = &state.incompatible_because {
        v["incompatible_because"] = json!(why);
    }
    v
}

// --------------------------------------------------------------- connect ---

/// `cairn connect` — install or update an integration.
pub async fn connect(opts: &Options) -> Result<Output, WireError> {
    let env = env();
    let snap = snapshot().await?;
    let owed = boundary_owed().await;

    // Bare `cairn connect` with no agent is equivalent to `--auto` (FR-163).
    let detected: Vec<AgentId> = detected_agents(&env);
    // Bare `cairn connect` is `--auto`: detect everything installed and
    // propose a plan covering all of it (FR-163).
    let selected: Vec<AgentId> = match opts.agent {
        Some(a) if !opts.auto => vec![a],
        _ => detected.clone(),
    };
    if selected.is_empty() {
        return Err(err(
            codes::AGENT_NOT_DETECTED,
            "no supported agent was detected here",
        ));
    }
    for a in &selected {
        if *a != AgentId::GenericMcp && !detected.contains(a) {
            return Err(err(
                codes::AGENT_NOT_DETECTED,
                format!("{a} is not installed on this machine"),
            ));
        }
    }

    // Adopt any Feature 001 installation before composing intent, so its
    // resources are recorded where they already are rather than relocated
    // (FR-217).
    let adopted = adopt_legacy(&env, &snap, &selected).await?;

    let desired = compose(opts, &selected, &detected, &snap, &adopted);
    let mut plan = IntegrationChangePlan::default();
    let mut states = Vec::new();
    for agent in &selected {
        let state = assess(&env, &snap, owed, *agent).await?;
        let mut p = plan_agent(Intent::Connect, *agent, &desired, &state.observed);
        plan.changes.append(&mut p.changes);
        plan.blocking.append(&mut p.blocking);
        plan.post_apply_actions.append(&mut p.post_apply_actions);
        if plan.untouched.is_empty() {
            plan.untouched = p.untouched;
        }
        states.push(state);
    }

    if opts.dry_run {
        return Ok(Output::with(
            plan_json(&plan, true),
            render_plan(&plan, true),
        ));
    }
    if plan.is_blocked() {
        return Err(blocking_error(&plan));
    }
    // Non-interactive runs without `--yes` print the plan and stop (FR-164).
    if !opts.yes && !is_interactive() {
        return Err(err(
            codes::CONFIRMATION_REQUIRED,
            "re-run with --yes to apply this plan, or --dry-run to preview it",
        ));
    }

    let result = apply_plan(&env, &desired, &plan, &states, opts).await?;
    Ok(result)
}

fn detected_agents(env: &Env) -> Vec<AgentId> {
    AgentId::ALL
        .into_iter()
        .filter(|a| *a != AgentId::GenericMcp && adapter_for(*a).detect(env).detected)
        .collect()
}

fn compose(
    opts: &Options,
    selected: &[AgentId],
    detected: &[AgentId],
    snap: &Snapshot,
    adopted: &[RecordedResource],
) -> DesiredIntegrationState {
    let mut recorded = snap.recorded_resources();
    for a in adopted {
        if !recorded
            .iter()
            .any(|r| r.agent == a.agent && r.kind == a.kind)
        {
            recorded.push(a.clone());
        }
    }
    let choices = Choices {
        agents: selected.to_vec(),
        shared: opts.shared,
        scope_overrides: opts.scopes.clone(),
        via_manager: opts.via,
        manager_resources: if opts.via.is_some() {
            vec![ResourceKind::Mcp, ResourceKind::Skill]
        } else {
            vec![]
        },
        manager_apps: opts.apps.clone(),
        disable: vec![],
        only: opts.only.clone(),
    };
    DesiredIntegrationState::compose(
        &choices,
        detected,
        &recorded,
        render::Contract::canonical().version(),
        cairn_integrate::model::ArtifactVersion::new(
            revision::embedded_schema(),
            revision::embedded_revision(),
        ),
    )
}

/// Recognize and adopt a Feature 001 installation, once.
///
/// Matched by a closed set of exact shapes, adopted in place at the scope it
/// is already at, recorded, and never matched by shape again (FR-139, FR-217).
async fn adopt_legacy(
    env: &Env,
    snap: &Snapshot,
    selected: &[AgentId],
) -> Result<Vec<RecordedResource>, WireError> {
    if !selected.contains(&AgentId::ClaudeCode) {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for found in cairn_integrate::agents::claude_code::detect_legacy(env) {
        // Already recorded: the bridge runs once, and after that ownership is
        // the record, not the shape.
        if snap
            .installs
            .iter()
            .any(|r| r.agent == AgentId::ClaudeCode && r.kind == found.kind)
        {
            continue;
        }
        client::send(&Request::IntegrationUpsertAgent {
            cwd: cwd(),
            agent: AgentId::ClaudeCode.as_str().into(),
            adapter_version: 1,
            detected_version: None,
            compatibility: CompatibilityClassification::CompatibleUnverified
                .as_str()
                .into(),
            level: IntegrationLevel::McpPlus.as_str().into(),
            completion_guarantee: "not_demonstrated".into(),
        })
        .await?;
        client::send(&Request::IntegrationBind {
            cwd: cwd(),
            agent: AgentId::ClaudeCode.as_str().into(),
            kind: found.kind.as_str().into(),
            owner: ResourceOwner::Direct.as_str().into(),
            scope: found.scope.as_str().into(),
            location: found.path.display().to_string(),
            content_hash: None,
            artifact_schema: None,
            artifact_revision: None,
            activation: ActivationState::NotApplicable.as_str().into(),
            container_single_line: false,
            created_container: false,
        })
        .await?;
        out.push(RecordedResource {
            agent: AgentId::ClaudeCode,
            kind: found.kind,
            owner: ResourceOwner::Direct,
            scope: found.scope,
            activation: ActivationState::NotApplicable,
        });
    }
    Ok(out)
}

/// Apply a plan: write, verify, record, and report both halves on partial
/// failure (FR-155, FR-196).
async fn apply_plan(
    env: &Env,
    desired: &DesiredIntegrationState,
    plan: &IntegrationChangePlan,
    states: &[AgentState],
    opts: &Options,
) -> Result<Output, WireError> {
    let mut applied = Vec::new();
    let mut not_applied = Vec::new();
    let mut artifacts: Vec<String> = Vec::new();
    // What this run actually wrote, so the binding pass below does not
    // re-record it from the changed file.
    let mut written: Vec<(AgentId, ResourceKind)> = Vec::new();

    // The agent row comes first: a binding references it, and recording a
    // resource for an agent that does not exist yet is a foreign-key failure
    // rather than an integration.
    for state in states {
        record_agent(state).await?;
    }

    for change in &plan.changes {
        if !change.action.writes() || change.action == ChangeAction::Remove {
            continue;
        }
        let Some(want) = desired.resource(change.agent, change.kind) else {
            continue;
        };
        if want.owner == ResourceOwner::Manager {
            continue;
        }

        // A forced repair preserves the Cairn-owned content first (FR-222).
        if opts.force && change.action == ChangeAction::Update {
            if let Some(owned) = install::owned_content(env, change.agent, change.kind, want.scope)
            {
                match preserve(env, change.agent, change.kind, &owned).await {
                    Ok(path) => artifacts.push(path),
                    Err(e) => {
                        not_applied.push(json!({
                            "agent": change.agent.as_str(),
                            "kind": change.kind.as_str(),
                            "reason": e.message,
                        }));
                        continue;
                    }
                }
            }
        }

        match install::materialize_install(env, change.agent, change.kind, want.scope) {
            Ok(m) => match install::commit(&m) {
                Ok(()) => {
                    record_resource(change.agent, &m).await?;
                    written.push((change.agent, change.kind));
                    applied.push(json!({
                        "agent": change.agent.as_str(),
                        "kind": change.kind.as_str(),
                        "target": m.location.display().to_string(),
                    }));
                }
                Err(e) => not_applied.push(json!({
                    "agent": change.agent.as_str(),
                    "kind": change.kind.as_str(),
                    "target": m.location.display().to_string(),
                    "reason": e.to_string(),
                })),
            },
            Err(e) => not_applied.push(json!({
                "agent": change.agent.as_str(),
                "kind": change.kind.as_str(),
                "reason": e.to_string(),
            })),
        }
    }

    // Ensure every agent's binding exists, whether or not this run wrote the
    // bytes. Connect is "ensure this binding exists" (D28): a resource another
    // agent already installed is exactly the shared case, and skipping it
    // because nothing changed is how a shared block loses the consumer that
    // depends on it (FR-243).
    let prior = snapshot().await.ok();
    for state in states {
        let Some(want) = desired.agents.iter().find(|a| a.agent == state.agent) else {
            continue;
        };
        for wanted in &want.resources {
            if wanted.owner == ResourceOwner::Manager {
                continue;
            }
            // Already recorded above, from the materialization that was
            // actually applied. Re-recording here would overwrite the layout
            // bits with values read back from the file Cairn has just
            // changed — which is how a minified container stopped being
            // restorable on disconnect.
            if written.contains(&(state.agent, wanted.kind)) {
                continue;
            }
            let present = state
                .observed
                .iter()
                .any(|o| o.kind == wanted.kind && o.condition.is_acceptable());
            if !present && !applied.iter().any(|a| a["kind"] == wanted.kind.as_str()) {
                continue;
            }
            if let Ok(mut m) =
                install::materialize_install(env, state.agent, wanted.kind, wanted.scope)
            {
                // Where Claude Code's Cairn Skill exists, OpenCode binds to
                // *that* resource rather than writing a second copy (D28), but
                // `scope::location` is a fixed table and cannot see it. Left
                // alone it records OpenCode's own path, which nothing ever
                // installs -- so doctor reads a location with no Skill in it and
                // calls a resource `missing` that is in fact shared and healthy,
                // and the next `connect` plans an `ADD` for the duplicate D28
                // forbids. The record has to name the same location the
                // adapter's `inspect` reports. Resolved from disk rather than
                // from `prior`, so an already-wrong record heals itself.
                if state.agent == AgentId::Opencode && wanted.kind == ResourceKind::Skill {
                    m.location = cairn_integrate::agents::opencode::skill_location(env, &[]);
                }
                // This resource was installed by an earlier run or by another
                // agent, so the file already holds Cairn's content and its
                // layout bits describe *that* edit. They are carried forward
                // rather than recomputed from a file that has already changed.
                if let Some(p) = prior.as_ref().and_then(|s| {
                    s.installs
                        .iter()
                        .find(|r| r.kind == wanted.kind && r.location == m.location)
                }) {
                    m.container_single_line = p.container_single_line;
                    m.created_container = p.created_container;
                }
                record_resource(state.agent, &m).await?;
            }
        }
    }

    // Re-inspect so the reported level is the one that now holds, and record
    // what that inspection established.
    let snap = snapshot().await?;
    let owed = boundary_owed().await;
    for state in states {
        let observed = adapter_for(state.agent).inspect(env, &snap.for_agent(state.agent));
        record_introspection(state.agent, &observed).await?;
    }
    let snap = snapshot().await?;
    let mut levels = serde_json::Map::new();
    for state in states {
        let after = assess(env, &snap, owed, state.agent).await?;
        levels.insert(
            state.agent.as_str().into(),
            json!(after.outcome.level.as_str()),
        );
    }

    let data = json!({
        "applied": applied,
        "not_applied": not_applied,
        "verified": not_applied.is_empty(),
        "recovery_artifacts": artifacts,
        "post_apply_actions": plan.post_apply_actions,
        "level_after": levels,
    });

    if !not_applied.is_empty() {
        let mut e = err(
            codes::PARTIAL_APPLY,
            format!(
                "{} of {} changes applied; the integration is incomplete",
                applied.len(),
                applied.len() + not_applied.len()
            ),
        );
        e.message.push_str(" — run `cairn doctor` for the detail");
        return Err(e);
    }

    let mut text = String::new();
    for a in applied.iter() {
        text.push_str(&format!(
            "installed  {:<13} {}\n",
            a["kind"].as_str().unwrap_or(""),
            a["target"].as_str().unwrap_or("")
        ));
    }
    if !artifacts.is_empty() {
        for p in &artifacts {
            text.push_str(&format!("preserved  {p}\n"));
        }
    }
    for action in &plan.post_apply_actions {
        text.push_str(&format!(
            "\n{}\n  then: {}\n",
            action.instruction, action.verify_with
        ));
    }
    if text.is_empty() {
        text.push_str("unchanged — everything Cairn owns is already installed\n");
    }
    Ok(Output::with(data, text))
}

/// Preserve Cairn-owned content before a forced change, and record where.
async fn preserve(
    env: &Env,
    agent: AgentId,
    kind: ResourceKind,
    owned: &str,
) -> Result<String, WireError> {
    let home = cairn_core::paths::home();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let source = scope::location(
        env,
        agent,
        kind,
        scope::resolve_scope(agent, kind, false, None).unwrap_or(InstallationScope::User),
    )
    .map(|p| p.display().to_string())
    .unwrap_or_default();

    let path = cairn_integrate::apply::write_recovery(
        &home,
        &now,
        &RecoveryWrite {
            agent,
            kind,
            source_path: PathBuf::from(&source),
            owned_content: owned.to_string(),
        },
    )
    .map_err(|e| err(e.code(), e.to_string()))?;

    // Only the path is ever printed or recorded; never the content (FR-239).
    client::send(&Request::IntegrationRecovery {
        cwd: cwd(),
        agent: agent.as_str().into(),
        kind: kind.as_str().into(),
        source_path: source,
        artifact_path: path.display().to_string(),
        content_hash: cairn_integrate::model::canonical_hash(owned),
    })
    .await?;
    Ok(path.display().to_string())
}

async fn record_resource(agent: AgentId, m: &install::Materialized) -> Result<(), WireError> {
    client::send(&Request::IntegrationBind {
        cwd: cwd(),
        agent: agent.as_str().into(),
        kind: m.kind.as_str().into(),
        owner: ResourceOwner::Direct.as_str().into(),
        scope: m.scope.as_str().into(),
        location: m.location.display().to_string(),
        content_hash: m.content_hash.clone(),
        artifact_schema: m.artifact.as_ref().map(|a| a.schema as i64),
        artifact_revision: m.artifact.as_ref().map(|a| a.revision.clone()),
        activation: if agent == AgentId::Codex && m.kind == ResourceKind::Lifecycle {
            ActivationState::PendingUserTrust.as_str().into()
        } else {
            ActivationState::NotApplicable.as_str().into()
        },
        container_single_line: m.container_single_line,
        created_container: m.created_container,
    })
    .await?;

    Ok(())
}

/// Record introspection evidence for every resource Cairn can read back.
///
/// The evidence is the *read-back*, not the write: a resource that was already
/// in place is just as established as one this run installed, and requiring a
/// rewrite to prove it would make the level depend on when Cairn last had
/// something to do (FR-242, FR-245, D19a).
async fn record_introspection(agent: AgentId, observed: &[Observed]) -> Result<(), WireError> {
    for o in observed {
        if !o.condition.is_acceptable() || o.condition == HealthCondition::Unknown {
            continue;
        }
        client::send(&Request::IntegrationEvidence {
            cwd: cwd(),
            agent: agent.as_str().into(),
            capability: capability::config_evidence_key(o.kind).into(),
            evidence: EvidenceKind::Introspection.as_str().into(),
            agent_version: None,
            degraded: None,
        })
        .await?;
    }
    Ok(())
}

async fn record_agent(state: &AgentState) -> Result<(), WireError> {
    client::send(&Request::IntegrationUpsertAgent {
        cwd: cwd(),
        agent: state.agent.as_str().into(),
        adapter_version: 1,
        detected_version: state.detection.version.clone(),
        compatibility: state.compatibility.as_str().into(),
        level: state.outcome.level.as_str().into(),
        completion_guarantee: state.outcome.completion_guarantee.as_str().into(),
    })
    .await?;
    // An agent upgrade discards observation evidence and keeps introspection
    // evidence (FR-245).
    client::send(&Request::IntegrationInvalidateEvidence {
        cwd: cwd(),
        agent: state.agent.as_str().into(),
        detected_version: state.detection.version.clone(),
    })
    .await?;
    Ok(())
}

fn is_interactive() -> bool {
    std::env::var_os("CI").is_none() && std::io::IsTerminal::is_terminal(&std::io::stdin())
}

fn blocking_error(plan: &IntegrationChangePlan) -> WireError {
    let first = &plan.blocking[0];
    let code = match first.condition {
        HealthCondition::Modified => codes::RESOURCE_MODIFIED,
        HealthCondition::DamagedMarkers => codes::DAMAGED_MARKERS,
        HealthCondition::MalformedConfig => codes::MALFORMED_CONFIG,
        HealthCondition::ConflictingOwner => codes::CONFLICTING_OWNER,
        HealthCondition::Duplicated => codes::DUPLICATE_RESOURCE,
        HealthCondition::Migrating => codes::MIGRATION_IN_PROGRESS,
        HealthCondition::ManagerActionRequired => codes::MANAGER_ACTION_REQUIRED,
        _ => codes::INVALID_REQUEST,
    };
    err(
        code,
        format!(
            "{} {}: {} — {}",
            first.agent,
            first.kind,
            first.detail,
            first.manual_sequence.join("; ")
        ),
    )
}

fn plan_json(plan: &IntegrationChangePlan, dry_run: bool) -> Value {
    json!({
        "dry_run": dry_run,
        "changes": plan.changes,
        "untouched": plan.untouched,
        "blocking": plan.blocking,
        "post_apply_actions": plan.post_apply_actions,
    })
}

fn render_plan(plan: &IntegrationChangePlan, dry_run: bool) -> String {
    let mut out = String::new();
    if dry_run {
        out.push_str("Plan (dry run — nothing written)\n\n");
    }
    for c in &plan.changes {
        out.push_str(&format!(
            "{:<9} {:<13} {:<15} {:<34} {}\n",
            c.action.as_str().to_uppercase(),
            c.kind.as_str(),
            c.scope.as_str(),
            c.target.as_deref().unwrap_or("-"),
            c.detail
        ));
    }
    if !plan.untouched.is_empty() {
        out.push_str(&format!("UNCHANGED {}\n", plan.untouched.join(", ")));
    }
    for b in &plan.blocking {
        out.push_str(&format!(
            "\nblocked: {} {} — {}\n",
            b.agent, b.kind, b.detail
        ));
        for step in &b.manual_sequence {
            out.push_str(&format!("  {step}\n"));
        }
    }
    for a in &plan.post_apply_actions {
        out.push_str(&format!("\n{}\n  then: {}\n", a.instruction, a.verify_with));
    }
    out
}

// ---------------------------------------------------------------- doctor ---

/// `cairn doctor` — read-only integration health (FR-166–FR-171).
pub async fn doctor(agent: Option<AgentId>) -> Result<Output, WireError> {
    let env = env();
    let mut snap = snapshot().await?;
    let owed = boundary_owed().await;
    let status = client::send(&Request::Status { cwd: cwd() }).await.ok();
    // Sync degradation belongs in doctor: it is a condition of the
    // installation, it resolves without the developer doing anything, and
    // discovering it only through a count in `sync status` would make retained
    // work look like a queue that stopped moving (FR-415, FR-499).
    let sync = client::send(&Request::SyncStatus { cwd: cwd() }).await.ok();

    // Doctor is where an agent upgrade is noticed. Re-recording each connected
    // agent's detected version discards observation evidence that belonged to
    // the build it replaced, and keeps introspection evidence, which is a fact
    // about Cairn's own artifact and does not age with the vendor (FR-245,
    // SC-138). Without this a developer could upgrade past a removed vendor
    // event and keep being told the integration is FULL.
    let connected: Vec<AgentId> = snap.installs.iter().map(|r| r.agent).collect();
    let mut refreshed = false;
    for a in AgentId::ALL {
        if !connected.contains(&a) {
            continue;
        }
        let state = assess(&env, &snap, owed, a).await?;
        if state.detection.detected {
            record_agent(&state).await?;
            refreshed = true;
        }
    }
    if refreshed {
        snap = snapshot().await?;
    }

    let mut agents = Vec::new();
    let mut text = String::new();
    let mut healthy = 0usize;
    let mut actionable = 0usize;

    let core = json!({
        "cli_version": env!("CARGO_PKG_VERSION"),
        "daemon_version": status.as_ref().and_then(|s| s["version"].as_str()),
        "versions_aligned": status
            .as_ref()
            .and_then(|s| s["version"].as_str())
            .map(|v| v == env!("CARGO_PKG_VERSION"))
            .unwrap_or(false),
        "daemon_reachable": status.is_some(),
        "local_schema_version": status.as_ref().and_then(|s| s["local_schema_version"].as_i64()),
        "project_registered": status.is_some(),
        "sessions_awaiting_handoff": status
            .as_ref()
            .and_then(|s| s["sessions_awaiting_handoff"].as_i64())
            .unwrap_or(0),
        "handoff_synthesis_failures": status
            .as_ref()
            .map(|s| s["handoff_synthesis_failures"].clone())
            .unwrap_or(json!([])),
        "sync_degradation": sync
            .as_ref()
            .map(|s| s["degradation"].clone())
            .unwrap_or(json!(null)),
    });
    text.push_str(&format!(
        "core        cli {} · daemon {} · schema {} · {}\n\n",
        env!("CARGO_PKG_VERSION"),
        core["daemon_version"].as_str().unwrap_or("unreachable"),
        core["local_schema_version"]
            .as_i64()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".into()),
        if status.is_some() {
            "project registered"
        } else {
            "daemon unreachable"
        }
    ));
    if let Some(d) = core["sync_degradation"].as_object() {
        text.push_str(&format!(
            "sync        {} item(s) retained for a server that cannot hold them yet\n\
                          waiting for: {}\n\
                          {}\n\n",
            d.get("blocked").and_then(|v| v.as_i64()).unwrap_or(0),
            d.get("missing_capabilities")
                .and_then(|v| v.as_array())
                .map(|a| a
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default(),
            d.get("note").and_then(|v| v.as_str()).unwrap_or_default(),
        ));
    }

    let targets: Vec<AgentId> = match agent {
        Some(a) => vec![a],
        None => AgentId::ALL.into_iter().collect(),
    };

    for a in targets {
        let state = assess(&env, &snap, owed, a).await?;
        if !state.detection.detected && a != AgentId::GenericMcp {
            continue;
        }
        text.push_str(&format!("{:<12} {}\n", a.as_str(), level_line(&state)));
        text.push_str(&format!(
            "             continuity: {}\n",
            continuity_note(state.profile.continuity_mode())
        ));
        // How sessions here actually end. A developer who reads nothing else
        // should still not believe a session is being completed when it is
        // being timed out (FR-229).
        if let Some(note) = &state.outcome.completion_note {
            text.push_str(&format!("             {note}\n"));
        }
        for b in &state.outcome.awaited_behaviors {
            text.push_str(&format!("             awaiting: {b}\n"));
        }
        for b in &state.profile.conditional_behaviors() {
            text.push_str(&format!("             {b}\n"));
        }

        let mut resources = Vec::new();
        for o in &state.observed {
            let manager_owned = manager_owns(&env, &snap, a, o.kind);
            let condition = if manager_owned {
                // The manager's binding is the authority for a resource the
                // manager installed; comparing it to Cairn's own canonical
                // entry reports a difference that is not a fault.
                HealthCondition::Healthy
            } else {
                migration_aware(&snap, a, o)
            };
            let owner = if manager_owned {
                ResourceOwner::Manager
            } else {
                o.owner
            };
            if condition.is_acceptable() {
                healthy += 1;
            } else {
                actionable += 1;
            }
            let serves: Vec<String> = snap
                .installs
                .iter()
                .find(|r| r.kind == o.kind && Some(&r.location) == o.location.as_ref())
                .map(|r| r.serves.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            let mut entry = json!({
                "kind": o.kind.as_str(),
                "condition": condition.as_str(),
                "owner": owner.as_str(),
                "scope": o.scope.as_str(),
                "location": o.location.as_ref().map(|p| p.display().to_string()),
                "detail": if manager_owned { None } else { o.detail.clone() },
                "remedy": if manager_owned { None } else { o.remedy.clone() },
            });
            if serves.len() > 1 {
                entry["serves"] = json!(serves);
            }
            if let (Some(schema), Some(rev)) = (o.installed_schema, o.installed_revision.clone()) {
                entry["installed"] = json!({ "schema": schema, "revision": rev });
            }
            text.push_str(&format!(
                "  {:<13} {:<24} {:<8} {:<15} {}\n",
                o.kind.as_str(),
                condition.as_str(),
                owner.as_str(),
                o.scope.as_str(),
                o.location
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
            if let Some(r) = &o.remedy {
                if !manager_owned {
                    text.push_str(&format!("                {r}\n"));
                }
            }
            resources.push(entry);
        }
        let mut v = agent_json(&state);
        v["resources"] = json!(resources);
        v["evidence"] = evidence_json(&snap, a);
        agents.push(v);
        text.push('\n');
    }

    // The manager section: what it holds, which applications those resources
    // are bound to, and whether that matches Cairn's record (FR-169).
    let manager = manager_health(&env, &snap, &mut text, &mut healthy, &mut actionable);

    let data = json!({
        "core": core,
        "agents": agents,
        "manager": manager,
        "summary": { "healthy": healthy, "actionable": actionable, "blocking": 0 },
    });
    let mut out = Output::with(data, text);
    // Exit 0 when every reported condition is healthy, shared or unknown;
    // otherwise 1 (FR-170).
    out.exit_nonzero = actionable > 0;
    Ok(out)
}

/// While a migration record exists, the resource is `migrating` and never
/// `duplicated` or `conflicting_owner` (FR-228).
fn migration_aware(snap: &Snapshot, agent: AgentId, o: &Observed) -> HealthCondition {
    if snap.migration_for(agent, o.kind).is_some() {
        return HealthCondition::Migrating;
    }
    o.condition
}

fn evidence_json(snap: &Snapshot, agent: AgentId) -> Value {
    let mut map = serde_json::Map::new();
    for e in snap.evidence.get(&agent).cloned().unwrap_or_default() {
        let mut entry = json!({ "kind": e.kind.as_str(), "agent_version": e.agent_version });
        if let Some(d) = e.degraded {
            entry["degraded"] = json!(d);
        }
        map.insert(e.capability, entry);
    }
    Value::Object(map)
}

/// Whether CC Switch, not Cairn, put this resource in the agent's own
/// configuration.
///
/// Cairn assumes `Direct` whenever it holds no record, which after a manager
/// import is wrong in a way that does damage: the entry CC Switch wrote is a
/// superset of Cairn's canonical one for some targets -- CC Switch adds
/// `type = "stdio"` for Codex -- so it was reported `modified` with the remedy
/// `cairn repair --force`, which would overwrite the manager's entry and
/// silently take ownership back. Absence of a direct record plus presence in
/// the target's config means the manager owns it; there is no third writer.
fn manager_owns(env: &Env, snap: &Snapshot, agent: AgentId, kind: ResourceKind) -> bool {
    let manager = CcSwitch;
    if !manager.detect(env).detected || !manager.distributable().contains(&kind) {
        return false;
    }
    if snap
        .installs
        .iter()
        .any(|r| r.agent == agent && r.kind == kind && r.owner == ResourceOwner::Direct)
    {
        return false;
    }
    let Some(app) = manager
        .target_apps()
        .iter()
        .find(|a| agent_for_app(a) == Some(agent))
    else {
        return false;
    };
    manager
        .inspect_bindings(env, &[(*app).to_string()], kind)
        .iter()
        .any(|b| b.condition.is_acceptable())
}

/// The agent behind a CC Switch target application name.
fn agent_for_app(app: &str) -> Option<AgentId> {
    match app {
        "claude" => Some(AgentId::ClaudeCode),
        "codex" => Some(AgentId::Codex),
        "opencode" => Some(AgentId::Opencode),
        _ => None,
    }
}

fn manager_health(
    env: &Env,
    snap: &Snapshot,
    text: &mut String,
    healthy: &mut usize,
    actionable: &mut usize,
) -> Value {
    let manager = CcSwitch;
    let detection = manager.detect(env);
    if !detection.detected {
        return Value::Null;
    }
    let apps: Vec<String> = manager
        .target_apps()
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut resources = Vec::new();
    let mut consistent = true;
    text.push_str(&format!(
        "cc-switch    detected {}\n",
        detection.version.as_deref().unwrap_or("(version unknown)")
    ));
    for kind in manager.distributable() {
        // Presence in the target application's own configuration is the
        // evidence, not a record of ours (FR-234).
        //
        // This used to require an existing install recorded with
        // `owner == Manager` before it would look. Nothing ever records that:
        // `distribute` returns `manager_action_required` and records nothing,
        // because at that point the user has not confirmed the import yet. So
        // the documented sequence -- distribute, confirm inside CC Switch, run
        // `cairn doctor` -- could never report manager ownership, and the
        // manager block stayed empty however many resources CC Switch had
        // actually written.
        //
        // A kind with nothing installed anywhere is skipped rather than
        // reported `Missing` for every target, so a machine that does not use
        // CC Switch to distribute Cairn sees no new findings.
        // Only applications Cairn does not own *directly* can be the
        // manager's. There are two owners and no third, so a Cairn entry in a
        // target's configuration that Cairn did not install itself was put
        // there by the manager. Presence alone cannot tell them apart, which is
        // why the directly-recorded ones are excluded rather than reported
        // twice -- a Skill installed by `cairn connect` is not a CC Switch
        // binding, and listing it as one manufactures the direct + manager dual
        // ownership this is supposed to detect.
        let manager_apps: Vec<String> = apps
            .iter()
            .filter(|app| {
                let Some(agent) = agent_for_app(app) else {
                    return false;
                };
                !snap.installs.iter().any(|r| {
                    r.agent == agent && r.kind == *kind && r.owner == ResourceOwner::Direct
                })
            })
            .cloned()
            .collect();
        if manager_apps.is_empty() {
            continue;
        }
        let bindings = manager.inspect_bindings(env, &manager_apps, *kind);
        if !bindings.iter().any(|b| b.condition.is_acceptable()) {
            continue;
        }
        let mut rows = Vec::new();
        for b in &bindings {
            if b.condition.is_acceptable() {
                *healthy += 1;
            } else {
                *actionable += 1;
                consistent = false;
            }
            rows.push(json!({
                "app": b.app,
                "condition": b.condition.as_str(),
                "detail": b.detail,
                "remedy": b.remedy,
            }));
            text.push_str(&format!(
                "  {:<13} {:<10} {} {}\n",
                kind.as_str(),
                b.app,
                b.condition.as_str(),
                b.detail.as_deref().unwrap_or("")
            ));
        }
        resources.push(json!({ "kind": kind.as_str(), "bindings": rows }));
    }
    json!({
        "manager": ManagerId::CcSwitch.as_str(),
        "detected": true,
        "version": detection.version,
        "resources": resources,
        "ownership_consistent": consistent,
    })
}

// ---------------------------------------------------------------- repair ---

/// `cairn repair` — restore Cairn-owned state only (FR-172–FR-177).
pub async fn repair(opts: &Options) -> Result<Output, WireError> {
    let env = env();
    let snap = snapshot().await?;
    let owed = boundary_owed().await;

    // Repair restores what Cairn *owns*. An agent the developer never
    // connected owns nothing, so a bare `cairn repair` that reached every
    // detected agent would silently connect them — which is the opt-in
    // FR-164 exists to require. Naming one explicitly still repairs it.
    let connected: Vec<AgentId> = detected_agents(&env)
        .into_iter()
        .filter(|a| snap.installs.iter().any(|r| r.agent == *a))
        .collect();
    let targets: Vec<AgentId> = match opts.agent {
        Some(a) => vec![a],
        None => connected,
    };
    if targets.is_empty() {
        return Ok(Output::with(
            json!({ "changes": [], "blocking": [], "connected": [] }),
            "no connected agent to repair; run `cairn connect` first\n".into(),
        ));
    }
    let desired = compose(opts, &targets, &targets, &snap, &[]);

    let mut plan = IntegrationChangePlan::default();
    let mut states = Vec::new();
    for agent in &targets {
        let state = assess(&env, &snap, owed, *agent).await?;
        let mut p = plan_agent(
            Intent::Repair { force: opts.force },
            *agent,
            &desired,
            &state.observed,
        );
        plan.changes.append(&mut p.changes);
        plan.blocking.append(&mut p.blocking);
        plan.post_apply_actions.append(&mut p.post_apply_actions);
        if plan.untouched.is_empty() {
            plan.untouched = p.untouched;
        }
        states.push(state);
    }

    if opts.dry_run {
        return Ok(Output::with(
            plan_json(&plan, true),
            render_plan(&plan, true),
        ));
    }
    if plan.is_noop() {
        // A conflict repair will not resolve is reported, and it is not
        // "nothing to do": there is something to do, and only the developer
        // can decide it. Exiting non-zero is what lets a script notice
        // (FR-174, FR-196).
        if plan.is_blocked() {
            let mut text = String::new();
            for b in &plan.blocking {
                text.push_str(&format!("{} {} — {}\n", b.agent, b.kind, b.detail));
                for step in &b.manual_sequence {
                    text.push_str(&format!("  {step}\n"));
                }
            }
            text.push_str("\nnothing was changed\n");
            let mut out = Output::with(plan_json(&plan, false), text);
            out.exit_nonzero = true;
            return Ok(out);
        }
        return Ok(Output::with(
            plan_json(&plan, false),
            "nothing to do\n".into(),
        ));
    }
    apply_plan(&env, &desired, &plan, &states, opts).await
}

// ------------------------------------------------------------ disconnect ---

/// `cairn disconnect` — remove Cairn-owned integration for one agent.
///
/// Removal is **by binding, not by file** (FR-243): this agent's dependency on
/// each resource is dropped, and the resource itself is deleted only when no
/// other agent is still bound to it.
pub async fn disconnect(agent: AgentId, opts: &Options) -> Result<Output, WireError> {
    let env = env();
    let snap = snapshot().await?;
    let owed = boundary_owed().await;
    let state = assess(&env, &snap, owed, agent).await?;

    let kinds: Vec<ResourceKind> = if opts.only.is_empty() {
        scope::kinds_for(agent)
    } else {
        opts.only.clone()
    };

    if opts.dry_run {
        let desired = compose(opts, &[agent], &[agent], &snap, &[]);
        let plan = plan_agent(Intent::Disconnect, agent, &desired, &state.observed);
        return Ok(Output::with(
            plan_json(&plan, true),
            render_plan(&plan, true),
        ));
    }

    let mut text = String::new();
    let mut removed = Vec::new();
    let mut manager_actions: Vec<ManagerActionRequired> = Vec::new();

    for kind in kinds {
        let recorded = snap
            .installs
            .iter()
            .find(|r| r.agent == agent && r.kind == kind)
            .cloned();

        // A manager-owned resource is never removed by Cairn (FR-149,
        // FR-233). Its record and binding survive so the withdrawal stays
        // verifiable (FR-244).
        if recorded.as_ref().map(|r| r.owner) == Some(ResourceOwner::Manager) {
            let apps = scope::manager_app_for(agent)
                .map(|a| vec![a.to_string()])
                .unwrap_or_default();
            let action = cc_switch::removal_action(kind, &apps);
            text.push_str(&format!(
                "\nmanager action required — CC Switch owns the Cairn {kind} for {agent}\n  {}\n  Then run: {}\n",
                action.instructions, action.verify_with
            ));
            manager_actions.push(action);
            continue;
        }

        let outcome = client::send(&Request::IntegrationUnbind {
            cwd: cwd(),
            agent: agent.as_str().into(),
            kind: kind.as_str().into(),
        })
        .await?;
        match outcome["outcome"].as_str().unwrap_or("nothing") {
            "resource_removed" => {
                let scope = recorded
                    .as_ref()
                    .map(|r| r.scope)
                    .or_else(|| scope::resolve_scope(agent, kind, false, None))
                    .unwrap_or(InstallationScope::User);
                let m = install::materialize_removal(&env, agent, kind, scope, recorded.as_ref())
                    .map_err(|e| err(e.condition().as_str(), e.to_string()))?;
                if m.writes() {
                    install::commit(&m).map_err(|e| err(e.code(), e.to_string()))?;
                }
                text.push_str(&format!(
                    "removed   {:<13} {}\n",
                    kind.as_str(),
                    m.location.display()
                ));
                removed.push(
                    json!({ "kind": kind.as_str(), "target": m.location.display().to_string() }),
                );
            }
            "resource_kept" => {
                let serves: Vec<String> = recorded
                    .as_ref()
                    .map(|r| {
                        r.serves
                            .iter()
                            .filter(|s| **s != agent)
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                text.push_str(&format!(
                    "unbound   {:<13} (kept — still serving {})\n",
                    kind.as_str(),
                    if serves.is_empty() {
                        "another agent".to_string()
                    } else {
                        serves.join(", ")
                    }
                ));
            }
            _ => {}
        }
    }

    // The agent's record goes last, and only once its last binding is gone
    // (FR-244).
    let forgotten = client::send(&Request::IntegrationForgetAgent {
        cwd: cwd(),
        agent: agent.as_str().into(),
    })
    .await?;
    if !manager_actions.is_empty() {
        text.push_str(&format!(
            "\n{agent}'s local record is kept so this withdrawal stays verifiable; it is removed\nonce `cairn doctor {agent}` observes the entry gone.\n"
        ));
    }
    text.push_str("\nMemory, tasks, sessions and handoffs are untouched.\n");

    let data = json!({
        "removed": removed,
        "record_removed": forgotten["removed"],
        "manager_action_required": manager_actions,
    });
    if !manager_actions.is_empty() {
        // The operation did not fully complete (FR-233).
        return Err(err(
            codes::MANAGER_ACTION_REQUIRED,
            format!(
                "CC Switch owns {} Cairn resource(s) for {agent}; withdraw them there, then run `cairn doctor {agent}`",
                manager_actions.len()
            ),
        ));
    }
    Ok(Output::with(data, text))
}

// ------------------------------------------------------------ export mcp ---

/// `cairn integration export mcp` — deterministic, secret-free, writes nothing
/// (FR-131).
pub fn export_mcp(agent: Option<AgentId>, format: Option<&str>) -> Result<Output, WireError> {
    let entry = cairn_integrate::mcp_entry();
    let toml_form = format == Some("toml") || agent == Some(AgentId::Codex);
    let (value, text) = if toml_form {
        let text = format!(
            "[mcp_servers.{}]\ncommand = \"cairn\"\nargs = [\"mcp\"]\n",
            cairn_integrate::MCP_SERVER_NAME
        );
        (json!({ "format": "toml", "config": text.clone() }), text)
    } else {
        let key = if agent == Some(AgentId::Opencode) {
            "mcp"
        } else {
            "mcpServers"
        };
        let body = json!({ key: { cairn_integrate::MCP_SERVER_NAME: entry } });
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        (body, text)
    };
    Ok(Output::with(value, text))
}

// -------------------------------------------------------------- migrate ---

/// `cairn integration migrate` — move one resource between owners (FR-148,
/// FR-228, FR-236, FR-237).
pub async fn migrate(
    agent: AgentId,
    kind: ResourceKind,
    to: ResourceOwner,
    opts: &Options,
    resume: bool,
    abort: bool,
) -> Result<Output, WireError> {
    let env = env();
    let snap = snapshot().await?;
    let existing = snap.migration_for(agent, kind).cloned();

    if abort {
        // Reversing leaves the previously working configuration in place.
        client::send(&Request::IntegrationMigration {
            cwd: cwd(),
            agent: agent.as_str().into(),
            kind: kind.as_str().into(),
            action: MigrationAction::Clear,
            source_owner: None,
            source_scope: None,
            source_location: None,
            target_owner: None,
            target_scope: None,
            target_location: None,
            overlap_permitted: false,
            phase: None,
            last_error: None,
        })
        .await?;
        return Ok(Output::with(
            json!({ "migration": null, "aborted": true }),
            "migration abandoned; the previously working configuration is intact\n".into(),
        ));
    }

    let recorded = snap
        .installs
        .iter()
        .find(|r| r.agent == agent && r.kind == kind)
        .cloned()
        .ok_or_else(|| {
            err(
                codes::NOT_FOUND,
                format!("Cairn has no recorded {kind} for {agent} to migrate"),
            )
        })?;

    if existing.is_some() && !resume {
        return Err(err(
            codes::MIGRATION_IN_PROGRESS,
            format!("a migration for {agent} {kind} is already running; use --resume or --abort"),
        ));
    }

    let app = scope::manager_app_for(agent).unwrap_or("claude");
    let target_scope =
        scope::resolve_scope(agent, kind, false, None).unwrap_or(InstallationScope::User);

    // Where both owners write one effective slot, bounded overlap cannot be
    // unambiguous, so automatic migration is refused and the manual sequence
    // is printed (FR-148, D38).
    let same_slot = scope::shares_effective_slot(&env, agent, kind, target_scope, app);
    if same_slot && to == ResourceOwner::Manager {
        let text = format!(
            "cannot migrate automatically\n\n  CC Switch writes the same file at the same scope as Cairn's direct entry,\n  so the two cannot coexist unambiguously for the duration of the migration.\n\n  Safe sequence:\n    1. cairn integration migrate {agent} {kind} --to cc-switch --dry-run\n    2. cairn disconnect {agent} --only {kind}\n    3. cairn integration distribute --via cc-switch --resource {kind} --apps {app}\n    4. cairn doctor {agent}\n"
        );
        if opts.dry_run {
            return Ok(Output::with(json!({ "migration_unsafe": true }), text));
        }
        return Err(err(codes::MIGRATION_UNSAFE, text));
    }

    if opts.dry_run {
        return Ok(Output::with(
            json!({
                "dry_run": true,
                "agent": agent.as_str(),
                "kind": kind.as_str(),
                "from": recorded.owner.as_str(),
                "to": to.as_str(),
                "overlap_permitted": !same_slot,
            }),
            format!(
                "migrate {agent} {kind}: {} → {} (target verified before the source is removed)\n",
                recorded.owner, to
            ),
        ));
    }

    // planned → target_installed → target_verified → source_removed.
    if existing.is_none() {
        client::send(&Request::IntegrationMigration {
            cwd: cwd(),
            agent: agent.as_str().into(),
            kind: kind.as_str().into(),
            action: MigrationAction::Start,
            source_owner: Some(recorded.owner.as_str().into()),
            source_scope: Some(recorded.scope.as_str().into()),
            source_location: Some(recorded.location.display().to_string()),
            target_owner: Some(to.as_str().into()),
            target_scope: Some(target_scope.as_str().into()),
            target_location: Some(
                scope::location(&env, agent, kind, target_scope)
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ),
            overlap_permitted: !same_slot,
            phase: None,
            last_error: None,
        })
        .await?;
    }

    match to {
        // Direct → manager is automated up to the manager's own confirmation
        // boundary; Cairn verifies the result before removing what it owns
        // (FR-236).
        ResourceOwner::Manager => {
            let uri = CcSwitch
                .import_uri(kind, &[app.to_string()])
                .map_err(import_error)?;
            advance(agent, kind, "target_installed").await?;
            let action = cc_switch::import_action(kind, &[app.to_string()], uri);
            Err(err(
                codes::MANAGER_ACTION_REQUIRED,
                format!(
                    "confirm the import in CC Switch, then run `cairn doctor {agent}` — {}",
                    action.uri.unwrap_or_default()
                ),
            ))
        }
        // Manager → direct never deletes the manager's resource behind its
        // back (FR-237).
        ResourceOwner::Direct => {
            let m = install::materialize_install(&env, agent, kind, target_scope)
                .map_err(|e| err(e.condition().as_str(), e.to_string()))?;
            install::commit(&m).map_err(|e| err(e.code(), e.to_string()))?;
            advance(agent, kind, "target_installed").await?;

            // Verify in the agent's real configuration before going further.
            let observed = adapter_for(agent).inspect(&env, &snap.for_agent(agent));
            let ok = observed
                .iter()
                .any(|o| o.kind == kind && o.condition.is_acceptable());
            if !ok {
                fail(
                    agent,
                    kind,
                    "the installed target was not observed as effective",
                )
                .await?;
                return Err(err(
                    codes::VERIFICATION_FAILED,
                    "the target was written but not observed as effective; the source is intact",
                ));
            }
            advance(agent, kind, "target_verified").await?;
            record_resource(agent, &m).await?;

            let apps = vec![app.to_string()];
            let action = cc_switch::removal_action(kind, &apps);
            Err(err(
                codes::MANAGER_ACTION_REQUIRED,
                format!(
                    "the direct {kind} is installed and verified; withdraw the manager's copy — {} — then run `{}`",
                    action.instructions, action.verify_with
                ),
            ))
        }
        ResourceOwner::External => Err(err(
            codes::INVALID_REQUEST,
            "external is not a migration target; Cairn neither adopts nor manages it",
        )),
    }
}

async fn advance(agent: AgentId, kind: ResourceKind, phase: &str) -> Result<(), WireError> {
    client::send(&Request::IntegrationMigration {
        cwd: cwd(),
        agent: agent.as_str().into(),
        kind: kind.as_str().into(),
        action: MigrationAction::Advance,
        source_owner: None,
        source_scope: None,
        source_location: None,
        target_owner: None,
        target_scope: None,
        target_location: None,
        overlap_permitted: false,
        phase: Some(phase.into()),
        last_error: None,
    })
    .await
    .map(|_| ())
}

async fn fail(agent: AgentId, kind: ResourceKind, why: &str) -> Result<(), WireError> {
    client::send(&Request::IntegrationMigration {
        cwd: cwd(),
        agent: agent.as_str().into(),
        kind: kind.as_str().into(),
        action: MigrationAction::Fail,
        source_owner: None,
        source_scope: None,
        source_location: None,
        target_owner: None,
        target_scope: None,
        target_location: None,
        overlap_permitted: false,
        phase: None,
        last_error: Some(why.into()),
    })
    .await
    .map(|_| ())
}

fn import_error(e: cairn_integrate::adapter::ImportRefusal) -> WireError {
    use cairn_integrate::adapter::ImportRefusal;
    match e {
        ImportRefusal::UnpublishedSkillRef { revision, manual } => err(
            codes::UNPUBLISHED_SKILL_REF,
            format!(
                "this build's Skill revision {revision} has no published skill-release branch, and \
                 emitting an unpublished ref would make CC Switch silently install `main`. {manual}"
            ),
        ),
        ImportRefusal::NotDistributable(kind) => err(
            codes::INVALID_REQUEST,
            format!("CC Switch does not distribute {kind}"),
        ),
    }
}

// ----------------------------------------------------------- distribute ---

/// `cairn integration distribute` — initiate the manager's documented import
/// flow, then stop (FR-233).
///
/// It never reports success before verification; `cairn doctor` performs that
/// afterwards (FR-234).
pub async fn distribute(
    manager: ManagerId,
    resource: ResourceKind,
    apps: Vec<String>,
    opts: &Options,
) -> Result<Output, WireError> {
    let env = env();
    let _ = manager;
    let m = CcSwitch;
    if !m.detect(&env).detected {
        return Err(err(
            codes::AGENT_NOT_DETECTED,
            "CC Switch is not installed on this machine",
        ));
    }
    let apps = if apps.is_empty() {
        m.target_apps().iter().map(|s| s.to_string()).collect()
    } else {
        apps
    };
    let uri = m.import_uri(resource, &apps).map_err(import_error)?;

    if opts.dry_run {
        return Ok(Output::with(
            json!({ "dry_run": true, "uri": uri, "applications": apps }),
            format!("would open: {uri}\n"),
        ));
    }

    let action = cc_switch::import_action(resource, &apps, uri.clone());
    // Cairn opens the link, or prints it when it cannot. The confirmation
    // dialog belongs to CC Switch and Cairn does not attempt to pass it.
    let opened = open_uri(&uri);
    let text = format!(
        "{}\n{}\n\nCC Switch will ask you to confirm. Then run: {}\n",
        if opened {
            "opened the import link in CC Switch"
        } else {
            "open this link in CC Switch:"
        },
        if opened { String::new() } else { uri.clone() },
        action.verify_with
    );
    // The operation has not completed (FR-233).
    Err(err(
        codes::MANAGER_ACTION_REQUIRED,
        format!("awaiting your confirmation inside CC Switch. {text}"),
    ))
}

fn open_uri(uri: &str) -> bool {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(uri)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_002_code_exits_one() {
        // contracts/integration-cli.md §Error codes: a closed set, all exit 1,
        // with the two unavailability codes unchanged at exit 2.
        for code in codes::INTEGRATION_CODES {
            let e = WireError::new(code, "x");
            assert_eq!(crate::exit_code(&e), 1, "{code} does not exit 1");
        }
        assert_eq!(
            crate::exit_code(&WireError::new(codes::DAEMON_UNAVAILABLE, "x")),
            2
        );
        assert_eq!(
            crate::exit_code(&WireError::new(codes::STORAGE_UNAVAILABLE, "x")),
            2
        );
    }

    #[test]
    fn the_exported_configuration_is_deterministic_and_secret_free() {
        // FR-131, SC-135.
        let a = export_mcp(None, None).unwrap();
        let b = export_mcp(None, None).unwrap();
        assert_eq!(a.text, b.text);
        for word in ["token", "secret", "key", "password"] {
            assert!(!a.text.to_lowercase().contains(word));
        }
        assert!(a.text.contains("mcpServers"));
        // Codex takes TOML, and asking for it explicitly works too.
        assert!(export_mcp(Some(AgentId::Codex), None)
            .unwrap()
            .text
            .contains("[mcp_servers.cairn]"));
        assert!(export_mcp(Some(AgentId::Opencode), None)
            .unwrap()
            .text
            .contains("\"mcp\""));
    }
}

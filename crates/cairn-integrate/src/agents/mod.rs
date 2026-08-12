//! The four agent adapters, and the inspection they share.
//!
//! Each adapter knows one agent's configuration surfaces and lifecycle
//! vocabulary. Everything below is shared: checking whether Cairn's MCP entry
//! is present and equal, whether the managed block is current, whether the
//! Skill on disk is the Skill this build embeds. A bug in any of it is fixed
//! once.

pub mod claude_code;
pub mod codex;
pub mod generic_mcp;
pub mod opencode;

use crate::adapter::Observed;
use crate::edit::{json, EditError};
use crate::markers::{self, CONTRACT_ID};
use crate::model::{AgentId, HealthCondition, InstallationScope, ResourceKind, ResourceOwner};
use crate::plan::RecordedInstall;
use crate::{render, revision};
use cairn_core::lifecycle::CanonicalLifecycleEvent;
use cairn_core::tools::{classify_tool, is_test_command, normalize_vendor_tool};
use cairn_core::wire::ObservationInput;
use serde_json::Value;
use std::path::Path;

/// Read a file, treating absence as empty.
pub(crate) fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Which agents, other than this one, are bound to the same resource.
pub(crate) fn serves(record: &[RecordedInstall], kind: ResourceKind, path: &Path) -> Vec<AgentId> {
    record
        .iter()
        .filter(|r| r.kind == kind && r.location == path)
        .flat_map(|r| r.serves.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Inspect Cairn's MCP entry inside a JSON or JSONC configuration file.
pub(crate) fn inspect_mcp_json(
    path: &Path,
    keys: &[&str],
    scope: InstallationScope,
    recorded: Option<&RecordedInstall>,
) -> Observed {
    let display = path.display().to_string();
    let text = read(path);
    let found = match json::get(&display, &text, keys) {
        Ok(v) => v,
        Err(e) => return malformed(ResourceKind::Mcp, scope, path, &e),
    };
    classify_entry(
        ResourceKind::Mcp,
        scope,
        path,
        found,
        recorded,
        crate::mcp_entry(),
    )
}

/// Compare a found entry against Cairn's canonical one.
pub(crate) fn classify_entry(
    kind: ResourceKind,
    scope: InstallationScope,
    path: &Path,
    found: Option<Value>,
    recorded: Option<&RecordedInstall>,
    canonical: Value,
) -> Observed {
    let base = |c: HealthCondition| {
        Observed::new(kind, c)
            .at(scope, Some(path.to_path_buf()))
            .owned_by(recorded.map(|r| r.owner).unwrap_or(ResourceOwner::Direct))
    };
    match found {
        None => {
            if recorded.is_some() {
                base(HealthCondition::Missing)
                    .detail("recorded as installed, not found")
                    .remedy("cairn repair")
            } else {
                base(HealthCondition::Missing).detail("not installed")
            }
        }
        Some(value) => {
            // A manager-owned entry carries no content hash: Cairn did not
            // write the bytes, so it verifies presence and effectiveness
            // rather than equality (FR-234).
            if recorded.map(|r| r.owner) == Some(ResourceOwner::Manager) {
                return base(HealthCondition::Healthy)
                    .detail("present, distributed by the manager");
            }
            if value == canonical {
                base(HealthCondition::Healthy)
            } else {
                base(HealthCondition::Modified)
                    .detail("the entry differs from Cairn's canonical form")
                    .remedy("cairn repair --force")
            }
        }
    }
}

/// Inspect the managed instruction block in a Markdown file.
pub(crate) fn inspect_instructions(
    path: &Path,
    scope: InstallationScope,
    record: &[RecordedInstall],
    recorded: Option<&RecordedInstall>,
) -> Observed {
    let display = path.display().to_string();
    let text = read(path);
    let contract = render::Contract::canonical();
    let want = contract.version();

    let block = match markers::find(&text, CONTRACT_ID) {
        Ok(b) => b,
        Err(markers::MarkerError::Damaged(detail)) => {
            return Observed::new(ResourceKind::Instructions, HealthCondition::DamagedMarkers)
                .at(scope, Some(path.to_path_buf()))
                .detail(detail)
                .remedy("restore or remove the damaged markers by hand, then re-run doctor");
        }
    };
    let _ = display;

    let owner = recorded.map(|r| r.owner).unwrap_or(ResourceOwner::Direct);
    let base = |c: HealthCondition| {
        Observed::new(ResourceKind::Instructions, c)
            .at(scope, Some(path.to_path_buf()))
            .owned_by(owner)
    };

    let Some(block) = block else {
        return base(HealthCondition::Missing).detail("no cairn:managed block in this file");
    };

    let consumers = serves(record, ResourceKind::Instructions, path);
    let shared = consumers.len() > 1;

    if block.schema != want.schema || block.content != want.revision {
        if !block.self_consistent() {
            // The marker's digest no longer describes the body it wraps:
            // someone edited inside the markers (FR-177).
            return base(HealthCondition::Modified)
                .version(block.schema, block.content.clone())
                .detail("the managed block was edited by hand")
                .remedy("cairn repair --force");
        }
        return base(HealthCondition::Outdated)
            .version(block.schema, block.content.clone())
            .detail(format!(
                "Cairn's managed block is behind this build (schema {}, revision {}→{})",
                block.schema, block.content, want.revision
            ))
            .remedy("cairn repair");
    }
    if !block.matches_body(&contract.block_body()) {
        return base(HealthCondition::Modified)
            .version(block.schema, block.content.clone())
            .detail("the managed block was edited by hand")
            .remedy("cairn repair --force");
    }
    let condition = if shared {
        HealthCondition::Shared
    } else {
        HealthCondition::Healthy
    };
    let mut o = base(condition).version(block.schema, block.content.clone());
    if shared {
        o = o.detail(format!(
            "one managed block, {} bindings; disconnecting either agent keeps the block",
            consumers.len()
        ));
    }
    o
}

/// Inspect an installed Skill directory.
///
/// Doctor recomputes the revision from the installed files rather than
/// trusting the metadata: a `SKILL.md` claiming one revision over different
/// content is exactly the drift this comparison exists to catch (T045).
pub(crate) fn inspect_skill(
    path: &Path,
    scope: InstallationScope,
    record: &[RecordedInstall],
    recorded: Option<&RecordedInstall>,
) -> Observed {
    let owner = recorded.map(|r| r.owner).unwrap_or(ResourceOwner::Direct);
    let base = |c: HealthCondition| {
        Observed::new(ResourceKind::Skill, c)
            .at(scope, Some(path.to_path_buf()))
            .owned_by(owner)
    };
    if !path.join("SKILL.md").exists() {
        return base(HealthCondition::Missing).detail("not installed");
    }
    let installed = match revision::installed_revision(path) {
        Ok(i) => i,
        Err(e) => return base(HealthCondition::Unknown).detail(format!("could not be read: {e}")),
    };
    // A Skill named `cairn` that Cairn does not own is never overwritten
    // (FR-143).
    if recorded.is_none() && !installed.self_consistent() {
        return base(HealthCondition::ConflictingOwner)
            .owned_by(ResourceOwner::External)
            .detail("a Skill named `cairn` is installed here that Cairn did not write")
            .remedy("cairn doctor  # Cairn will neither adopt nor delete it");
    }

    let (schema, rev) = (revision::embedded_schema(), revision::embedded_revision());
    if !installed.matches(schema, &rev) {
        return base(HealthCondition::Outdated)
            .version(installed.schema, installed.computed_revision.clone())
            .detail(format!(
                "installed Skill is schema {} revision {}; this build carries schema {schema} revision {rev}",
                installed.schema, installed.computed_revision
            ))
            .remedy("cairn repair");
    }
    let consumers = serves(record, ResourceKind::Skill, path);
    let mut o = base(if consumers.len() > 1 {
        HealthCondition::Shared
    } else {
        HealthCondition::Healthy
    })
    .version(installed.schema, installed.computed_revision.clone());
    if consumers.len() > 1 {
        o = o.detail(format!(
            "one installed Skill, {} bindings; a second copy would collide on skill name",
            consumers.len()
        ));
    }
    o
}

pub(crate) fn malformed(
    kind: ResourceKind,
    scope: InstallationScope,
    path: &Path,
    e: &EditError,
) -> Observed {
    Observed::new(kind, e.condition())
        .at(scope, Some(path.to_path_buf()))
        .detail(e.to_string())
        .remedy("fix the file by hand; Cairn will not rewrite a file it cannot parse")
}

/// Build a success observation from a vendor tool payload.
///
/// Only allow-listed fields are read; everything else is used for routing and
/// discarded (FR-198, FR-199, D35).
pub(crate) fn tool_observation(
    tool: &str,
    input: Option<&Value>,
    exit_code: Option<i64>,
    failed: bool,
    failure_detail: Option<String>,
) -> ObservationInput {
    let path = input
        .and_then(|v| v.get("file_path"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let command = input
        .and_then(|v| v.get("command"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let kind = if failed {
        cairn_core::domain::ObservationType::Error
    } else {
        match &command {
            Some(c) if is_test_command(c) => cairn_core::domain::ObservationType::TestRun,
            Some(_) => cairn_core::domain::ObservationType::CommandRun,
            None => classify_tool(tool),
        }
    };
    let summary = if failed {
        match (&path, &command) {
            (_, Some(c)) => format!("{tool} failed: {c}"),
            (Some(p), _) => format!("{tool} failed: {p}"),
            _ => format!("{tool} failed"),
        }
    } else {
        match (&path, &command) {
            (_, Some(c)) => format!("{tool}: {c}"),
            (Some(p), _) => format!("{tool}: {p}"),
            _ => tool.to_string(),
        }
    };

    ObservationInput {
        kind,
        path,
        command,
        exit_code,
        outcome: if failed {
            Some("error".into())
        } else if kind == cairn_core::domain::ObservationType::TestRun {
            Some(if exit_code.unwrap_or(0) == 0 {
                "passed".into()
            } else {
                "failed".into()
            })
        } else {
            None
        },
        summary,
        details: failure_detail.map(Value::String),
        vendor_tool: normalize_vendor_tool(tool),
    }
}

/// Every adapter routes by the vendor's own session identifier. An event with
/// none cannot be routed and is declined (FR-118).
pub(crate) fn session_key(payload: &crate::RawPayload, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| payload.str(k))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Helper for adapters: build a canonical event with the common fields.
pub(crate) fn canonical(
    kind: cairn_core::lifecycle::CanonicalEvent,
    agent: AgentId,
    key: String,
    payload: &crate::RawPayload,
) -> CanonicalLifecycleEvent {
    CanonicalLifecycleEvent::new(kind, agent.as_str(), key, payload.cwd.clone())
}

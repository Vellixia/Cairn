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
use cairn_core::event::{
    ChangeKind, DeclineReason, EventAgent, EventContent, EventKind, FailureKind, FileIdentity,
    ResourceKind as ResearchResource, TestOutcome, ToolClass,
};
use cairn_core::lexicon::{map_semantic_signal, SourceRole};
use cairn_core::lifecycle::CanonicalLifecycleEvent;
use cairn_core::redact::redact;
use cairn_core::tools::{classify_tool, is_test_command, normalize_vendor_tool};
use cairn_core::validate::SafeEventField;
use cairn_core::vocabulary::SessionVocabulary;
use cairn_core::wire::ObservationInput;
use serde_json::Value;
use std::collections::BTreeMap;
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
    canonical: serde_json::Value,
) -> Observed {
    let display = path.display().to_string();
    let text = read(path);
    let found = match json::get(&display, &text, keys) {
        Ok(v) => v,
        Err(e) => return malformed(ResourceKind::Mcp, scope, path, &e),
    };
    classify_entry(ResourceKind::Mcp, scope, path, found, recorded, canonical)
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
        // The reason belongs in the summary, not only in the details: Feature
        // 001's handoff reads failures from it, and a failure the developer
        // cannot recognise is not a useful record (FR-033, US2 #5).
        let reason = failure_detail.as_deref().unwrap_or("tool execution failed");
        match (&path, &command) {
            (_, Some(c)) => format!("{tool} failed: {c}: {reason}"),
            (Some(p), _) => format!("{tool} failed: {p}: {reason}"),
            _ => format!("{tool} failed: {reason}"),
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
            // Read, never inferred. A vendor that reports no exit code has
            // told Cairn nothing about the run, and recording that as a pass
            // would put "tests green" in a handoff on no evidence at all
            // (FR-117, and Feature 001's own `unknown` default).
            Some(match exit_code {
                Some(0) => "passed".into(),
                Some(_) => "failed".into(),
                None => "unknown".to_string(),
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

// ---------------------------------------------------------------------------
// Feature 005 capture (T046, `contracts/safe-events.md`, `contracts/extraction.md`)
// ---------------------------------------------------------------------------

/// Which vendor fields feed which canonical content, declared per agent.
///
/// This replaces the two-field allowlist above. The allowlist was one pair of
/// keys read from every vendor's `tool_input`, which worked while the only
/// question was "did a tool touch a file or run a command" — and cannot express
/// the twenty-one canonical kinds, because different vendors put the same fact
/// in differently-named places and some of them put it nowhere.
///
/// A **typed map, per vendor**, rather than a wider shared allowlist, because a
/// shared allowlist is a claim that every vendor spells everything the same
/// way. Naming each field per agent is also what makes the capture matrix
/// checkable: a cell is supported when this map names a field for it, and
/// declined or unimplemented when it does not — never silently absent (FR-728,
/// SC-706).
///
/// Every entry is a list of alternatives tried in order, because vendors rename
/// fields between versions and an adapter that knew only the current name would
/// go quiet rather than fail.
pub struct FieldMap {
    pub agent: EventAgent,
    /// The vendor's own session identifier. Routing only; never transmitted.
    pub session_keys: &'static [&'static str],
    pub tool_name: &'static [&'static str],
    pub tool_input: &'static [&'static str],
    pub tool_response: &'static [&'static str],
    /// Inside `tool_input`: the file a file-touching tool acted on.
    pub input_file_path: &'static [&'static str],
    /// Inside `tool_input`: the command an executing tool ran.
    pub input_command: &'static [&'static str],
    /// Inside `tool_response`: the process exit status.
    pub response_exit_status: &'static [&'static str],
    /// Inside `tool_response`: the vendor's own failure description.
    pub response_error: &'static [&'static str],
    pub open_trigger: &'static [&'static str],
    pub compaction_trigger: &'static [&'static str],
    pub close_reason: &'static [&'static str],
    /// The user-prompt field. Emits `user_instruction_signal` (§13.10).
    pub user_prompt: &'static [&'static str],
    /// The **settled** assistant-message field. Emits `decision_signal`.
    pub assistant_message: &'static [&'static str],
    pub subagent_ref: &'static [&'static str],
    pub subagent_kind: &'static [&'static str],
}

/// What the local machine knows that a pure payload parse cannot.
///
/// The repository root is machine configuration and never crosses the boundary
/// (FR-753); it is here so an absolute path can be *relativized* locally rather
/// than transmitted or discarded. The vocabulary is here because a semantic
/// signal's tokens must be justified against evidence already in the event
/// stream, and only the daemon holds that stream — so the hook is handed the
/// derived set rather than being trusted to invent one.
pub struct CaptureEnv<'a> {
    pub repo_root: Option<&'a Path>,
    pub vocabulary: &'a SessionVocabulary,
    /// Normalized `topic_key` → normalized `value_key` for keys this project
    /// already establishes. Supplies the §13.5 fallback object; empty simply
    /// means the fallback is unavailable.
    pub established_values: &'a BTreeMap<String, String>,
}

impl Default for CaptureEnv<'_> {
    fn default() -> Self {
        static EMPTY_VOCAB: std::sync::OnceLock<SessionVocabulary> = std::sync::OnceLock::new();
        static EMPTY_VALUES: std::sync::OnceLock<BTreeMap<String, String>> =
            std::sync::OnceLock::new();
        CaptureEnv {
            repo_root: None,
            vocabulary: EMPTY_VOCAB.get_or_init(SessionVocabulary::new),
            established_values: EMPTY_VALUES.get_or_init(BTreeMap::new),
        }
    }
}

// The three types below are `cairn-core`'s, re-exported rather than redefined.
//
// They are the shape that crosses the capture-process boundary, so the wire
// request that carries them and the adapter that builds them must be talking
// about the same type — two structurally identical definitions would compile
// and would let one side gain a field the other silently dropped.
pub use cairn_core::event::{CaptureDecline, CaptureOutput, SafeEventDraft};

/// Build a draft with the fields every kind shares.
pub(crate) fn draft(
    map: &FieldMap,
    kind: EventKind,
    vendor_event: &str,
    content: Option<EventContent>,
) -> SafeEventDraft {
    SafeEventDraft {
        kind,
        agent: map.agent,
        vendor_event: normalize_vendor_tool(vendor_event),
        content,
    }
}

/// The first of several alternative keys that carries a string.
pub(crate) fn first_str<'a>(value: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    let value = value?;
    keys.iter()
        .find_map(|k| value.get(*k))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// The first of several alternative keys that carries an object or array.
pub(crate) fn first_value<'a>(value: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
    let value = value?;
    keys.iter().find_map(|k| value.get(*k))
}

/// The first of several alternative keys that carries an integer.
pub(crate) fn first_i64(value: Option<&Value>, keys: &[&str]) -> Option<i64> {
    let value = value?;
    keys.iter()
        .find_map(|k| value.get(*k))
        .and_then(serde_json::Value::as_i64)
}

/// Establish a file's repository-relative identity, or say honestly that there
/// is none (`contracts/safe-events.md` §6, FR-777e–g, SC-707, SC-744).
///
/// Four dispositions and no fifth. In particular there is **no** synthesized
/// value, no working-directory substitute, and no degradation to a generic
/// command: a tool that changed a file Cairn could not place is recorded as a
/// file change whose identity is unavailable, because that is a different fact
/// from "a command ran" and reporting it as one would hide that Cairn is
/// watching a tool it cannot place.
///
/// The repository root is used here and never transmitted (FR-753).
pub fn repo_file_identity(
    raw: Option<&str>,
    repo_root: Option<&Path>,
) -> (FileIdentity, Option<String>) {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return (FileIdentity::UnavailableFromVendor, None);
    };

    // Relativize before validating. An absolute path *inside* the repository is
    // the ordinary case, and refusing it as absolute would report the ordinary
    // case as an attack.
    let candidate = match (repo_root, Path::new(raw).is_absolute()) {
        (Some(root), true) => match Path::new(raw).strip_prefix(root) {
            Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
            // Absolute and not under the root: outside the repository. Not a
            // malformed path — a true fact about where the tool worked.
            Err(_) => return (FileIdentity::OutOfRepository, None),
        },
        (None, true) => return (FileIdentity::OutOfRepository, None),
        (_, false) => raw.replace('\\', "/"),
    };

    match cairn_core::validate::validate_repo_file(&candidate) {
        // Screened here as well as bounded, because path segments are a
        // vocabulary source: a file *named* after a credential must not
        // contribute one (`contracts/extraction.md` §13.3).
        Ok(normalized) => {
            for segment in normalized.split('/') {
                if cairn_core::validate::validate_safe_event_text(
                    SafeEventField::Provenance,
                    segment,
                    &[],
                )
                .is_err()
                {
                    return (FileIdentity::OutOfRepository, None);
                }
            }
            (FileIdentity::Present, Some(normalized))
        }
        // A traversing or otherwise unusable path points somewhere Cairn cannot
        // place inside the repository. Refused, never repaired: stripping a
        // leading `/` or resolving a `..` would turn a path outside the
        // repository into one that looks inside it.
        Err(_) => (FileIdentity::OutOfRepository, None),
    }
}

/// Redact a free-text field and refuse it if screening still fails.
///
/// Redaction runs first and every later step reads only the redacted form, so a
/// credential can neither reach the wire nor enter the session vocabulary and
/// legitimise a token for itself. Screening after redaction is the second line:
/// redaction is a mechanism, not a guarantee, and a field it did not clean is
/// dropped rather than sent.
pub fn safe_text(field: SafeEventField, raw: &str) -> Option<String> {
    let redacted = redact(raw);
    if redacted.len() > cairn_core::event::FREE_TEXT_MAX_BYTES {
        // Over-bound values are refused, never truncated: a truncated command
        // is a different command.
        return None;
    }
    match cairn_core::validate::validate_safe_event_text(field, &redacted, &[]) {
        Ok(()) => Some(redacted),
        Err(_) => None,
    }
}

/// Everything one tool invocation establishes, in spool order.
///
/// The tool event comes first because it is the fact the vendor reported; the
/// derived events follow because they are what that fact means. A file-touching
/// tool always yields a `file_changed` or `file_read` — with an explicit
/// identity disposition when the path could not be established — and never
/// degrades into `command_executed` (SC-707, SC-744).
pub fn tool_capture(
    map: &FieldMap,
    vendor_event: &str,
    payload: &Value,
    failed: bool,
    env: &CaptureEnv<'_>,
) -> CaptureOutput {
    let Some(tool) = first_str(Some(payload), map.tool_name) else {
        return CaptureOutput::default()
            .declined(EventKind::ToolSucceeded, DeclineReason::VendorUnavailable);
    };
    let vendor_tool = normalize_vendor_tool(tool).unwrap_or_else(|| "tool".to_string());
    let input = first_value(Some(payload), map.tool_input);
    let response = first_value(Some(payload), map.tool_response);
    let exit_status = first_i64(response, map.response_exit_status).map(|v| v as i32);
    let note = first_str(response, map.response_error)
        .and_then(|raw| safe_text(SafeEventField::FailureNote, raw));

    let raw_command = first_str(input, map.input_command);
    let raw_file = first_str(input, map.input_file_path);
    let tool_class = tool_class_of(&vendor_tool, raw_command, raw_file);

    let mut out = CaptureOutput::default();
    out = if failed {
        out.event(draft(
            map,
            EventKind::ToolFailed,
            vendor_event,
            Some(EventContent::ToolFailure {
                vendor_tool: vendor_tool.clone(),
                tool_class,
                failure_kind: failure_kind_of(exit_status, note.as_deref()),
                failure_note: note,
                exit_status,
            }),
        ))
    } else {
        out.event(draft(
            map,
            EventKind::ToolSucceeded,
            vendor_event,
            Some(EventContent::Tool {
                vendor_tool: vendor_tool.clone(),
                tool_class,
            }),
        ))
    };

    match tool_class {
        ToolClass::Edit | ToolClass::Read => {
            let (file_identity, repo_file) = repo_file_identity(raw_file, env.repo_root);
            let kind = if tool_class == ToolClass::Edit {
                EventKind::FileChanged
            } else {
                EventKind::FileRead
            };
            out = out.event(draft(
                map,
                kind,
                vendor_event,
                Some(EventContent::File {
                    repo_file,
                    repo_file_from: None,
                    // Only a rename or a deletion the vendor actually reported
                    // may claim to be one. An edit tool establishes that the
                    // file changed, and nothing finer.
                    change_kind: (kind == EventKind::FileChanged).then_some(ChangeKind::Modified),
                    file_identity,
                }),
            ));
        }
        ToolClass::Test => {
            if let Some(command) =
                raw_command.and_then(|c| safe_text(SafeEventField::TestCommand, c))
            {
                out = out.event(draft(
                    map,
                    EventKind::TestExecuted,
                    vendor_event,
                    Some(EventContent::TestInvocation {
                        test_command: command,
                    }),
                ));
            }
            out = out.event(draft(
                map,
                EventKind::TestResult,
                vendor_event,
                Some(EventContent::TestVerdict {
                    // Read, never inferred. A runner whose verdict Cairn could
                    // not read is not a pass.
                    test_outcome: match (failed, exit_status) {
                        (true, _) => TestOutcome::Failed,
                        (false, Some(0)) => TestOutcome::Passed,
                        (false, Some(_)) => TestOutcome::Failed,
                        (false, None) => TestOutcome::Unknown,
                    },
                    exit_status,
                    tests_total: None,
                    tests_failed: None,
                }),
            ));
        }
        ToolClass::Execute => {
            if let Some(command) =
                raw_command.and_then(|c| safe_text(SafeEventField::CommandLine, c))
            {
                out = out.event(draft(
                    map,
                    EventKind::CommandExecuted,
                    vendor_event,
                    Some(EventContent::Command {
                        command_line: command,
                        exit_status,
                    }),
                ));
            }
        }
        ToolClass::Research => {
            out = out.event(draft(
                map,
                EventKind::ResearchActivity,
                vendor_event,
                // Deliberately coarse. A URL is a locator, and a locator is
                // exactly the sort of value this boundary does not carry.
                Some(EventContent::Research {
                    resource_kind: ResearchResource::Web,
                }),
            ));
        }
        ToolClass::Other => {}
    }
    out
}

/// A tool's class, from what it is called and what it was given.
///
/// The command and the file are consulted because a vendor's generic execution
/// tool is a test run when what it ran was a test — and recording a test run as
/// a generic command would lose the strongest signal in the extraction model.
fn tool_class_of(vendor_tool: &str, command: Option<&str>, file: Option<&str>) -> ToolClass {
    if let Some(command) = command {
        return if is_test_command(command) {
            ToolClass::Test
        } else {
            ToolClass::Execute
        };
    }
    match classify_tool(vendor_tool) {
        cairn_core::domain::ObservationType::FileChanged => ToolClass::Edit,
        cairn_core::domain::ObservationType::FileRead => ToolClass::Read,
        cairn_core::domain::ObservationType::TestRun => ToolClass::Test,
        cairn_core::domain::ObservationType::CommandRun => ToolClass::Execute,
        cairn_core::domain::ObservationType::Discovery => ToolClass::Research,
        _ if file.is_some() => ToolClass::Edit,
        _ => ToolClass::Other,
    }
}

/// Classify a failure rather than describing it.
///
/// Consolidation counts repeats of *the same* failure, and two spellings of one
/// English sentence would never count as the same failure. The note carries the
/// redacted detail alongside, for a human.
fn failure_kind_of(exit_status: Option<i32>, note: Option<&str>) -> FailureKind {
    let lowered = note.unwrap_or_default().to_ascii_lowercase();
    for (needle, kind) in [
        ("not found", FailureKind::NotFound),
        ("no such file", FailureKind::NotFound),
        ("permission denied", FailureKind::PermissionDenied),
        ("timed out", FailureKind::Timeout),
        ("timeout", FailureKind::Timeout),
        ("interrupted", FailureKind::Interrupted),
        ("invalid", FailureKind::InvalidInput),
        ("unavailable", FailureKind::Unavailable),
    ] {
        if lowered.contains(needle) {
            return kind;
        }
    }
    match exit_status {
        Some(0) | None => FailureKind::Unknown,
        Some(_) => FailureKind::NonZeroExit,
    }
}

/// Map transient vendor text to a semantic signal, or record why not.
///
/// The material is read here, in memory, during the invocation that already
/// parses and redacts it, and is discarded when this function returns. No
/// vendor field it reads is ever persisted, locally or centrally — which is why
/// the raw payload never crosses to the daemon (FR-730, SC-741).
pub fn semantic_capture(
    map: &FieldMap,
    vendor_event: &str,
    payload: &Value,
    role: SourceRole,
    env: &CaptureEnv<'_>,
) -> CaptureOutput {
    let keys = match role {
        SourceRole::UserPrompt => map.user_prompt,
        SourceRole::AssistantMessage => map.assistant_message,
    };
    // An agent Cairn declines to read semantic material from names no field
    // here, and that is the decline — not an omission to be discovered later.
    if keys.is_empty() {
        return CaptureOutput::default().declined(role.event_kind(), DeclineReason::PolicyExcluded);
    }
    // A null settled assistant message is not an empty decision. It is the
    // vendor saying there was no settled turn text, which declines.
    let Some(text) = first_str(Some(payload), keys) else {
        return CaptureOutput::default()
            .declined(role.event_kind(), DeclineReason::VendorUnavailable);
    };

    match map_semantic_signal(text, role, env.vocabulary, env.established_values) {
        Ok(signal) => CaptureOutput::default().event(draft(
            map,
            signal.kind,
            vendor_event,
            Some(signal.content),
        )),
        Err(reason) => CaptureOutput::default().declined(role.event_kind(), reason),
    }
}

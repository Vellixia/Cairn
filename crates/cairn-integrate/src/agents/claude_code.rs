//! The Claude Code adapter (D30).
//!
//! Also the home of the **legacy bridge**: a Feature 001 installation has no
//! local record, so its resources are recognized by a closed set of exact
//! shapes, adopted in place at the scope they are already at, and never
//! matched by shape again.
//!
//! Feature 001 identified its own hook entries with
//! `entry.to_string().contains("cairn hook")`, which also matches a
//! developer's `echo "run cairn hook first"`. FR-139 forbids that, and this
//! module is where it is replaced.

use super::*;
use crate::adapter::{AgentAdapter, Detection, RawPayload};
use crate::model::{AgentId, InstallationScope, ResourceKind};
use crate::scope::{self, Env};
use cairn_core::lifecycle::{CanonicalEvent, CanonicalLifecycleEvent};

pub struct ClaudeCode;

/// The vendor events Cairn registers.
///
/// The first seven are Feature 002's, one per canonical lifecycle event, and
/// they still drive sessions, handoffs and context delivery. Feature 005 adds
/// three that the lifecycle has no counterpart for and safe-event capture does:
/// a prompt-time event, a pre-tool event, and a subagent boundary. They are
/// registered rather than inferred because a hook that is not registered never
/// fires, and an event that never fires cannot be reported as anything.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "Stop",
    "SubagentStop",
    "PreCompact",
    "PostCompact",
    "SessionEnd",
];

/// Which vendor field carries which fact (T046, `contracts/extraction.md`
/// §13.10, checked against official documentation on 2026-08-30).
///
/// `StopFailure.last_assistant_message` is deliberately absent. On Claude Code
/// that field carries the API error string itself — *"API Error: Rate limit
/// reached"* — not model prose, and feeding an error string to classification
/// would manufacture decisions out of infrastructure failures. `StopFailure` is
/// not a registered event either, so the field cannot be reached by accident.
///
/// `MessageDisplay.delta` is absent for the same kind of reason: it streams
/// partial assistant text, and classifying half a sentence would fire on a
/// decision the session had not finished making. Only settled turn text is
/// read.
pub const FIELDS: FieldMap = FieldMap {
    agent: EventAgent::ClaudeCode,
    session_keys: &["session_id"],
    tool_name: &["tool_name"],
    tool_input: &["tool_input"],
    tool_response: &["tool_response"],
    input_file_path: &["file_path", "notebook_path"],
    input_command: &["command"],
    response_exit_status: &["exit_code"],
    response_error: &["error", "message", "stderr"],
    open_trigger: &["source"],
    compaction_trigger: &["trigger"],
    close_reason: &["reason"],
    user_prompt: &["prompt"],
    assistant_message: &["last_assistant_message"],
    subagent_ref: &["agent_id"],
    subagent_kind: &["agent_type"],
    // Claude has a dedicated failure event, so nothing here ever has to decide
    // from a success payload whether a tool failed.
    classify_failure: failure_from_response,
};

/// What each registered event produces, in spool order.
pub const ROUTES: RoutingTable = &[
    ("SessionStart", &[Route::SessionOpen]),
    ("UserPromptSubmit", &[Route::UserPrompt]),
    ("PreToolUse", &[Route::ToolStarted]),
    (
        "PostToolUse",
        &[Route::Tool {
            failed: Some(false),
        }],
    ),
    ("PostToolUseFailure", &[Route::Tool { failed: Some(true) }]),
    // The turn boundary and the decision it may have settled. The quiescence
    // comes first because it is the fact the vendor reported; the signal
    // follows because it is what that turn's text meant, and it may cite a
    // token only an earlier ordinal established.
    ("Stop", &[Route::Quiesced, Route::AssistantMessage]),
    (
        "SubagentStop",
        &[Route::SubagentCompleted, Route::AssistantMessage],
    ),
    ("PreCompact", &[Route::Compacting]),
    ("PostCompact", &[Route::Compacted]),
    ("SessionEnd", &[Route::SessionClose]),
];

/// The six events Feature 001 registered. The legacy bridge matches these
/// shapes exactly and nothing else.
pub const LEGACY_EVENTS: &[&str] = &[
    "SessionStart",
    "PostToolUse",
    "PostToolUseFailure",
    "PreCompact",
    "Stop",
    "SessionEnd",
];

/// Events that fire per tool call and therefore carry a matcher.
const TOOL_EVENTS: &[&str] = &["PostToolUse", "PostToolUseFailure"];

/// The hook entry Cairn writes for one event.
pub fn hook_entry(event: &str) -> serde_json::Value {
    let command = format!("cairn hook {event}");
    if TOOL_EVENTS.contains(&event) {
        serde_json::json!({
            "matcher": "*",
            "hooks": [{ "type": "command", "command": command }]
        })
    } else {
        serde_json::json!({ "hooks": [{ "type": "command", "command": command }] })
    }
}

/// Whether a hook entry is Cairn's own, by exact shape.
///
/// The closed set is: an entry whose `hooks` list contains exactly one
/// command hook whose command is exactly `cairn hook <Event>` for an event
/// Cairn registers. A longer command that merely mentions `cairn hook` does
/// not match, which is the whole point (FR-139).
pub fn is_cairn_hook_entry(entry: &serde_json::Value, event: &str) -> bool {
    let Some(hooks) = entry.get("hooks").and_then(|h| h.as_array()) else {
        return false;
    };
    if hooks.len() != 1 {
        return false;
    }
    let h = &hooks[0];
    if h.get("type").and_then(|t| t.as_str()) != Some("command") {
        return false;
    }
    let Some(command) = h.get("command").and_then(|c| c.as_str()) else {
        return false;
    };
    command == format!("cairn hook {event}")
}

/// Whether an MCP entry is Cairn's own, by exact shape.
pub fn is_cairn_mcp_entry(entry: &serde_json::Value) -> bool {
    entry == &crate::mcp_entry()
}

impl AgentAdapter for ClaudeCode {
    fn id(&self) -> AgentId {
        AgentId::ClaudeCode
    }

    fn detect(&self, env: &Env) -> Detection {
        // Filesystem only: detection modifies nothing and needs no network
        // (FR-105), and spawning the vendor binary would make fixtures depend
        // on what happens to be installed.
        let marker = env.home.join(".claude");
        let config = env.home.join(".claude.json");
        if !marker.exists() && !config.exists() {
            return Detection::absent();
        }
        let version = std::fs::read_to_string(&config)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| {
                v.get("version")
                    .and_then(|s| s.as_str())
                    .map(str::to_string)
            });
        Detection::found(version, Some(marker))
    }

    fn inspect(&self, env: &Env, record: &[RecordedInstall]) -> Vec<Observed> {
        let agent = AgentId::ClaudeCode;
        let mut out = Vec::new();
        let find = |k: ResourceKind| record.iter().find(|r| r.agent == agent && r.kind == k);

        // MCP — adopted at whatever scope it is actually at (FR-217).
        let mcp_scope = find(ResourceKind::Mcp)
            .map(|r| r.scope)
            .unwrap_or(InstallationScope::User);
        if let Some(path) = scope::location(env, agent, ResourceKind::Mcp, mcp_scope) {
            // Claude uses `mcpServers` at both scopes: `~/.claude.json` and
            // `.mcp.json` share the key, only the file differs.
            let keys = ["mcpServers", crate::MCP_SERVER_NAME];
            out.push(inspect_mcp_json(
                &path,
                &keys,
                mcp_scope,
                find(ResourceKind::Mcp),
                crate::mcp_entry(),
            ));
        }

        // Lifecycle.
        let life_scope = find(ResourceKind::Lifecycle)
            .map(|r| r.scope)
            .unwrap_or(InstallationScope::ProjectLocal);
        if let Some(path) = scope::location(env, agent, ResourceKind::Lifecycle, life_scope) {
            out.push(inspect_hooks(
                &path,
                life_scope,
                find(ResourceKind::Lifecycle),
            ));
        }

        // Instructions and Skill are shared machinery.
        if let Some(path) = scope::location(
            env,
            agent,
            ResourceKind::Instructions,
            InstallationScope::ProjectShared,
        ) {
            out.push(inspect_instructions(
                &path,
                InstallationScope::ProjectShared,
                record,
                find(ResourceKind::Instructions),
            ));
        }
        if let Some(path) =
            scope::location(env, agent, ResourceKind::Skill, InstallationScope::User)
        {
            out.push(inspect_skill(
                &path,
                InstallationScope::User,
                record,
                find(ResourceKind::Skill),
            ));
        }
        out
    }

    fn registered_events(&self) -> &'static [&'static str] {
        EVENTS
    }

    fn capture(&self, event: &str, payload: &RawPayload, env: &CaptureEnv<'_>) -> CaptureOutput {
        route_capture(&FIELDS, ROUTES, event, payload, env)
    }

    fn carries_semantic_material(&self, event: &str) -> bool {
        matches!(event, "UserPromptSubmit" | "Stop" | "SubagentStop")
    }

    fn normalize(&self, event: &str, payload: &RawPayload) -> Option<CanonicalLifecycleEvent> {
        let key = session_key(payload, &["session_id"])?;
        let ev = |k: CanonicalEvent| canonical(k, AgentId::ClaudeCode, key.clone(), payload);

        match event {
            "SessionStart" => Some(
                ev(CanonicalEvent::SessionOpened)
                    .with_source(payload.str("source").map(str::to_string)),
            ),
            "PostToolUse" => {
                let tool = payload.str("tool_name").unwrap_or("unknown");
                let exit = payload
                    .value("tool_response")
                    .and_then(|r| r.get("exit_code"))
                    .and_then(|v| v.as_i64());
                Some(
                    ev(CanonicalEvent::ToolSucceeded).with_observation(tool_observation(
                        tool,
                        payload.value("tool_input"),
                        exit,
                        false,
                        None,
                    )),
                )
            }
            "PostToolUseFailure" => {
                let tool = payload.str("tool_name").unwrap_or("unknown");
                let exit = payload
                    .value("tool_response")
                    .and_then(|r| r.get("exit_code"))
                    .and_then(|v| v.as_i64());
                // Built from the failure event's own data, never inferred from
                // a success payload (D16, FR-117).
                let detail = payload
                    .value("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(str::to_string);
                Some(
                    ev(CanonicalEvent::ToolFailed).with_observation(tool_observation(
                        tool,
                        payload.value("tool_input"),
                        exit,
                        true,
                        detail,
                    )),
                )
            }
            // `last_assistant_message` and `tool_calls` are read for nothing
            // and never persisted (D35).
            "Stop" => Some(ev(CanonicalEvent::AgentQuiesced)),
            "PreCompact" => Some(
                ev(CanonicalEvent::ContextCompacting)
                    .with_trigger(payload.str("trigger").map(str::to_string)),
            ),
            "PostCompact" => Some(
                ev(CanonicalEvent::ContextCompacted)
                    .with_trigger(payload.str("trigger").map(str::to_string)),
            ),
            "SessionEnd" => Some(
                ev(CanonicalEvent::SessionClosed)
                    .with_reason(payload.str("reason").map(str::to_string)),
            ),
            // Every other Claude event — PreToolUse, UserPromptSubmit,
            // SubagentStop, Notification and the rest — is declined. Cairn
            // registers only what its canonical lifecycle needs (US2 #6).
            _ => None,
        }
    }
}

/// Inspect Claude's hook registrations.
fn inspect_hooks(
    path: &std::path::Path,
    scope: InstallationScope,
    recorded: Option<&RecordedInstall>,
) -> Observed {
    let display = path.display().to_string();
    let text = read(path);
    let value = match crate::edit::json::read(&display, &text) {
        Ok(v) => v,
        Err(e) => return malformed(ResourceKind::Lifecycle, scope, path, &e),
    };
    let hooks = value.get("hooks");
    let mut present = 0usize;
    let mut duplicated = false;
    for ev in EVENTS {
        let entries = hooks
            .and_then(|h| h.get(*ev))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        let ours = entries
            .iter()
            .filter(|e| is_cairn_hook_entry(e, ev))
            .count();
        if ours > 0 {
            present += 1;
        }
        if ours > 1 {
            duplicated = true;
        }
    }
    let base = Observed::new(ResourceKind::Lifecycle, HealthCondition::Healthy)
        .at(scope, Some(path.to_path_buf()))
        .owned_by(recorded.map(|r| r.owner).unwrap_or(ResourceOwner::Direct));

    if duplicated {
        return Observed::new(ResourceKind::Lifecycle, HealthCondition::Duplicated)
            .at(scope, Some(path.to_path_buf()))
            .detail("more than one Cairn registration for a single event")
            .remedy("cairn repair claude-code");
    }
    if present == 0 {
        return Observed::new(ResourceKind::Lifecycle, HealthCondition::Missing)
            .at(scope, Some(path.to_path_buf()))
            .detail("no Cairn hook registrations in this file");
    }
    if present < EVENTS.len() {
        return Observed::new(ResourceKind::Lifecycle, HealthCondition::Outdated)
            .at(scope, Some(path.to_path_buf()))
            .detail(format!(
                "{present} of {} Cairn hook registrations present",
                EVENTS.len()
            ))
            .remedy("cairn repair claude-code");
    }
    base
}

/// What the legacy bridge found in a Feature 001 installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyAdoption {
    pub kind: ResourceKind,
    pub scope: InstallationScope,
    pub path: std::path::PathBuf,
}

/// Recognize a Feature 001 installation by its exact shapes, so it is adopted
/// in place rather than duplicated (FR-217, SC-103).
///
/// Feature 001 wrote lifecycle to `.claude/settings.json` and MCP to
/// `.mcp.json` — both committed project scope. Both are adopted at
/// `project_shared` and **not** relocated to Feature 002's defaults.
pub fn detect_legacy(env: &Env) -> Vec<LegacyAdoption> {
    let mut out = Vec::new();

    let settings = env.worktree.join(".claude").join("settings.json");
    let text = read(&settings);
    if let Ok(value) = crate::edit::json::read(&settings.display().to_string(), &text) {
        let has_any = LEGACY_EVENTS.iter().any(|ev| {
            value
                .get("hooks")
                .and_then(|h| h.get(*ev))
                .and_then(|e| e.as_array())
                .map(|a| a.iter().any(|e| is_cairn_hook_entry(e, ev)))
                .unwrap_or(false)
        });
        if has_any {
            out.push(LegacyAdoption {
                kind: ResourceKind::Lifecycle,
                scope: InstallationScope::ProjectShared,
                path: settings,
            });
        }
    }

    let mcp = env.worktree.join(".mcp.json");
    let text = read(&mcp);
    if let Ok(value) = crate::edit::json::read(&mcp.display().to_string(), &text) {
        if value
            .get("mcpServers")
            .and_then(|s| s.get(crate::MCP_SERVER_NAME))
            .map(is_cairn_mcp_entry)
            .unwrap_or(false)
        {
            out.push(LegacyAdoption {
                kind: ResourceKind::Mcp,
                scope: InstallationScope::ProjectShared,
                path: mcp,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(json: serde_json::Value) -> RawPayload {
        RawPayload::new(json, "/repo")
    }

    #[test]
    fn all_seven_canonical_events_are_produced() {
        // contracts/lifecycle.md §Claude Code.
        let a = ClaudeCode;
        let cases: Vec<(&str, CanonicalEvent)> = vec![
            ("SessionStart", CanonicalEvent::SessionOpened),
            ("PostToolUse", CanonicalEvent::ToolSucceeded),
            ("PostToolUseFailure", CanonicalEvent::ToolFailed),
            ("Stop", CanonicalEvent::AgentQuiesced),
            ("PreCompact", CanonicalEvent::ContextCompacting),
            ("PostCompact", CanonicalEvent::ContextCompacted),
            ("SessionEnd", CanonicalEvent::SessionClosed),
        ];
        for (vendor, canonical) in cases {
            let p = payload(json!({"session_id": "s-1", "tool_name": "Read"}));
            let e = a.normalize(vendor, &p).expect(vendor);
            assert_eq!(e.event, canonical, "{vendor}");
            assert!(e.is_well_formed(), "{vendor} produced a malformed event");
        }
    }

    #[test]
    fn unregistered_events_produce_nothing() {
        // FR-115, SC-110's negative half.
        let a = ClaudeCode;
        for vendor in [
            "PreToolUse",
            "UserPromptSubmit",
            "SubagentStop",
            "Notification",
            "PermissionRequest",
            "MessageDisplay",
        ] {
            assert!(
                a.normalize(vendor, &payload(json!({"session_id": "s-1"})))
                    .is_none(),
                "{vendor} was mapped"
            );
        }
    }

    #[test]
    fn an_event_without_a_session_id_is_declined() {
        let a = ClaudeCode;
        assert!(a.normalize("Stop", &payload(json!({}))).is_none());
        assert!(a
            .normalize("Stop", &payload(json!({"session_id": ""})))
            .is_none());
    }

    #[test]
    fn quiescence_carries_no_observation() {
        // FR-230, FR-231: it asserts only that the agent stopped working.
        let a = ClaudeCode;
        let e = a
            .normalize(
                "Stop",
                &payload(json!({
                    "session_id": "s-1",
                    "last_assistant_message": "I have finished the task.",
                    "tool_calls": [{"name": "Bash"}]
                })),
            )
            .unwrap();
        assert!(e.observation.is_none());
    }

    #[test]
    fn conversation_text_never_reaches_the_canonical_event() {
        // FR-199, D35, SC-121.
        let a = ClaudeCode;
        let secret = "SEEDED_ASSISTANT_TEXT";
        for vendor in ["Stop", "PostToolUse", "SessionEnd"] {
            let e = a
                .normalize(
                    vendor,
                    &payload(json!({
                        "session_id": "s-1",
                        "tool_name": "Bash",
                        "transcript_path": "/tmp/t.jsonl",
                        "last_assistant_message": secret,
                        "prompt": secret,
                        "tool_calls": [secret],
                        "tool_input": {"command": "cargo test"}
                    })),
                )
                .unwrap();
            let text = serde_json::to_string(&e).unwrap();
            assert!(!text.contains(secret), "{vendor} leaked conversation text");
            assert!(
                !text.contains("transcript"),
                "{vendor} leaked the transcript path"
            );
        }
    }

    #[test]
    fn a_failure_observation_comes_from_the_failure_event() {
        let a = ClaudeCode;
        let e = a
            .normalize(
                "PostToolUseFailure",
                &payload(json!({
                    "session_id": "s-1",
                    "tool_name": "Bash",
                    "tool_input": {"command": "cargo test"},
                    "error": {"message": "exit 101"}
                })),
            )
            .unwrap();
        let o = e.observation.unwrap();
        assert_eq!(o.kind, cairn_core::domain::ObservationType::Error);
        assert_eq!(o.outcome.as_deref(), Some("error"));
        assert_eq!(o.vendor_tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn a_test_command_is_classified_as_a_test_run() {
        let a = ClaudeCode;
        let e = a
            .normalize(
                "PostToolUse",
                &payload(json!({
                    "session_id": "s-1",
                    "tool_name": "Bash",
                    "tool_input": {"command": "cargo test --workspace"},
                    "tool_response": {"exit_code": 1}
                })),
            )
            .unwrap();
        let o = e.observation.unwrap();
        assert_eq!(o.kind, cairn_core::domain::ObservationType::TestRun);
        assert_eq!(o.outcome.as_deref(), Some("failed"));
    }

    #[test]
    fn legacy_recognition_is_exact_and_not_a_substring_match() {
        // FR-139: this is the Feature 001 rule being replaced.
        assert!(is_cairn_hook_entry(&hook_entry("Stop"), "Stop"));
        assert!(!is_cairn_hook_entry(&hook_entry("Stop"), "SessionEnd"));

        let developer_hook = json!({
            "hooks": [{ "type": "command", "command": "echo 'run cairn hook first' && make lint" }]
        });
        assert!(
            !is_cairn_hook_entry(&developer_hook, "Stop"),
            "a developer's command mentioning `cairn hook` was claimed as Cairn's"
        );

        let two_hooks = json!({
            "hooks": [
                { "type": "command", "command": "cairn hook Stop" },
                { "type": "command", "command": "make lint" }
            ]
        });
        assert!(!is_cairn_hook_entry(&two_hooks, "Stop"));
    }

    #[test]
    fn the_mcp_entry_is_matched_by_exact_shape() {
        assert!(is_cairn_mcp_entry(
            &json!({"command": "cairn", "args": ["mcp"]})
        ));
        assert!(!is_cairn_mcp_entry(&json!({
            "command": "cairn", "args": ["mcp"], "env": {"TOKEN": "x"}
        })));
        assert!(!is_cairn_mcp_entry(
            &json!({"command": "cairn-wrapper", "args": ["mcp"]})
        ));
    }

    #[test]
    fn the_seven_lifecycle_registrations_survive_the_three_capture_ones() {
        // Feature 005 registers three more events than the canonical lifecycle
        // has: a prompt-time hook, a pre-tool hook and a subagent boundary. The
        // addition is additive, and the property worth pinning is not the count
        // but that nothing the lifecycle depends on was dropped to make room.
        for lifecycle in [
            "SessionStart",
            "PostToolUse",
            "PostToolUseFailure",
            "Stop",
            "PreCompact",
            "PostCompact",
            "SessionEnd",
        ] {
            assert!(
                EVENTS.contains(&lifecycle),
                "{lifecycle} stopped being registered"
            );
        }
        for capture in ["UserPromptSubmit", "PreToolUse", "SubagentStop"] {
            assert!(EVENTS.contains(&capture), "{capture} is not registered");
        }
        assert_eq!(EVENTS.len(), 10);
        assert_eq!(LEGACY_EVENTS.len(), 6);
        assert!(!LEGACY_EVENTS.contains(&"PostCompact"));
    }

    #[test]
    fn the_error_bearing_and_streaming_fields_are_unreachable() {
        // `StopFailure.last_assistant_message` carries the API error string,
        // not model prose, and classifying it would manufacture decisions out
        // of infrastructure failures. `MessageDisplay.delta` streams a partial
        // turn. Neither event is registered and neither field is named, so the
        // refusal is structural rather than a rule somebody has to remember.
        for never in ["StopFailure", "MessageDisplay"] {
            assert!(!EVENTS.contains(&never), "{never} must not be registered");
        }
        let out = ClaudeCode.capture(
            "StopFailure",
            &payload(json!({
                "session_id": "s",
                "last_assistant_message": "API Error: Rate limit reached",
            })),
            &CaptureEnv::default(),
        );
        assert!(out.is_empty(), "an unregistered event produced capture");

        let out = ClaudeCode.capture(
            "MessageDisplay",
            &payload(json!({"session_id": "s", "delta": "we should use post"})),
            &CaptureEnv::default(),
        );
        assert!(out.is_empty(), "a streaming fragment produced capture");
    }

    #[test]
    fn an_editing_tool_yields_a_file_change_and_never_a_generic_command() {
        // SC-707 and SC-744. The identity may be unavailable; the event may not
        // degrade into something else.
        let out = ClaudeCode.capture(
            "PostToolUse",
            &payload(json!({
                "session_id": "s",
                "tool_name": "Edit",
                "tool_input": {"file_path": "crates/cairnd/src/sync.rs"},
                "tool_response": {"exit_code": 0},
            })),
            &CaptureEnv::default(),
        );
        let kinds: Vec<_> = out.events.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&cairn_core::event::EventKind::ToolSucceeded));
        assert!(kinds.contains(&cairn_core::event::EventKind::FileChanged));
        assert!(!kinds.contains(&cairn_core::event::EventKind::CommandExecuted));
    }
}

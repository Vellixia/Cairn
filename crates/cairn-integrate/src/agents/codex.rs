//! The Codex adapter (D31, D23, D24).
//!
//! Codex has **six** hook registrations for seven canonical events, because it
//! has no separate tool-failure hook: its one `PostToolUse` registration
//! normalizes into either `tool_succeeded` or `tool_failed` depending on the
//! payload. Six registrations, seven canonical events — one is not the other.
//!
//! Its handlers are trust-gated. A newly written hook does not run until the
//! user trusts it inside Codex, and editing a trusted hook resets that trust.
//! Both are reported as `installed_not_activated` with the exact step, and the
//! level reflects what actually works until they are active (FR-209).

use super::*;
use crate::adapter::{AgentAdapter, Detection, RawPayload};
use crate::capability::CapabilityProfile;
use crate::model::{ActivationState, AgentId, InstallationScope, ResourceKind};
use crate::scope::{self, Env};
use cairn_core::lifecycle::{CanonicalEvent, CanonicalLifecycleEvent};

pub struct Codex;

/// The vendor registrations Cairn writes.
///
/// Codex has no separate tool-failure hook: its one `PostToolUse` registration
/// normalizes into either outcome depending on the payload. Feature 005 adds
/// the prompt-time, pre-tool and subagent boundaries the safe-event model needs
/// and the lifecycle has no counterpart for.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "SubagentStop",
    "PreCompact",
    "PostCompact",
    "SessionEnd",
];

/// Which vendor field carries which fact (`contracts/extraction.md` §13.10).
///
/// `last_assistant_message` is **nullable** on Codex — the vendor's own field
/// table says `string | null`. A null is not an empty decision; it is the
/// vendor saying there was no settled turn text, and the mapping declines
/// rather than emitting a signal with empty tokens. Nothing here special-cases
/// it, because an absent field already declines.
///
/// `StopFailure` is not registered and no field of it is named, for the same
/// reason as on Claude Code: an error string is not model prose.
pub const FIELDS: FieldMap = FieldMap {
    agent: EventAgent::Codex,
    // Codex has spelled its session identity two ways across versions. Both are
    // tried, in order, because an adapter that knew only the current one would
    // go quiet on an upgrade rather than fail visibly.
    session_keys: &["session_id", "thread_id"],
    tool_name: &["tool_name"],
    tool_input: &["tool_input"],
    tool_response: &["tool_response"],
    input_file_path: &["file_path", "path"],
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
    // One event carries both outcomes, so the payload decides. `classify` is
    // the same order `classify_failure` has always used, so the lifecycle path
    // and the capture path cannot disagree about whether a tool failed.
    classify_failure: classify,
};

/// The boolean half of [`classify_failure`], for the capture path.
///
/// One function delegating to the other rather than two independent tests: a
/// tool that the lifecycle recorded as failed and capture recorded as succeeded
/// would be one act described two ways, and the disagreement would be invisible
/// until somebody read both.
fn classify(response: Option<&serde_json::Value>) -> bool {
    classify_failure(response).0
}

/// What each registered event produces, in spool order.
pub const ROUTES: RoutingTable = &[
    ("SessionStart", &[Route::SessionOpen]),
    ("UserPromptSubmit", &[Route::UserPrompt]),
    ("PreToolUse", &[Route::ToolStarted]),
    ("PostToolUse", &[Route::Tool { failed: None }]),
    ("Stop", &[Route::Quiesced, Route::AssistantMessage]),
    (
        "SubagentStop",
        &[Route::SubagentCompleted, Route::AssistantMessage],
    ),
    ("PreCompact", &[Route::Compacting]),
    ("PostCompact", &[Route::Compacted]),
    ("SessionEnd", &[Route::SessionClose]),
];

/// Codex's own session-end handler budget (D31). Cairn's session-end work must
/// be shown by measurement to fit inside it, or the completion guarantee is
/// not claimed (FR-208, SC-128).
pub const SESSION_END_DEFAULT_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);
pub const SESSION_END_MAX_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// How a developer activates the handlers inside Codex.
///
/// Codex exposes no `hooks` subcommand -- verified against codex-cli 0.144.6,
/// whose complete subcommand list has no such entry, so the `codex hooks trust`
/// this once named simply errored with `unexpected argument 'trust'`. Trust is
/// granted from an interactive session, and automation that already vets the
/// hook source uses Codex's own bypass flag instead. Printing a command that
/// does not exist is worse than describing the real step, because the developer
/// concludes Cairn is broken rather than that a confirmation is waiting.
pub const TRUST_REMEDY: &str =
    "start an interactive `codex` session in this repository and approve the hook trust \
     prompt (for unattended runs, Codex accepts `--dangerously-bypass-hook-trust`)";

/// Codex's feature flag that has to be on before any hook runs at all.
///
/// A developer who has set `hooks = false` under `[features]` is not waiting on
/// trust: nothing will run however trusted it is. Reporting trust as the blocker
/// there names the wrong cause.
pub const HOOKS_FEATURE_FLAG: &str = "hooks";

/// Whether `~/.codex/config.toml` disables hooks outright.
pub fn hooks_feature_disabled(config_toml: &str) -> bool {
    let mut in_features = false;
    for line in config_toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if !in_features {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == HOOKS_FEATURE_FLAG {
            return value.split('#').next().unwrap_or("").trim() == "false";
        }
    }
    false
}

/// The hook entry Cairn writes for one event.
///
/// Codex keys hooks by event and nests a group under each, the same structure
/// Claude Code uses -- `hooks.<Event>[group].hooks[i]` -- which is what its own
/// persisted trust keys describe when they read
/// `<path>/hooks.json:session_start:0:0`. Cairn used to write a flat array of
/// `{event, type, command}` instead. Codex parses the file, rejects it with
/// `invalid type: map, expected a sequence`, and drops every registration, so
/// no Cairn hook ran and the whole Codex lifecycle integration was inert while
/// reporting itself installed. Verified against codex-cli 0.144.6 with a probe
/// hook that fires.
pub fn hook_entry(event: &str) -> serde_json::Value {
    let _ = event;
    serde_json::json!({
        "hooks": [ { "type": "command", "command": hook_command(event) } ]
    })
}

/// The command Cairn registers.
///
/// Codex names itself, because the same event word means a different payload
/// shape to a different vendor and the hook has to pick the right adapter.
/// Claude Code's entry deliberately does not, so a Feature 001 hook keeps
/// working unchanged.
pub fn hook_command(event: &str) -> String {
    format!("cairn hook {event} --agent codex")
}

/// Whether a hook entry is Cairn's own, by exact shape (FR-139).
///
/// The closed set is a group whose `hooks` list holds exactly one command hook
/// whose command is exactly Cairn's for an event Cairn registers. A longer
/// command that merely mentions `cairn hook` does not match.
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
    h.get("command").and_then(|c| c.as_str()) == Some(hook_command(event).as_str())
}

/// Classify a Codex tool response (D23).
///
/// The order is fixed: explicit non-zero `exit_code`, then explicit
/// `success: false` or an `error` member, otherwise success. An
/// uninterpretable response yields the success-shaped observation and never a
/// fabricated error (FR-117).
pub fn classify_failure(response: Option<&serde_json::Value>) -> (bool, Option<String>) {
    let Some(r) = response else {
        return (false, None);
    };
    if let Some(code) = r.get("exit_code").and_then(|v| v.as_i64()) {
        if code != 0 {
            return (true, Some(format!("exit code {code}")));
        }
        return (false, None);
    }
    if r.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return (true, Some("the tool reported success: false".into()));
    }
    if let Some(err) = r.get("error") {
        if !err.is_null() {
            let detail = err
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
                .or_else(|| err.as_str().map(str::to_string));
            return (
                true,
                detail.or_else(|| Some("the tool reported an error".into())),
            );
        }
    }
    (false, None)
}

impl AgentAdapter for Codex {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn detect(&self, env: &Env) -> Detection {
        let marker = env.home.join(".codex");
        if !marker.exists() {
            return Detection::absent();
        }
        let version = std::fs::read_to_string(marker.join("version.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| {
                v.get("version")
                    .and_then(|s| s.as_str())
                    .map(str::to_string)
            });
        Detection::found(version, Some(marker))
    }

    fn capabilities(&self, detection: &Detection) -> CapabilityProfile {
        let mut p = CapabilityProfile::base(AgentId::Codex);
        if !detection.detected {
            return p;
        }
        p.handlers_require_trust = true;
        p
    }

    fn inspect(&self, env: &Env, record: &[RecordedInstall]) -> Vec<Observed> {
        let agent = AgentId::Codex;
        let mut out = Vec::new();
        let find = |k: ResourceKind| record.iter().find(|r| r.agent == agent && r.kind == k);

        let mcp_scope = find(ResourceKind::Mcp)
            .map(|r| r.scope)
            .unwrap_or(InstallationScope::User);
        if let Some(path) = scope::location(env, agent, ResourceKind::Mcp, mcp_scope) {
            out.push(inspect_mcp_toml(&path, mcp_scope, find(ResourceKind::Mcp)));
        }

        let life_scope = find(ResourceKind::Lifecycle)
            .map(|r| r.scope)
            .unwrap_or(InstallationScope::User);
        if let Some(path) = scope::location(env, agent, ResourceKind::Lifecycle, life_scope) {
            out.push(inspect_hooks(
                &path,
                life_scope,
                find(ResourceKind::Lifecycle),
            ));
        }

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
        let key = session_key(payload, &["session_id", "thread_id"])?;
        let ev = |k: CanonicalEvent| canonical(k, AgentId::Codex, key.clone(), payload);

        match event {
            "SessionStart" => Some(
                ev(CanonicalEvent::SessionOpened)
                    .with_source(payload.str("source").map(str::to_string)),
            ),
            "PostToolUse" => {
                let tool = payload.str("tool_name").unwrap_or("unknown");
                let response = payload.value("tool_response");
                let (failed, detail) = classify_failure(response);
                let exit = response
                    .and_then(|r| r.get("exit_code"))
                    .and_then(|v| v.as_i64());
                let canonical = if failed {
                    CanonicalEvent::ToolFailed
                } else {
                    CanonicalEvent::ToolSucceeded
                };
                Some(ev(canonical).with_observation(tool_observation(
                    tool,
                    payload.value("tool_input"),
                    exit,
                    failed,
                    detail,
                )))
            }
            "Stop" => Some(ev(CanonicalEvent::AgentQuiesced)),
            "PreCompact" => Some(
                ev(CanonicalEvent::ContextCompacting)
                    .with_trigger(payload.str("trigger").map(str::to_string)),
            ),
            "PostCompact" => Some(ev(CanonicalEvent::ContextCompacted)),
            "SessionEnd" => Some(
                ev(CanonicalEvent::SessionClosed)
                    .with_reason(payload.str("reason").map(str::to_string)),
            ),
            // PreToolUse, PermissionRequest, UserPromptSubmit, SubagentStart
            // and SubagentStop are not registered.
            _ => None,
        }
    }
}

fn inspect_mcp_toml(
    path: &std::path::Path,
    scope: InstallationScope,
    recorded: Option<&RecordedInstall>,
) -> Observed {
    let display = path.display().to_string();
    let text = read(path);
    let found =
        match crate::edit::toml::get(&display, &text, &["mcp_servers", crate::MCP_SERVER_NAME]) {
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

/// Inspect Codex's hooks file, including its trust state.
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
    // Hooks are keyed by event, each holding a list of groups.
    let hooks = value.get("hooks");

    let mut present = 0usize;
    let mut duplicated = false;
    for ev in EVENTS {
        let ours = hooks
            .and_then(|h| h.get(ev))
            .and_then(|g| g.as_array())
            .map(|groups| groups.iter().filter(|e| is_cairn_hook_entry(e, ev)).count())
            .unwrap_or(0);
        if ours > 0 {
            present += 1;
        }
        if ours > 1 {
            duplicated = true;
        }
    }
    let activation = trust_state(&value, recorded, path);

    let at = |c: HealthCondition| {
        Observed::new(ResourceKind::Lifecycle, c)
            .at(scope, Some(path.to_path_buf()))
            .owned_by(recorded.map(|r| r.owner).unwrap_or(ResourceOwner::Direct))
    };

    if duplicated {
        return at(HealthCondition::Duplicated)
            .detail("more than one Cairn registration for a single event")
            .remedy("cairn repair codex");
    }
    if present == 0 {
        return at(HealthCondition::Missing).detail("no Cairn hook registrations in this file");
    }
    if present < EVENTS.len() {
        return at(HealthCondition::Outdated)
            .detail(format!(
                "{present} of {} Cairn hook registrations present",
                EVENTS.len()
            ))
            .remedy("cairn repair codex");
    }
    // `[features] hooks = false` outranks trust: nothing runs however trusted
    // it is, so naming trust as the blocker points the developer at the wrong
    // switch. `config.toml` sits beside `hooks.json`.
    let feature_off = path
        .parent()
        .map(|dir| dir.join("config.toml"))
        .map(|cfg| hooks_feature_disabled(&read(&cfg)))
        .unwrap_or(false);
    if feature_off {
        let mut o = at(HealthCondition::InstalledNotActivated)
            .detail("Codex has hooks disabled outright: `[features] hooks = false`")
            .remedy(
                "set `hooks = true` under `[features]` in `~/.codex/config.toml`, then re-run \
                 `cairn doctor codex`",
            );
        o.activation = activation;
        return o;
    }
    if activation.needs_user_action() {
        let mut o = at(HealthCondition::InstalledNotActivated)
            .detail(match activation {
                ActivationState::Invalidated => {
                    "Cairn's upgrade reset Codex's trust of these handlers"
                }
                _ => "Codex will not run these handlers until you trust them",
            })
            .remedy(format!("{TRUST_REMEDY}, then re-run `cairn doctor codex`"));
        o.activation = activation;
        return o;
    }
    let mut o = at(HealthCondition::Healthy);
    o.activation = activation;
    o
}

/// The key Codex records a trusted hook under, in its own `config.toml`.
///
/// `<absolute hooks.json path>:<event in snake_case>:0:0` — the group index and
/// the hook index within it, both zero because Cairn writes exactly one group
/// holding exactly one command per event (FR-139).
fn trust_key(hooks_path: &std::path::Path, event: &str) -> String {
    let mut snake = String::with_capacity(event.len() + 2);
    for (i, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                snake.push('_');
            }
            snake.push(c.to_ascii_lowercase());
        } else {
            snake.push(c);
        }
    }
    format!("{}:{}:0:0", hooks_path.display(), snake)
}

/// Whether Codex has recorded a trusted hash for every hook Cairn registers.
///
/// Codex does **not** write trust into `hooks.json`. It writes it into its own
/// `config.toml`, beside that file:
///
/// ```toml
/// [hooks.state."/…/.codex/hooks.json:pre_compact:0:0"]
/// trusted_hash = "sha256:…"
/// ```
///
/// Cairn used to look for a `trust` field inside `hooks.json`, which Codex never
/// writes, so a developer who had trusted the handlers was still told they had
/// not — and the continuity mode stayed at its most conservative value forever,
/// with no action available that could change it.
///
/// Only presence is read, never the hash's value: verifying it would mean
/// recomputing Codex's own digest of its own file, and guessing that wrong would
/// either forge a trust Cairn cannot see or deny one the user really gave. Codex
/// re-prompts by itself when the content stops matching, and Cairn's separate
/// `content_hash` record is what detects an upgrade having reset trust (D24).
fn codex_trusted_hooks(hooks_path: &std::path::Path) -> Option<usize> {
    let config = hooks_path.parent()?.join("config.toml");
    let text = std::fs::read_to_string(&config).ok()?;
    let value = crate::edit::toml::read(&config.display().to_string(), &text).ok()?;
    let state = value.get("hooks")?.get("state")?;
    let trusted = EVENTS
        .iter()
        .filter(|ev| {
            state
                .get(trust_key(hooks_path, ev))
                .and_then(|e| e.get("trusted_hash"))
                .and_then(|h| h.as_str())
                .is_some_and(|h| !h.is_empty())
        })
        .count();
    Some(trusted)
}

/// Read the trust state Codex records for a hook.
///
/// Cairn never forges a trusted hash: it reads what Codex wrote, and where
/// there is nothing to read the honest answer is "not yet trusted" (D24).
fn trust_state(
    value: &serde_json::Value,
    recorded: Option<&RecordedInstall>,
    hooks_path: &std::path::Path,
) -> ActivationState {
    let declared = value
        .get("trust")
        .and_then(|t| t.as_str())
        .or_else(|| value.get("trust_state").and_then(|t| t.as_str()));
    match declared {
        Some("trusted") | Some("Trusted") => ActivationState::Active,
        Some("modified") | Some("Modified") => ActivationState::Invalidated,
        Some(_) => ActivationState::PendingUserTrust,
        // Nothing declared in the hooks file itself, which is the normal case:
        // ask Codex's own configuration what it trusts.
        None => match codex_trusted_hooks(hooks_path) {
            Some(n) if n == EVENTS.len() => ActivationState::Active,
            Some(_) => ActivationState::PendingUserTrust,
            None => recorded
                .map(|r| r.activation)
                .filter(|a| *a != ActivationState::NotApplicable)
                .unwrap_or(ActivationState::PendingUserTrust),
        },
    }
}

#[cfg(test)]
mod tests {

    /// Pinned against **Codex's** shape, not against `hook_entry` itself.
    ///
    /// Cairn used to write a flat array of `{event, type, command}`. codex-cli
    /// 0.144.6 parses `hooks.json`, rejects that with `invalid type: map,
    /// expected a sequence`, and drops every registration -- so no Cairn hook
    /// ran while doctor reported the resource installed. The fixtures could not
    /// catch it because they build their expectation from `hook_entry` too;
    /// this asserts the literal structure a real Codex accepts, verified with a
    /// probe hook that fires.
    #[test]
    fn the_hook_entry_is_a_group_codex_will_actually_load() {
        let e = hook_entry("SessionStart");
        // A group, whose `hooks` is a sequence of command hooks.
        let inner = e
            .get("hooks")
            .and_then(|h| h.as_array())
            .expect("group carries a `hooks` sequence");
        assert_eq!(inner.len(), 1);
        assert_eq!(inner[0]["type"], "command");
        assert_eq!(inner[0]["command"], "cairn hook SessionStart --agent codex");
        // The event is the *key* it is filed under, never a field on the entry:
        // a member named `event` is what made the file a map where Codex wanted
        // a sequence.
        assert!(e.get("event").is_none());
        assert!(e.get("command").is_none());
    }

    /// The whole document, as Codex reads it: `hooks.<Event>[group].hooks[i]`.
    #[test]
    fn the_installed_document_is_keyed_by_event() {
        let mut hooks = serde_json::Map::new();
        for ev in EVENTS {
            hooks.insert((*ev).to_string(), serde_json::json!([hook_entry(ev)]));
        }
        let doc = serde_json::json!({ "hooks": hooks });
        for ev in EVENTS {
            let groups = doc["hooks"][ev]
                .as_array()
                .unwrap_or_else(|| panic!("{ev} is keyed to a sequence of groups"));
            assert_eq!(groups.len(), 1);
            assert!(is_cairn_hook_entry(&groups[0], ev));
        }
        // Codex's trust keys are `<path>:<event>:<group>:<hook>`; two indices
        // only exist because a group nests a list.
        assert!(doc["hooks"]["SessionStart"][0]["hooks"][0]["command"].is_string());
    }

    #[test]
    fn a_foreign_group_is_not_mistaken_for_cairns() {
        let foreign = serde_json::json!({
            "hooks": [ { "type": "command", "command": "cairn hook SessionStart --agent codex && rm -rf /" } ]
        });
        assert!(!is_cairn_hook_entry(&foreign, "SessionStart"));
        let two = serde_json::json!({
            "hooks": [
                { "type": "command", "command": "cairn hook SessionStart --agent codex" },
                { "type": "command", "command": "something-else" }
            ]
        });
        assert!(!is_cairn_hook_entry(&two, "SessionStart"));
    }

    /// codex-cli 0.144.6 has no `hooks` subcommand at all -- its full
    /// subcommand list does not contain one -- so naming `codex hooks trust`
    /// as the remedy sent the developer to `unexpected argument 'trust'`.
    #[test]
    fn the_trust_remedy_names_no_nonexistent_subcommand() {
        assert!(!TRUST_REMEDY.contains("codex hooks"));
        // It still has to say what to actually do.
        assert!(TRUST_REMEDY.contains("codex"));
    }

    #[test]
    fn hooks_disabled_outright_is_detected() {
        let cfg = "\
[features]
hooks = false
js_repl = false

[desktop]
followUpQueueMode = \"queue\"
";
        assert!(hooks_feature_disabled(cfg));
    }

    #[test]
    fn hooks_enabled_or_absent_is_not_reported_as_disabled() {
        assert!(!hooks_feature_disabled("[features]\nhooks = true\n"));
        assert!(!hooks_feature_disabled("[features]\njs_repl = false\n"));
        assert!(!hooks_feature_disabled(""));
        // A `hooks` key in an unrelated table is not the feature flag.
        assert!(!hooks_feature_disabled("[other]\nhooks = false\n"));
    }
    use super::*;
    use serde_json::json;

    fn payload(json: serde_json::Value) -> RawPayload {
        RawPayload::new(json, "/repo")
    }

    #[test]
    fn six_registrations_cover_seven_canonical_events() {
        // D23: `PostToolUse` normalizes into two outcomes; that does not make
        // it two registrations. Feature 005 adds three registrations the
        // lifecycle has no counterpart for, and none of the six it replaces.
        assert_eq!(EVENTS.len(), 9);
        assert!(!EVENTS.contains(&"PostToolUseFailure"));
        for capture in ["UserPromptSubmit", "PreToolUse", "SubagentStop"] {
            assert!(EVENTS.contains(&capture), "{capture} is not registered");
        }
        assert!(
            !EVENTS.contains(&"StopFailure"),
            "an error string is not model prose and must be unreachable"
        );
        let a = Codex;
        let produced: std::collections::BTreeSet<_> = EVENTS
            .iter()
            .flat_map(|ev| {
                [
                    a.normalize(ev, &payload(json!({"session_id": "s"})))
                        .map(|e| e.event),
                    a.normalize(
                        ev,
                        &payload(json!({
                            "session_id": "s",
                            "tool_name": "shell",
                            "tool_response": {"exit_code": 1}
                        })),
                    )
                    .map(|e| e.event),
                ]
            })
            .flatten()
            .collect();
        assert_eq!(produced.len(), 7, "not every canonical event is reachable");
    }

    #[test]
    fn failure_classification_follows_the_documented_order() {
        // D23, FR-117.
        assert!(classify_failure(Some(&json!({"exit_code": 1}))).0);
        assert!(!classify_failure(Some(&json!({"exit_code": 0}))).0);
        assert!(classify_failure(Some(&json!({"success": false}))).0);
        assert!(classify_failure(Some(&json!({"error": {"message": "boom"}}))).0);
        // An exit code of 0 wins over a stray error member: the explicit
        // outcome is the outcome.
        assert!(!classify_failure(Some(&json!({"exit_code": 0, "error": {"message": "x"}}))).0);
    }

    #[test]
    fn an_ambiguous_response_never_fabricates_a_failure() {
        // FR-117, US3 #5: record the call without asserting a failure.
        for ambiguous in [
            json!({}),
            json!({"output": "some text"}),
            json!({"status": "who knows"}),
        ] {
            let (failed, detail) = classify_failure(Some(&ambiguous));
            assert!(!failed, "{ambiguous} was read as a failure");
            assert!(detail.is_none());
        }
        let a = Codex;
        let e = a
            .normalize(
                "PostToolUse",
                &payload(json!({
                    "session_id": "s",
                    "tool_name": "shell",
                    "tool_response": {"output": "ambiguous"}
                })),
            )
            .unwrap();
        assert_eq!(e.event, CanonicalEvent::ToolSucceeded);
        assert_ne!(
            e.observation.unwrap().kind,
            cairn_core::domain::ObservationType::Error
        );
    }

    #[test]
    fn unregistered_events_produce_nothing() {
        let a = Codex;
        for vendor in [
            "PreToolUse",
            "PermissionRequest",
            "UserPromptSubmit",
            "SubagentStart",
            "SubagentStop",
        ] {
            assert!(a
                .normalize(vendor, &payload(json!({"session_id": "s"})))
                .is_none());
        }
    }

    #[test]
    fn a_thread_id_routes_when_session_id_is_absent() {
        let a = Codex;
        let e = a
            .normalize("Stop", &payload(json!({"thread_id": "t-9"})))
            .unwrap();
        assert_eq!(e.agent_session_key, "t-9");
    }

    #[test]
    fn conversation_text_never_reaches_the_canonical_event() {
        let a = Codex;
        let secret = "SEEDED_PROMPT_TEXT";
        let e = a
            .normalize(
                "Stop",
                &payload(json!({
                    "session_id": "s",
                    "last_assistant_message": secret,
                    "prompt": secret
                })),
            )
            .unwrap();
        assert!(!serde_json::to_string(&e).unwrap().contains(secret));
    }

    #[test]
    fn trust_is_read_never_forged() {
        // D24: where there is nothing to read, the honest answer is "not yet".
        assert_eq!(
            trust_state(
                &json!({}),
                None,
                std::path::Path::new("/nonexistent/hooks.json")
            ),
            ActivationState::PendingUserTrust
        );
        assert_eq!(
            trust_state(
                &json!({"trust": "trusted"}),
                None,
                std::path::Path::new("/nonexistent/hooks.json"),
            ),
            ActivationState::Active
        );
        assert_eq!(
            trust_state(
                &json!({"trust": "modified"}),
                None,
                std::path::Path::new("/nonexistent/hooks.json"),
            ),
            ActivationState::Invalidated
        );
    }

    #[test]
    fn the_vendor_budget_is_the_documented_one() {
        assert_eq!(SESSION_END_DEFAULT_BUDGET.as_secs(), 1);
        assert_eq!(SESSION_END_MAX_BUDGET.as_secs(), 3);
    }

    #[test]
    fn hook_entries_are_matched_by_exact_shape() {
        assert!(is_cairn_hook_entry(&hook_entry("Stop"), "Stop"));
        assert!(!is_cairn_hook_entry(&hook_entry("Stop"), "SessionEnd"));
        assert!(!is_cairn_hook_entry(
            &json!({"type": "command", "command": "cairn hook Stop && make lint"}),
            "Stop"
        ));
    }

    /// Codex is a committed automatic-delivery agent (T074, `contracts/
    /// retrieval-delivery.md` §1, FR-838a): both delivery points are
    /// documented and stable for it, exactly as for Claude Code, and neither
    /// is declined the way OpenCode's are.
    ///
    /// This pins the two structural preconditions `cairn::hook`'s delivery
    /// gate depends on, from this crate's side of that boundary:
    ///
    /// - `UserPromptSubmit` is registered (so the hook actually fires and is
    ///   not silently declined by the adapter, `event_class` note in
    ///   `cairn::hook::run_blocking`); and
    /// - it maps to no canonical lifecycle event (so it is reached only
    ///   through the capture path that carries delivery, never mistaken for
    ///   a boundary event that would take the other one).
    ///
    /// `SessionStart` is unaffected by any of this: session-open delivery
    /// rides the existing `session_opened` canonical event, unchanged since
    /// before Feature 005.
    #[test]
    fn codex_registers_prompt_submit_as_capture_only() {
        let a = Codex;
        assert!(
            a.registered_events().contains(&"UserPromptSubmit"),
            "prompt-time delivery has no event to fire on otherwise"
        );
        assert!(
            a.normalize("UserPromptSubmit", &payload(json!({"session_id": "s"})))
                .is_none(),
            "UserPromptSubmit must stay capture-only, never a boundary event"
        );
    }

    /// The declared matrix agrees: neither delivery point is
    /// `declined_by_cairn` for Codex, in contrast with OpenCode's three
    /// (`opencode.rs::opencode_declines_automatic_delivery_for_every_
    /// registered_event`).
    #[test]
    fn codexs_declared_matrix_does_not_decline_delivery() {
        use crate::capability::{declared_matrix, MatrixStatus};
        let matrix = declared_matrix("codex");
        for key in ["deliver:session_open", "deliver:prompt_time"] {
            let status = matrix
                .iter()
                .find(|c| c.capability == key)
                .map(|c| c.status)
                .unwrap_or_else(|| panic!("{key} is not in Codex's matrix"));
            assert_ne!(
                status,
                MatrixStatus::DeclinedByCairn,
                "{key} must not be declined for a committed agent"
            );
        }
    }
}

//! The OpenCode adapter (D32, D21).
//!
//! OpenCode is the case that proves the honesty rule. Its event vocabulary
//! does not line up with Claude Code's, and the tempting shortcuts are wrong:
//!
//! - `session.idle` means the agent went quiet. It is **never**
//!   `session_closed` (FR-116). OpenCode signals no session end at all, which
//!   is the one genuine absence in the whole capability table.
//! - `tool.execute.after` carries no outcome flag. Provable failures do exist,
//!   so tool failure is **conditional**: emitted where the output
//!   unambiguously establishes it, and nothing where it does not (FR-117).
//! - Lifecycle is a **file drop**, not a config edit: OpenCode auto-discovers
//!   `{plugin,plugins}/*.{ts,js}` in every config directory, so installing the
//!   plugin needs no mutation of `opencode.json` at all.

use super::*;
use crate::adapter::{AgentAdapter, Detection, RawPayload};
use crate::capability::{Availability, Capability, CapabilityProfile};
use crate::model::{AgentId, InstallationScope, ResourceKind};
use crate::scope::{self, Env};
use cairn_core::lifecycle::{CanonicalEvent, CanonicalLifecycleEvent};

pub struct OpenCode;

/// The vendor signals the Cairn plugin subscribes to.
///
/// `session_closed` is deliberately absent from this list, and there is no
/// vendor signal that would produce it.
pub const EVENTS: &[&str] = &[
    "session.created",
    "tool.execute.after",
    "session.idle",
    "experimental.session.compacting",
    "session.compacted",
];

/// Which vendor field carries which fact.
///
/// **`user_prompt` and `assistant_message` are the empty slice, and that is the
/// decline.** OpenCode v1's prompt text is not in a named field at all — it has
/// to be walked out of `chat.message`'s `output.parts[]` entries — and that
/// hook is absent from the vendor's documentation, appearing only in published
/// type definitions. Its assistant-text hook carries an `experimental.` prefix.
/// Official v2 documentation does expose `event.prompt.text`, but the v2 plugin
/// API is beta and Cairn has established no stable, dedicated
/// settled-assistant-message completion boundary.
///
/// So Cairn declines semantic-signal capture for OpenCode, reported as
/// `declined_by_cairn` — a decision about an unstable surface, not a claim that
/// the vendor cannot do it (FR-838b). Expressing the decline as an empty field
/// list rather than as a check somewhere is deliberate: there is no field to
/// read, so the capability cannot be gained by an oversight in the router.
///
/// Structural capture is unaffected. R1–R6 need no prompt or assistant text, so
/// OpenCode's failure, convention and procedure learning works exactly as the
/// other two agents\' does.
pub const FIELDS: FieldMap = FieldMap {
    agent: EventAgent::OpenCode,
    session_keys: &["sessionID", "session_id"],
    tool_name: &["tool", "tool_name"],
    tool_input: &["args", "tool_input"],
    tool_response: &["output", "result"],
    input_file_path: &["filePath", "file_path", "path"],
    input_command: &["command"],
    response_exit_status: &["exit_code", "exitCode"],
    response_error: &["error", "message", "stderr"],
    open_trigger: &["source"],
    compaction_trigger: &["trigger"],
    close_reason: &["reason"],
    user_prompt: &[],
    assistant_message: &[],
    // Not established. A subagent reference sourced from a field that carries a
    // description would put authored text into an identifier, which
    // `contracts/safe-events.md` §2 forbids outright.
    subagent_ref: &[],
    subagent_kind: &[],
    classify_failure: establishes_failure_bool,
};

/// The boolean half of [`establishes_failure`], for the capture path.
///
/// Deliberately conservative in the same way: output that does not
/// *unambiguously* establish failure is not a failure. Inferring one from an
/// ambiguous payload is the fabrication FR-117 and SC-110 exist to prevent.
fn establishes_failure_bool(output: Option<&serde_json::Value>) -> bool {
    establishes_failure(output).0
}

/// What each registered event produces, in spool order.
///
/// No `UserPrompt` and no `AssistantMessage` route: the decline is structural,
/// and a route that called into the semantic mapper with an empty field list
/// would only reach the same answer more slowly.
///
/// `session.idle` means the agent went quiet. It is never `session_closed`
/// (FR-116) — OpenCode signals no session end at all — so nothing here maps one.
pub const ROUTES: RoutingTable = &[
    ("session.created", &[Route::SessionOpen]),
    ("tool.execute.after", &[Route::Tool { failed: None }]),
    ("session.idle", &[Route::Quiesced]),
    ("experimental.session.compacting", &[Route::Compacting]),
    ("session.compacted", &[Route::Compacted]),
];

/// The plugin Cairn installs. Cairn generates the whole file and owns every
/// byte of it, which is why whole-file replacement is legitimate here.
pub const PLUGIN_SOURCE: &str = include_str!("../../assets/opencode-plugin.js");

/// Whether the tool output unambiguously establishes a failure.
///
/// Deliberately narrow. An ambiguous output emits nothing rather than a
/// fabricated failure, and that is the behavior SC-110 tests from both sides.
pub fn establishes_failure(output: Option<&serde_json::Value>) -> (bool, Option<String>) {
    let Some(o) = output else {
        return (false, None);
    };
    if let Some(code) = o.get("exit_code").and_then(|v| v.as_i64()) {
        if code != 0 {
            return (true, Some(format!("exit code {code}")));
        }
        return (false, None);
    }
    if o.get("error").map(|e| !e.is_null()).unwrap_or(false) {
        let detail = o
            .get("error")
            .and_then(|e| {
                e.get("message")
                    .and_then(|m| m.as_str())
                    .or_else(|| e.as_str())
            })
            .map(str::to_string);
        return (
            true,
            detail.or_else(|| Some("the tool reported an error".into())),
        );
    }
    if o.get("failed").and_then(|v| v.as_bool()) == Some(true) {
        return (true, Some("the tool reported failed: true".into()));
    }
    (false, None)
}

impl AgentAdapter for OpenCode {
    fn id(&self) -> AgentId {
        AgentId::Opencode
    }

    fn detect(&self, env: &Env) -> Detection {
        let marker = env.config_home.join("opencode");
        if !marker.exists() {
            return Detection::absent();
        }
        let version = std::fs::read_to_string(marker.join("opencode.json"))
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
        let mut p = CapabilityProfile::base(AgentId::Opencode);
        // Pre-compaction is delivered by an experimental hook. Where the
        // installed OpenCode does not expose it, the capability is reported
        // absent rather than assumed (contracts/lifecycle.md §OpenCode).
        if detection.detected && !exposes_compaction_hook(detection) {
            if let Some(state) = p.capabilities.get_mut(&Capability::LifecyclePreCompaction) {
                state.availability = Availability::Conditional;
            }
        }
        p
    }

    fn inspect(&self, env: &Env, record: &[RecordedInstall]) -> Vec<Observed> {
        let agent = AgentId::Opencode;
        let mut out = Vec::new();
        let find = |k: ResourceKind| record.iter().find(|r| r.agent == agent && r.kind == k);

        let mcp_scope = find(ResourceKind::Mcp)
            .map(|r| r.scope)
            .unwrap_or(InstallationScope::User);
        if let Some(path) = scope::location(env, agent, ResourceKind::Mcp, mcp_scope) {
            let mut observed = inspect_mcp_json(
                &path,
                &["mcp", crate::MCP_SERVER_NAME],
                mcp_scope,
                find(ResourceKind::Mcp),
                crate::mcp_entry_opencode(),
            );
            // A `.jsonc` sibling merges *after* the `.json` and would shadow
            // Cairn's entry. It is detected and reported, never edited (D37,
            // D38).
            if let Some(shadow) = shadowing_jsonc(&path) {
                observed = Observed::new(ResourceKind::Mcp, HealthCondition::ConflictingOwner)
                    .at(mcp_scope, Some(path.clone()))
                    .detail(format!(
                        "{} also declares mcp.cairn and merges after this file, shadowing it",
                        shadow.display()
                    ))
                    .remedy("remove the cairn entry from the .jsonc file, then re-run doctor");
            }
            out.push(observed);
        }

        let life_scope = find(ResourceKind::Lifecycle)
            .map(|r| r.scope)
            .unwrap_or(InstallationScope::User);
        if let Some(path) = scope::location(env, agent, ResourceKind::Lifecycle, life_scope) {
            out.push(inspect_plugin(
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
        // OpenCode scans `~/.claude/skills` as well as its own directories, so
        // where Claude Code's Cairn Skill exists it binds to *that* resource
        // rather than writing a second copy — two copies with one skill name
        // make OpenCode log a conflict and pick non-deterministically (D28).
        let skill_path = skill_location(env, record);
        out.push(inspect_skill(
            &skill_path,
            InstallationScope::User,
            record,
            find(ResourceKind::Skill),
        ));
        out
    }

    fn registered_events(&self) -> &'static [&'static str] {
        EVENTS
    }

    fn capture(&self, event: &str, payload: &RawPayload, env: &CaptureEnv<'_>) -> CaptureOutput {
        route_capture(&FIELDS, ROUTES, event, payload, env)
    }

    // `carries_semantic_material` keeps the trait default of `false`. OpenCode
    // reads no prompt and no settled assistant message, so there is never a
    // vocabulary for a caller to fetch on its behalf.

    fn normalize(&self, event: &str, payload: &RawPayload) -> Option<CanonicalLifecycleEvent> {
        let key = session_key(payload, &["sessionID", "session_id"])?;
        let ev = |k: CanonicalEvent| canonical(k, AgentId::Opencode, key.clone(), payload);

        match event {
            "session.created" => Some(
                ev(CanonicalEvent::SessionOpened)
                    .with_source(payload.str("source").map(str::to_string)),
            ),
            "tool.execute.after" => {
                let tool = payload
                    .str("tool")
                    .or(payload.str("tool_name"))
                    .unwrap_or("unknown");
                let output = payload.value("output").or_else(|| payload.value("result"));
                let (failed, detail) = establishes_failure(output);
                let exit = output
                    .and_then(|o| o.get("exit_code"))
                    .and_then(|v| v.as_i64());
                let canonical = if failed {
                    CanonicalEvent::ToolFailed
                } else {
                    CanonicalEvent::ToolSucceeded
                };
                // The tool's `output` text and `metadata` are never persisted
                // (D35); only the derived outcome and the allow-listed input
                // fields are.
                Some(
                    ev(canonical).with_observation(tool_observation(
                        tool,
                        payload
                            .value("args")
                            .or_else(|| payload.value("tool_input")),
                        exit,
                        failed,
                        detail,
                    )),
                )
            }
            // Never `session_closed`. Idleness means the agent stopped
            // working, not that a turn succeeded and not that the session
            // ended (FR-116, FR-230, SC-111).
            "session.idle" => Some(ev(CanonicalEvent::AgentQuiesced)),
            "experimental.session.compacting" => Some(ev(CanonicalEvent::ContextCompacting)),
            "session.compacted" => Some(ev(CanonicalEvent::ContextCompacted)),
            // Deleting a record is not completing work, and none of the rest
            // is a lifecycle boundary Cairn maps.
            _ => None,
        }
    }
}

/// Whether this installation exposes the experimental compaction hook.
///
/// Absent evidence means absent capability, not an assumption in Cairn's
/// favour.
fn exposes_compaction_hook(detection: &Detection) -> bool {
    detection
        .evidence_path
        .as_ref()
        .map(|p| std::path::Path::new(p).join("experimental").exists())
        .unwrap_or(false)
}

/// Where OpenCode's Skill binding points.
pub fn skill_location(env: &Env, record: &[RecordedInstall]) -> std::path::PathBuf {
    // A record is authority only while it still describes something. One that
    // names a directory with no `SKILL.md` in it is stale -- and trusting it
    // anyway is what made `inspect` report `missing`, the plan plan an `ADD`,
    // and the apply write the second copy D28 exists to prevent. Falling
    // through to the rule below lets a wrong record heal itself.
    if let Some(r) = record
        .iter()
        .find(|r| r.agent == AgentId::Opencode && r.kind == ResourceKind::Skill)
    {
        if r.location.join("SKILL.md").exists() {
            return r.location.clone();
        }
    }
    let claude = env.home.join(".claude").join("skills").join("cairn");
    if claude.join("SKILL.md").exists() {
        return claude;
    }
    scope::location(
        env,
        AgentId::Opencode,
        ResourceKind::Skill,
        InstallationScope::User,
    )
    .unwrap_or(claude)
}

/// A `.jsonc` sibling that declares `mcp.cairn` and would shadow Cairn's entry.
fn shadowing_jsonc(json_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let jsonc = json_path.with_extension("jsonc");
    if !jsonc.exists() {
        return None;
    }
    let text = read(&jsonc);
    let display = jsonc.display().to_string();
    match crate::edit::json::get(&display, &text, &["mcp", crate::MCP_SERVER_NAME]) {
        Ok(Some(_)) => Some(jsonc),
        _ => None,
    }
}

/// Inspect the Cairn plugin file. Cairn generated the whole file, so ownership
/// is the path plus the content hash.
fn inspect_plugin(
    path: &std::path::Path,
    scope: InstallationScope,
    recorded: Option<&RecordedInstall>,
) -> Observed {
    let at = |c: HealthCondition| {
        Observed::new(ResourceKind::Lifecycle, c)
            .at(scope, Some(path.to_path_buf()))
            .owned_by(recorded.map(|r| r.owner).unwrap_or(ResourceOwner::Direct))
    };
    if !path.exists() {
        return at(HealthCondition::Missing).detail("the Cairn plugin is not installed");
    }
    let found = read(path);
    let want = crate::model::canonical_hash(PLUGIN_SOURCE);
    let have = crate::model::canonical_hash(&found);
    if have == want {
        return at(HealthCondition::Healthy);
    }
    // Cairn generated this file in its entirety, so any difference is either a
    // stale version or a hand edit. The record's hash distinguishes them.
    match recorded.and_then(|r| r.content_hash.clone()) {
        Some(recorded_hash) if recorded_hash == have => at(HealthCondition::Outdated)
            .detail("the installed Cairn plugin is behind this build")
            .remedy("cairn repair opencode"),
        Some(_) => at(HealthCondition::Modified)
            .detail("the Cairn plugin file was edited by hand")
            .remedy("cairn repair opencode --force"),
        None => at(HealthCondition::Outdated)
            .detail("the installed Cairn plugin differs from this build")
            .remedy("cairn repair opencode"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_never_gains_semantic_capture_by_accident() {
        // The decline is structural: there is no field to read, so a payload
        // that would be a decision on another agent produces nothing here. This
        // is the assertion that stops a well-meaning routing change from
        // quietly enabling a capability Cairn declined (FR-838b).
        assert!(FIELDS.user_prompt.is_empty());
        assert!(FIELDS.assistant_message.is_empty());
        for event in EVENTS {
            assert!(
                !OpenCode.carries_semantic_material(event),
                "{event} claimed to carry semantic material"
            );
            let out = OpenCode.capture(
                event,
                &RawPayload::new(
                    serde_json::json!({
                        "sessionID": "s",
                        "prompt": "we should use postgresql for storage",
                        "text": "we should use postgresql for storage",
                    }),
                    "/repo",
                ),
                &CaptureEnv::default(),
            );
            for produced in &out.events {
                assert!(
                    !matches!(
                        produced.kind,
                        cairn_core::event::EventKind::DecisionSignal
                            | cairn_core::event::EventKind::UserInstructionSignal
                    ),
                    "{event} produced a semantic signal"
                );
            }
        }
    }

    #[test]
    fn structural_capture_is_unaffected_by_the_semantic_decline() {
        // R1–R6 need no prompt or assistant text, so OpenCode's failure,
        // convention and procedure learning works exactly as the other two
        // agents' does.
        let out = OpenCode.capture(
            "tool.execute.after",
            &RawPayload::new(
                serde_json::json!({
                    "sessionID": "s",
                    "tool": "bash",
                    "args": {"command": "cargo test -p cairn-core"},
                    "output": {"exit_code": 0},
                }),
                "/repo",
            ),
            &CaptureEnv::default(),
        );
        let kinds: Vec<_> = out.events.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&cairn_core::event::EventKind::ToolSucceeded));
        assert!(kinds.contains(&cairn_core::event::EventKind::TestExecuted));
        assert!(kinds.contains(&cairn_core::event::EventKind::TestResult));
    }

    #[test]
    fn going_quiet_is_never_a_session_end() {
        // FR-116. OpenCode signals no session end at all, and reporting idle as
        // one would close a session that is still open.
        let out = OpenCode.capture(
            "session.idle",
            &RawPayload::new(serde_json::json!({"sessionID": "s"}), "/repo"),
            &CaptureEnv::default(),
        );
        let kinds: Vec<_> = out.events.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![cairn_core::event::EventKind::AgentQuiesced]);
    }
    use serde_json::json;

    fn skill_env() -> (tempfile::TempDir, crate::scope::Env) {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = crate::scope::Env::new(dir.path().join("home"), dir.path().join("repo"));
        std::fs::create_dir_all(&env.home).unwrap();
        (dir, env)
    }

    fn claude_skill(env: &crate::scope::Env) -> std::path::PathBuf {
        let p = env.home.join(".claude").join("skills").join("cairn");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("SKILL.md"), "---\nname: cairn\n---\n").unwrap();
        p
    }

    /// D28: bind to Claude Code's copy, never write a second one.
    #[test]
    fn the_skill_binds_to_claude_codes_copy_where_it_exists() {
        let (_d, env) = skill_env();
        let claude = claude_skill(&env);
        assert_eq!(skill_location(&env, &[]), claude);
    }

    /// A record naming a directory with no `SKILL.md` is stale. Trusting it made
    /// `inspect` report `missing`, the next plan `ADD`, and the apply write the
    /// duplicate D28 forbids -- so it must not win over the sharing rule.
    #[test]
    fn a_record_pointing_at_no_skill_does_not_win_over_sharing() {
        let (_d, env) = skill_env();
        let claude = claude_skill(&env);
        let stale = crate::plan::RecordedInstall {
            agent: AgentId::Opencode,
            kind: ResourceKind::Skill,
            owner: crate::model::ResourceOwner::Direct,
            scope: InstallationScope::User,
            location: env
                .config_home
                .join("opencode")
                .join("skills")
                .join("cairn"),
            content_hash: None,
            artifact_schema: None,
            artifact_revision: None,
            activation: crate::model::ActivationState::NotApplicable,
            serves: vec![AgentId::Opencode],
            container_single_line: false,
            created_container: false,
        };
        assert_eq!(skill_location(&env, &[stale]), claude);
    }

    /// The plan and the apply both resolve through `materialize_install`, so it
    /// has to agree with `inspect` about where the Skill is.
    #[test]
    fn materialize_agrees_with_the_sharing_rule() {
        let (_d, env) = skill_env();
        let claude = claude_skill(&env);
        let m = crate::install::materialize_install(
            &env,
            AgentId::Opencode,
            ResourceKind::Skill,
            InstallationScope::User,
        )
        .expect("materialize");
        assert_eq!(m.location, claude);
    }

    fn payload(json: serde_json::Value) -> RawPayload {
        RawPayload::new(json, "/repo")
    }

    #[test]
    fn idle_never_closes_a_session() {
        // FR-116, SC-111 — the single most important negative in this adapter.
        let a = OpenCode;
        let e = a
            .normalize("session.idle", &payload(json!({"sessionID": "s-1"})))
            .unwrap();
        assert_eq!(e.event, CanonicalEvent::AgentQuiesced);
        assert!(!e.event.produces_durable_handoff());
    }

    #[test]
    fn no_payload_produces_a_session_closed_event() {
        // The one genuine absence: there is no OpenCode signal Cairn maps to
        // session_closed, from any event, with any payload (FR-115).
        let a = OpenCode;
        let candidates = [
            "session.idle",
            "session.deleted",
            "session.updated",
            "session.status",
            "session.error",
            "session.end",
            "SessionEnd",
            "tool.execute.after",
            "session.compacted",
        ];
        for ev in candidates {
            for body in [
                json!({"sessionID": "s-1"}),
                json!({"sessionID": "s-1", "reason": "exit"}),
                json!({"sessionID": "s-1", "ended": true}),
            ] {
                if let Some(e) = a.normalize(ev, &payload(body)) {
                    assert_ne!(
                        e.event,
                        CanonicalEvent::SessionClosed,
                        "{ev} produced session_closed"
                    );
                }
            }
        }
    }

    #[test]
    fn quiescence_after_an_error_synthesizes_nothing() {
        // FR-231, SC-134: exactly one checkpoint, zero synthesized outcomes.
        let a = OpenCode;
        let e = a
            .normalize(
                "session.idle",
                &payload(json!({"sessionID": "s-1", "error": {"message": "boom"}})),
            )
            .unwrap();
        assert_eq!(e.event, CanonicalEvent::AgentQuiesced);
        assert!(e.observation.is_none());
    }

    #[test]
    fn a_conditional_failure_is_emitted_only_where_the_output_establishes_it() {
        // SC-110, both halves.
        let a = OpenCode;
        let establishing = a
            .normalize(
                "tool.execute.after",
                &payload(json!({
                    "sessionID": "s-1",
                    "tool": "bash",
                    "args": {"command": "cargo test"},
                    "output": {"exit_code": 101}
                })),
            )
            .unwrap();
        assert_eq!(establishing.event, CanonicalEvent::ToolFailed);

        let ambiguous = a
            .normalize(
                "tool.execute.after",
                &payload(json!({
                    "sessionID": "s-1",
                    "tool": "bash",
                    "args": {"command": "cargo test"},
                    "output": {"text": "something went wrong"}
                })),
            )
            .unwrap();
        assert_eq!(
            ambiguous.event,
            CanonicalEvent::ToolSucceeded,
            "an ambiguous output fabricated a failure"
        );
        assert_ne!(
            ambiguous.observation.unwrap().kind,
            cairn_core::domain::ObservationType::Error
        );
    }

    #[test]
    fn tool_output_text_is_never_persisted() {
        // D35: OpenCode tool `output` text and `metadata` are never retained.
        let a = OpenCode;
        let secret = "SEEDED_TOOL_OUTPUT";
        let e = a
            .normalize(
                "tool.execute.after",
                &payload(json!({
                    "sessionID": "s-1",
                    "tool": "bash",
                    "args": {"command": "ls"},
                    "output": {"text": secret, "metadata": {"note": secret}}
                })),
            )
            .unwrap();
        assert!(!serde_json::to_string(&e).unwrap().contains(secret));
    }

    #[test]
    fn unmapped_signals_produce_nothing() {
        let a = OpenCode;
        for ev in [
            "session.deleted",
            "session.updated",
            "session.status",
            "session.error",
            "session.diff",
            "message.updated",
            "file.edited",
            "permission.asked",
            "todo.changed",
            "lsp.diagnostics",
            "installation.updated",
            "server.connected",
        ] {
            assert!(
                a.normalize(ev, &payload(json!({"sessionID": "s-1"})))
                    .is_none(),
                "{ev} was mapped"
            );
        }
    }

    #[test]
    fn the_profile_reports_session_close_absent_and_failure_conditional() {
        let p = CapabilityProfile::base(AgentId::Opencode);
        assert_eq!(
            p.get(Capability::LifecycleSessionClose).availability,
            Availability::Absent
        );
        assert_eq!(
            p.get(Capability::LifecycleToolFailure).availability,
            Availability::Conditional
        );
        assert!(p.get(Capability::LifecycleToolFailure).depends_on.is_some());
    }

    #[test]
    fn establishes_failure_is_narrow() {
        assert!(establishes_failure(Some(&json!({"exit_code": 2}))).0);
        assert!(establishes_failure(Some(&json!({"error": {"message": "x"}}))).0);
        assert!(establishes_failure(Some(&json!({"failed": true}))).0);
        assert!(!establishes_failure(Some(&json!({"exit_code": 0}))).0);
        assert!(!establishes_failure(Some(&json!({"text": "error: nope"}))).0);
        assert!(!establishes_failure(Some(&json!({}))).0);
        assert!(!establishes_failure(None).0);
    }
}

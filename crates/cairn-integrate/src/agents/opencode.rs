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
    if let Some(r) = record
        .iter()
        .find(|r| r.agent == AgentId::Opencode && r.kind == ResourceKind::Skill)
    {
        return r.location.clone();
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
    use serde_json::json;

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

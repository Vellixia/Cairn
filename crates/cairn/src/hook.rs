//! Claude Code lifecycle hooks (FR-041, D15, D16).
//!
//! Two deadline classes. Capture hooks parse, hand the payload to the daemon
//! and return — 250 ms, and a missed deadline is a dropped observation, not a
//! failure. `SessionStart` must actually answer, so it gets 1,500 ms and falls
//! back to reduced context rather than blocking the agent.
//!
//! **This entry point always exits 0.** Cairn is never the reason a session
//! breaks.

use crate::client;
use crate::render;
use cairn_core::wire::{ContextPayload, Request};
use cairn_core::CairnConfig;
use std::time::Duration;

/// Which adapter this invocation serves.
///
/// Claude Code's entry stays `cairn hook <Event>` so a Feature 001 hook still
/// works unchanged; the other adapters name themselves, because the same event
/// word means different payload shapes to different vendors.
pub fn agent_from_args(argv: &[String]) -> cairn_integrate::AgentId {
    argv.iter()
        .position(|a| a == "--agent")
        .and_then(|i| argv.get(i + 1))
        .and_then(|name| cairn_integrate::AgentId::parse(name))
        .unwrap_or(cairn_integrate::AgentId::ClaudeCode)
}

/// Translate a vendor event into the canonical vocabulary.
///
/// This is the only part of `cairn-integrate` on the capture path: a pure
/// function with no I/O, no editors and no embedded assets. The cost of
/// linking the rest is binary size, not work (plan.md risk table).
fn to_canonical(
    agent: cairn_integrate::AgentId,
    event: &str,
    raw: &serde_json::Value,
    cwd: &str,
) -> Option<cairn_core::lifecycle::CanonicalLifecycleEvent> {
    cairn_integrate::normalize(
        agent,
        event,
        &cairn_integrate::RawPayload::new(raw.clone(), cwd),
    )
}

/// Run Feature 005 capture for one vendor event and spool what it produced.
///
/// Beside the canonical-lifecycle path and never instead of it: one drives
/// sessions, handoffs and context delivery, the other produces the safe events
/// the server consolidates.
///
/// The raw payload does not leave this process. Where the event carries
/// transient prompt or assistant text, the daemon's session vocabulary is
/// fetched *here* and the mapping runs *here* — sending the text the other way
/// would put a prompt fragment across the capture-process boundary, which
/// FR-730 closes and SC-741 tests. A vocabulary that cannot be fetched in time
/// is treated as empty, which declines the signal rather than delaying the
/// agent; the decline is counted, so a daemon that is always too slow is
/// visible rather than silently lossy.
#[derive(Default)]
struct Captured {
    /// The vendor's own session key, which routes the events.
    key: String,
    /// What this vendor event established, or nothing.
    output: Option<cairn_core::event::CaptureOutput>,
}

fn capture_pass(
    agent: cairn_integrate::AgentId,
    event: &str,
    raw: &serde_json::Value,
    cwd: &str,
    config: &CairnConfig,
) -> Captured {
    let payload = cairn_integrate::RawPayload::new(raw.clone(), cwd);
    let deadline = capture_deadline(config);

    let key = raw
        .get("session_id")
        .or_else(|| raw.get("sessionID"))
        .or_else(|| raw.get("thread_id"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if key.is_empty() {
        // An event that cannot name its session cannot be routed, and it is
        // declined here exactly as the lifecycle path declines it (FR-737).
        return Captured::default();
    }

    let (vocabulary, established) = if cairn_integrate::carries_semantic_material(agent, event) {
        fetch_vocabulary(agent, cwd, &key, deadline)
    } else {
        Default::default()
    };
    let root = repository_root(cwd);
    let env = cairn_integrate::agents::CaptureEnv {
        repo_root: root.as_deref(),
        vocabulary: &vocabulary,
        established_values: &established,
    };

    let output = cairn_integrate::capture(agent, event, &payload, &env);
    Captured {
        key,
        output: (!output.is_empty()).then_some(output),
    }
}

/// Ask the daemon for this session's vocabulary and established values.
///
/// Failure is not an error here. An empty vocabulary justifies no token, so the
/// mapping declines with `insufficient_vocabulary` — the honest answer when
/// Cairn cannot check a claim's grounding, and a better one than recording a
/// claim it could not ground.
fn fetch_vocabulary(
    agent: cairn_integrate::AgentId,
    cwd: &str,
    key: &str,
    deadline: Duration,
) -> (
    cairn_core::vocabulary::SessionVocabulary,
    std::collections::BTreeMap<String, String>,
) {
    let request = Request::CaptureVocabulary {
        cwd: cwd.to_string(),
        agent: agent.as_str().to_string(),
        agent_session_key: key.to_string(),
    };
    let Ok(value) = client::send_blocking(&request, deadline) else {
        return Default::default();
    };
    let vocabulary = value
        .get("vocabulary")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let established = value
        .get("established_values")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    (vocabulary, established)
}

/// The repository root an absolute path is relativized against.
///
/// Walked from the working directory rather than asked of the daemon, because
/// the answer is needed before any round trip and a `.git` entry is the same
/// fact either way. The root is machine configuration and never crosses the
/// boundary (FR-753); it is used here and discarded.
fn repository_root(cwd: &str) -> Option<std::path::PathBuf> {
    let mut here = std::path::Path::new(cwd).to_path_buf();
    loop {
        if here.join(".git").exists() {
            return Some(here);
        }
        if !here.pop() {
            return None;
        }
    }
}

/// Handle a capture-class event without an async runtime (SC-007).
///
/// Returns `true` when the event was handled here. A capture-class event needs
/// no reply, so it never needs a reactor — and building one per tool call is
/// the single largest cost Cairn adds to a session.
///
/// The adapter still runs: `normalize` is a pure function, so the boundary
/// costs nothing on this path (FR-112).
pub fn run_blocking(event: &str) -> bool {
    let argv: Vec<String> = std::env::args().collect();
    let agent = agent_from_args(&argv);

    // The class is decided from the event name *before* stdin is touched: a
    // boundary event needs a reply and takes the async path, and both paths
    // reading the payload would leave the second one with nothing.
    // Feature 005 registers three events the canonical lifecycle has no
    // counterpart for — a prompt-time hook, a pre-tool hook and a subagent
    // boundary. They have no class because they map to no lifecycle event, and
    // they are still capture: `event_class` returning `None` no longer means
    // there is nothing to do.
    let registered = cairn_integrate::adapter_for(agent)
        .registered_events()
        .contains(&event);
    match cairn_integrate::event_class(agent, event) {
        // Declined by the adapter and not registered for capture either: the
        // normal way an event Cairn does not map is handled (FR-115). Nothing
        // to do, and nothing is wrong — but the agent is writing the payload to
        // this process's stdin right now, and exiting without reading it gives
        // *the agent* a broken pipe. Cairn's hook must be invisible even when
        // it does nothing (FR-193, FR-194), so the payload is drained and
        // discarded.
        None if !registered => {
            drain_stdin();
            return true;
        }
        Some(class) if class.is_boundary_class() => return false,
        _ => {}
    }

    let raw = read_raw();
    let cwd = raw
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_else(|| ".".to_string());

    let config = CairnConfig::load();
    let captured = capture_pass(agent, event, &raw, &cwd, &config);

    // One request carrying both halves where there is a lifecycle event, and a
    // capture-only request where there is not. Two writes per tool call is the
    // largest cost Cairn adds to a session, and this path runs on every one
    // (SC-007).
    let request = match to_canonical(agent, event, &raw, &cwd) {
        Some(canonical) => Request::CanonicalEvent {
            event: canonical,
            wait_for_handoff: false,
            token_budget: None,
            capture: captured.output,
        },
        None => match captured.output {
            Some(output) => Request::CaptureEvents {
                cwd: cwd.clone(),
                agent: agent.as_str().to_string(),
                agent_session_key: captured.key,
                output,
            },
            // Registered, and this payload established nothing. Not a failure.
            None => return true,
        },
    };
    if let Err(e) = client::send_oneway_blocking(&request, capture_deadline(&config)) {
        log_drop(event, &e.message);
    }
    true
}

/// Run one hook event. Always returns; the caller always exits 0.
///
/// Every event goes through the adapter first: the daemon sees only canonical
/// events, and this is where the translation happens (FR-112). An event the
/// adapter declines simply does not occur for that agent — that is the normal
/// case for everything Cairn does not map (FR-115).
pub async fn run(event: &str) {
    let argv: Vec<String> = std::env::args().collect();
    let agent = agent_from_args(&argv);
    let raw = read_raw();
    let cwd = raw
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_else(|| ".".to_string());
    let config = CairnConfig::load();

    let captured = capture_pass(agent, event, &raw, &cwd, &config);
    let Some(canonical) = to_canonical(agent, event, &raw, &cwd) else {
        // A boundary-class caller reaching an event the lifecycle declines has
        // nothing to answer with, but the event may still be capture. This is
        // the path a registered-but-unmapped event takes when the async entry
        // point is used.
        if let Some(output) = captured.output {
            let request = Request::CaptureEvents {
                cwd: cwd.clone(),
                agent: agent.as_str().to_string(),
                agent_session_key: captured.key,
                output,
            };
            if let Err(e) = client::send_oneway(&request, capture_deadline(&config)).await {
                log_drop(event, &e.message);
            }
        }
        return;
    };

    let boundary = canonical.event.is_boundary_class();
    let deadline = if boundary {
        context_deadline(&config)
    } else {
        capture_deadline(&config)
    };
    let delivers_context = canonical.event == cairn_core::lifecycle::CanonicalEvent::SessionOpened;
    let key = canonical.agent_session_key.clone();

    let request = Request::CanonicalEvent {
        event: canonical,
        // The hook never waits for a boundary's handoff: the vendor's own
        // handler budget holds a deadline over it, and the seal is what is
        // acknowledged (D22, FR-240).
        wait_for_handoff: false,
        token_budget: None,
        capture: captured.output,
    };

    if !boundary {
        // Capture class: fire and forget. A missed deadline is a dropped
        // event, not a failure (FR-015, FR-193).
        if let Err(e) = client::send_oneway(&request, deadline).await {
            log_drop(event, &e.message);
        }
        return;
    }

    match client::send_with_deadline(&request, deadline).await {
        Ok(value) => {
            if delivers_context {
                let degraded = deliver_context(agent, &value);
                // The adapter reports the delivery outcome back, which is what
                // establishes `context_at_session_open`. A session start that
                // emitted nothing leaves the capability expected — the session
                // started, and Cairn's context did not reach it (D19a).
                report_context_delivery(agent, &cwd, &key, degraded, deadline).await;
            }
        }
        Err(e) => {
            if delivers_context {
                // Feature 001's bounded fallback: the session starts with
                // reduced context rather than waiting (FR-046, FR-195). No
                // evidence is recorded, because nothing was delivered.
                emit_context(agent, &reduced_context_notice(&e.message));
            }
            log_drop(event, &e.message);
        }
    }
}

/// Emit the briefing on the agent's own context surface.
///
/// Returns whether what was delivered was degraded. The distinction that
/// matters for evidence is between *reduced* and *absent*: an empty or
/// unassemblable briefing still demonstrates that the agent's context surface
/// carries what Cairn puts on it, and is recorded with `degraded: true`. A
/// start where nothing was emitted at all demonstrates nothing, and that case
/// never reaches this function — it is the caller's error branch, which
/// records no evidence (D19a).
fn deliver_context(agent: cairn_integrate::AgentId, value: &serde_json::Value) -> bool {
    match serde_json::from_value::<ContextPayload>(value.clone()) {
        Ok(payload) => {
            // A restored checkpoint is rendered from the raw reply, not from
            // `ContextPayload`, which has no field for it -- so deserializing
            // first and rendering only that dropped the checkpoint on the floor.
            //
            // That was a real over-claim: the daemon restored the checkpoint and
            // Cairn recorded a post-compaction *delivery*, while the text the
            // agent actually received said "no prior history for this project"
            // and carried none of it. An agent that asked with
            // `reason=post_compaction` got the checkpoint, because the CLI
            // renders from the raw value; an agent delivered to automatically
            // did not. `automatic` has to mean the same thing as asking.
            //
            // It leads, for the reason `render::continuity` documents: a stale
            // next action acted on is worse than no next action at all.
            let mut text = render::continuity(value);
            text.push_str(&render::briefing(&payload));
            let degraded = text.trim().is_empty();
            emit_context(agent, &text);
            degraded
        }
        Err(e) => {
            emit_context(agent, &reduced_context_notice(&e.to_string()));
            true
        }
    }
}

/// Tell the daemon the context surface actually carried the payload.
///
/// A degraded briefing still establishes the capability and records that it
/// was degraded: the channel demonstrably carried it, and Cairn's assembly is
/// what fell short (D19a).
async fn report_context_delivery(
    agent: cairn_integrate::AgentId,
    cwd: &str,
    _key: &str,
    degraded: bool,
    deadline: Duration,
) {
    let request = Request::IntegrationEvidence {
        cwd: cwd.to_string(),
        agent: agent.as_str().to_string(),
        capability: "context_at_session_open".into(),
        evidence: "observation".into(),
        agent_version: None,
        degraded: Some(degraded),
    };
    let _ = client::send_oneway(&request, deadline).await;
}

fn capture_deadline(config: &CairnConfig) -> Duration {
    Duration::from_millis(config.capture_deadline_ms)
}

fn context_deadline(config: &CairnConfig) -> Duration {
    Duration::from_millis(config.context_deadline_ms)
}

fn reduced_context_notice(reason: &str) -> String {
    format!(
        "# Cairn context\n\n_Reduced context: Cairn could not deliver a briefing in time \
         ({reason}). The session started anyway; run `cairn context` for the full briefing._\n"
    )
}

/// Emit context on the agent's own supported context surface.
///
/// Claude Code and Codex both read `hookSpecificOutput.additionalContext`.
///
/// OpenCode is emitted to as plain stdout, but note that its installed plugin
/// spawns `cairn hook` with stdout ignored, so nothing written here reaches an
/// OpenCode session today. OpenCode has no post-compaction session open either,
/// which is why it derives `agent_initiated` and asks instead.
fn emit_context(agent: cairn_integrate::AgentId, text: &str) {
    match agent {
        cairn_integrate::AgentId::Opencode => println!("{text}"),
        _ => {
            let out = serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": text,
                }
            });
            println!("{out}");
        }
    }
}

/// The raw vendor payload, as JSON. Nothing here is interpreted: the adapter
/// does that, and only allow-listed fields survive it (D35).
fn read_raw() -> serde_json::Value {
    use std::io::Read;
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return serde_json::Value::Null;
    }
    serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
}

/// Read whatever the agent is writing and throw it away.
///
/// For an event Cairn declines. The payload is of no interest; the *reading*
/// is, because the writer on the other end is the agent.
fn drain_stdin() {
    use std::io::Read;
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
}

/// Cairn's own log. Never stderr in a way the agent would surface.
fn log_drop(event: &str, reason: &str) {
    use std::io::Write;
    let _ = cairn_core::paths::ensure_home();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(cairn_core::paths::log_path())
    {
        let _ = writeln!(
            file,
            "{} hook {event} dropped: {reason}",
            chrono::Utc::now().to_rfc3339()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_integrate::AgentId;
    use serde_json::json;

    fn raw(v: serde_json::Value) -> serde_json::Value {
        v
    }

    #[test]
    fn every_registered_event_is_handled_by_one_path_or_the_other() {
        // The rule used to be "every registered event normalizes", which held
        // while registration and the canonical lifecycle were the same list.
        // Feature 005 registers three events the lifecycle has no counterpart
        // for, so the rule that actually matters is the one SC-706 states: an
        // event Cairn registers is either translated or captured, and zero are
        // silently dropped. A hook that fires and does nothing is the failure
        // this pins.
        for e in cairn_integrate::agents::claude_code::EVENTS {
            let payload = raw(json!({
                "session_id": "s-1",
                "tool_name": "Read",
                "tool_input": { "file_path": "a.rs" },
                "prompt": "prefer sync over drift",
                "last_assistant_message": "prefer sync over drift",
                "agent_id": "sub-1",
                "agent_type": "explorer",
            }));
            let translated = to_canonical(AgentId::ClaudeCode, e, &payload, "/repo").is_some();
            let captured = !cairn_integrate::capture(
                AgentId::ClaudeCode,
                e,
                &cairn_integrate::RawPayload::new(payload.clone(), "/repo"),
                &cairn_integrate::agents::CaptureEnv::default(),
            )
            .is_empty();
            assert!(translated || captured, "{e} is registered and does nothing");
        }
    }

    #[test]
    fn the_lifecycle_still_declines_what_it_never_mapped() {
        // FR-115. `PreToolUse` and `UserPromptSubmit` are now registered for
        // capture, and they still map to no canonical lifecycle event — the two
        // paths stayed separate rather than one quietly widening the other.
        for e in [
            "PreToolUse",
            "UserPromptSubmit",
            "SubagentStop",
            "Notification",
        ] {
            assert!(
                to_canonical(
                    AgentId::ClaudeCode,
                    e,
                    &raw(json!({"session_id": "s-1"})),
                    "/repo"
                )
                .is_none(),
                "{e} reached the canonical lifecycle"
            );
        }
    }

    #[test]
    fn an_event_no_adapter_registers_captures_nothing() {
        // The other half: not registered means nothing happens, for both paths.
        for e in ["Notification", "StopFailure", "MessageDisplay"] {
            let payload = cairn_integrate::RawPayload::new(
                json!({"session_id": "s-1", "prompt": "use postgresql"}),
                "/repo",
            );
            assert!(
                cairn_integrate::capture(
                    AgentId::ClaudeCode,
                    e,
                    &payload,
                    &cairn_integrate::agents::CaptureEnv::default()
                )
                .is_empty(),
                "{e} produced capture without being registered"
            );
        }
    }

    #[test]
    fn the_agent_comes_from_the_command_line_and_defaults_to_claude() {
        // Claude Code's entry stays `cairn hook <Event>` so a Feature 001 hook
        // keeps working unchanged.
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            agent_from_args(&argv(&["cairn", "hook", "Stop"])),
            AgentId::ClaudeCode
        );
        assert_eq!(
            agent_from_args(&argv(&["cairn", "hook", "Stop", "--agent", "codex"])),
            AgentId::Codex
        );
        assert_eq!(
            agent_from_args(&argv(&[
                "cairn",
                "hook",
                "session.idle",
                "--agent",
                "opencode"
            ])),
            AgentId::Opencode
        );
    }

    #[test]
    fn capture_class_and_boundary_class_are_split_the_documented_way() {
        // contracts/lifecycle.md: three events are boundary/context class and
        // the rest are capture class.
        use cairn_core::lifecycle::CanonicalEvent;
        for e in CanonicalEvent::ALL {
            let boundary = matches!(
                e,
                CanonicalEvent::SessionOpened
                    | CanonicalEvent::ContextCompacting
                    | CanonicalEvent::SessionClosed
            );
            assert_eq!(e.is_boundary_class(), boundary, "{e:?}");
        }
    }

    #[test]
    fn an_empty_payload_never_panics() {
        assert!(to_canonical(AgentId::ClaudeCode, "Stop", &json!({}), "/repo").is_none());
        assert!(to_canonical(
            AgentId::ClaudeCode,
            "Stop",
            &serde_json::Value::Null,
            "/repo"
        )
        .is_none());
    }
}

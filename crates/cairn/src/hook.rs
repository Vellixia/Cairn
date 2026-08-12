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
    match cairn_integrate::event_class(agent, event) {
        // Declined by the adapter: the normal way an event Cairn does not map
        // is handled (FR-115). Nothing to do, and nothing is wrong.
        None => return true,
        Some(class) if class.is_boundary_class() => return false,
        Some(_) => {}
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

    let Some(canonical) = to_canonical(agent, event, &raw, &cwd) else {
        return true;
    };

    let config = CairnConfig::load();
    let request = Request::CanonicalEvent {
        event: canonical,
        wait_for_handoff: false,
        token_budget: None,
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

    let Some(canonical) = to_canonical(agent, event, &raw, &cwd) else {
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
/// Returns whether what was delivered was degraded.
fn deliver_context(agent: cairn_integrate::AgentId, value: &serde_json::Value) -> bool {
    match serde_json::from_value::<ContextPayload>(value.clone()) {
        Ok(payload) => {
            let text = render::briefing(&payload);
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
/// Claude Code and Codex both read `hookSpecificOutput.additionalContext`;
/// OpenCode's plugin passes what it reads on stdout straight through.
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
    fn every_registered_event_reaches_the_canonical_vocabulary() {
        // The hook's job is translation, and every event Cairn registers must
        // survive it (FR-112).
        for e in cairn_integrate::agents::claude_code::EVENTS {
            let payload = raw(json!({
                "session_id": "s-1",
                "tool_name": "Read",
                "tool_input": { "file_path": "a.rs" }
            }));
            assert!(
                to_canonical(AgentId::ClaudeCode, e, &payload, "/repo").is_some(),
                "{e} did not normalize"
            );
        }
    }

    #[test]
    fn an_unregistered_event_is_declined_rather_than_mapped() {
        // FR-115: it simply does not occur for that agent.
        for e in ["PreToolUse", "UserPromptSubmit", "Notification"] {
            assert!(to_canonical(
                AgentId::ClaudeCode,
                e,
                &raw(json!({"session_id": "s-1"})),
                "/repo"
            )
            .is_none());
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

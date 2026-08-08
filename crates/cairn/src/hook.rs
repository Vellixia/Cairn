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
use cairn_core::domain::{HandoffTrigger, ObservationType, SessionStatus};
use cairn_core::tools::{classify_tool, is_test_command};
use cairn_core::wire::{ContextPayload, ContextReason, ObservationInput, Request};
use cairn_core::CairnConfig;
use serde::Deserialize;
use std::time::Duration;

/// The subset of the hook payload Cairn reads (D16).
///
/// There is no process identity and no liveness signal here, which is why
/// session boundaries are deterministic rather than inferred.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct HookPayload {
    pub session_id: Option<String>,
    pub transcript_path: Option<String>,
    pub cwd: Option<String>,
    pub hook_event_name: Option<String>,
    /// `SessionStart`: startup | resume | clear | compact.
    pub source: Option<String>,
    /// `SessionEnd`: why the session ended.
    pub reason: Option<String>,
    /// `PreCompact`: manual | auto.
    pub trigger: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_response: Option<serde_json::Value>,
    /// Some versions name the result differently; accept both.
    pub tool_result: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
    pub message: Option<String>,
}

impl HookPayload {
    fn result(&self) -> Option<&serde_json::Value> {
        self.tool_response.as_ref().or(self.tool_result.as_ref())
    }
}

/// Handle a capture-class event without an async runtime (SC-007).
///
/// Returns `true` when the event was handled here. `PostToolUse`,
/// `PostToolUseFailure` and `Stop` need no reply, so they never need a
/// reactor — and building one per tool call is the single largest cost Cairn
/// adds to a session.
pub fn run_blocking(event: &str) -> bool {
    let kind = normalize(event);
    if !matches!(
        kind,
        Event::PostToolUse | Event::PostToolUseFailure | Event::Stop
    ) {
        return false;
    }

    let payload = read_payload();
    let cwd = payload
        .cwd
        .clone()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_else(|| ".".to_string());
    let config = CairnConfig::load();
    let deadline = capture_deadline(&config);

    let request = match kind {
        Event::Stop => Request::TurnCheckpoint {
            cwd,
            agent_session_key: payload.session_id.clone(),
        },
        Event::PostToolUseFailure => Request::Observe {
            cwd,
            agent_session_key: payload.session_id.clone(),
            observation: failure_observation(&payload),
        },
        _ => Request::Observe {
            cwd,
            agent_session_key: payload.session_id.clone(),
            observation: success_observation(&payload),
        },
    };

    if let Err(e) = client::send_oneway_blocking(&request, deadline) {
        log_drop(event, &e.message);
    }
    true
}

/// Run one hook event. Always returns; the caller always exits 0.
pub async fn run(event: &str) {
    let payload = read_payload();
    let cwd = payload
        .cwd
        .clone()
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_else(|| ".".to_string());
    let config = CairnConfig::load();

    let outcome = match normalize(event) {
        Event::SessionStart => session_start(&cwd, &payload, &config).await,
        Event::PostToolUse => post_tool_use(&cwd, &payload, &config, false).await,
        Event::PostToolUseFailure => post_tool_use(&cwd, &payload, &config, true).await,
        Event::PreCompact => pre_compact(&cwd, &payload, &config).await,
        Event::Stop => stop(&cwd, &payload, &config).await,
        Event::SessionEnd => session_end(&cwd, &payload, &config).await,
        Event::Unknown => Ok(()),
    };

    if let Err(e) = outcome {
        // Dropped work is logged for the developer and invisible to the agent.
        log_drop(event, &e);
    }
}

enum Event {
    SessionStart,
    PostToolUse,
    PostToolUseFailure,
    PreCompact,
    Stop,
    SessionEnd,
    Unknown,
}

fn normalize(event: &str) -> Event {
    match event.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "sessionstart" => Event::SessionStart,
        "posttooluse" => Event::PostToolUse,
        "posttoolusefailure" => Event::PostToolUseFailure,
        "precompact" => Event::PreCompact,
        "stop" => Event::Stop,
        "sessionend" => Event::SessionEnd,
        _ => Event::Unknown,
    }
}

fn capture_deadline(config: &CairnConfig) -> Duration {
    Duration::from_millis(config.capture_deadline_ms)
}

fn context_deadline(config: &CairnConfig) -> Duration {
    Duration::from_millis(config.context_deadline_ms)
}

/// Context class: start or resume the session, then deliver the briefing.
///
/// If the deadline passes the session still starts — Cairn reports reduced
/// context rather than holding the agent (FR-046).
async fn session_start(
    cwd: &str,
    payload: &HookPayload,
    config: &CairnConfig,
) -> Result<(), String> {
    let deadline = context_deadline(config);
    let key = payload.session_id.clone();

    let start = Request::SessionStart {
        cwd: cwd.to_string(),
        agent: "claude-code".into(),
        agent_session_key: key.clone(),
        task_id: None,
    };
    if let Err(e) = client::send_with_deadline(&start, deadline).await {
        emit_context(&reduced_context_notice(&e.message));
        return Err(e.message);
    }

    let context = Request::Context {
        cwd: cwd.to_string(),
        agent_session_key: key,
        session_id: None,
        reason: Some(ContextReason::SessionStart),
        token_budget: None,
    };
    match client::send_with_deadline(&context, deadline).await {
        Ok(value) => {
            let text = match serde_json::from_value::<ContextPayload>(value) {
                Ok(payload) => render::briefing(&payload),
                Err(e) => reduced_context_notice(&e.to_string()),
            };
            emit_context(&text);
            Ok(())
        }
        Err(e) => {
            emit_context(&reduced_context_notice(&e.message));
            Err(e.message)
        }
    }
}

fn reduced_context_notice(reason: &str) -> String {
    format!(
        "# Cairn context\n\n_Reduced context: Cairn could not deliver a briefing in time \
         ({reason}). The session started anyway; run `cairn context` for the full briefing._\n"
    )
}

/// Emit context for Claude Code to inject at `SessionStart`.
fn emit_context(text: &str) {
    let out = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": text,
        }
    });
    println!("{out}");
}

/// Capture class. `PostToolUse` carries successes; `PostToolUseFailure` carries
/// failures and is always an `error` observation — never inferred from a
/// success payload (D16).
async fn post_tool_use(
    cwd: &str,
    payload: &HookPayload,
    config: &CairnConfig,
    failure: bool,
) -> Result<(), String> {
    let observation = if failure {
        failure_observation(payload)
    } else {
        success_observation(payload)
    };
    let request = Request::Observe {
        cwd: cwd.to_string(),
        agent_session_key: payload.session_id.clone(),
        observation,
    };
    // Fire and forget: the agent is never held waiting for a write to SQLite
    // (contracts/agent-integration.md, H3).
    client::send_oneway(&request, capture_deadline(config))
        .await
        .map_err(|e| e.message)
}

/// A successful tool call, as a structured observation.
fn success_observation(payload: &HookPayload) -> ObservationInput {
    let tool = payload
        .tool_name
        .clone()
        .unwrap_or_else(|| "unknown".into());
    let input = payload
        .tool_input
        .clone()
        .unwrap_or(serde_json::Value::Null);
    let path = input
        .get("file_path")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let command = input
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let kind = match &command {
        Some(c) if is_test_command(c) => ObservationType::TestRun,
        Some(_) => ObservationType::CommandRun,
        None => classify_tool(&tool),
    };
    let outcome = (kind == ObservationType::TestRun).then(|| test_outcome(payload).to_string());

    ObservationInput {
        kind,
        path: path.clone(),
        command: command.clone(),
        exit_code: payload
            .result()
            .and_then(|r| r.get("exit_code"))
            .and_then(|v| v.as_i64()),
        outcome,
        summary: success_summary(&tool, path.as_deref(), command.as_deref()),
        details: None,
    }
}

/// A failed tool call. Built from the failure event's own data, never inferred
/// from a success payload (D16).
fn failure_observation(payload: &HookPayload) -> ObservationInput {
    let tool = payload
        .tool_name
        .clone()
        .unwrap_or_else(|| "unknown".into());
    let input = payload
        .tool_input
        .clone()
        .unwrap_or(serde_json::Value::Null);
    ObservationInput {
        kind: ObservationType::Error,
        path: input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        command: input
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        exit_code: payload
            .result()
            .and_then(|r| r.get("exit_code"))
            .and_then(|v| v.as_i64()),
        outcome: Some("error".into()),
        summary: failure_summary(&tool, payload),
        details: payload.error.clone(),
    }
}

fn success_summary(tool: &str, path: Option<&str>, command: Option<&str>) -> String {
    match (path, command) {
        (Some(p), _) => format!("{tool} {p}"),
        (_, Some(c)) => format!("{tool}: {c}"),
        _ => tool.to_string(),
    }
}

fn failure_summary(tool: &str, payload: &HookPayload) -> String {
    let detail = payload
        .message
        .clone()
        .or_else(|| {
            payload
                .error
                .as_ref()
                .and_then(|e| e.as_str().map(str::to_string))
        })
        .or_else(|| {
            payload
                .error
                .as_ref()
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .result()
                .and_then(|r| r.get("error"))
                .and_then(|e| e.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "tool execution failed".to_string());
    format!("{tool} failed: {detail}")
}

/// A test's outcome, read from the tool result rather than guessed.
fn test_outcome(payload: &HookPayload) -> &'static str {
    match payload
        .result()
        .and_then(|r| r.get("exit_code"))
        .and_then(|v| v.as_i64())
    {
        Some(0) => "passed",
        Some(_) => "failed",
        None => {
            let text = payload
                .result()
                .map(|r| r.to_string().to_ascii_lowercase())
                .unwrap_or_default();
            if text.contains("failed") || text.contains("failure") || text.contains("error") {
                "failed"
            } else if text.contains("passed") || text.contains("ok") {
                "passed"
            } else {
                "unknown"
            }
        }
    }
}

/// Compaction boundary: a durable handoff, and the session stays active.
async fn pre_compact(cwd: &str, payload: &HookPayload, config: &CairnConfig) -> Result<(), String> {
    let request = Request::HandoffGenerate {
        cwd: cwd.to_string(),
        session_id: None,
        agent_session_key: payload.session_id.clone(),
        trigger: HandoffTrigger::PreCompact,
    };
    // A handoff is a boundary record, not per-call telemetry: it happens once
    // per compaction, so it is worth waiting for. Dropping it on a 250 ms
    // deadline would mean no handoff at all at the boundary (FR-032).
    client::send_with_deadline(&request, context_deadline(config))
        .await
        .map(|_| ())
        .map_err(|e| e.message)
}

/// `Stop`: the agent finished a turn. A checkpoint, not a session boundary —
/// the developer can send another prompt and the session continues (D16).
async fn stop(cwd: &str, payload: &HookPayload, config: &CairnConfig) -> Result<(), String> {
    let request = Request::TurnCheckpoint {
        cwd: cwd.to_string(),
        agent_session_key: payload.session_id.clone(),
    };
    client::send_oneway(&request, capture_deadline(config))
        .await
        .map_err(|e| e.message)
}

/// The one hook that completes a session, with its `reason` recorded.
async fn session_end(cwd: &str, payload: &HookPayload, config: &CairnConfig) -> Result<(), String> {
    let request = Request::SessionEnd {
        cwd: cwd.to_string(),
        session_id: None,
        agent_session_key: payload.session_id.clone(),
        status: SessionStatus::Completed,
        reason: payload.reason.clone(),
    };
    // Ending writes the final handoff, so it is allowed the context deadline.
    client::send_with_deadline(&request, context_deadline(config))
        .await
        .map(|_| ())
        .map_err(|e| e.message)
}

fn read_payload() -> HookPayload {
    use std::io::Read;
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() || raw.trim().is_empty() {
        return HookPayload::default();
    }
    serde_json::from_str(&raw).unwrap_or_default()
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

    #[test]
    fn every_documented_event_is_recognised() {
        for e in [
            "SessionStart",
            "PostToolUse",
            "PostToolUseFailure",
            "PreCompact",
            "Stop",
            "SessionEnd",
        ] {
            assert!(!matches!(normalize(e), Event::Unknown), "{e} unrecognised");
        }
    }

    #[test]
    fn failure_summary_uses_the_failure_payload() {
        let payload = HookPayload {
            error: Some(serde_json::json!({"message": "file not found"})),
            ..Default::default()
        };
        let s = failure_summary("Read", &payload);
        assert!(s.contains("file not found"), "{s}");
    }

    #[test]
    fn test_outcome_is_read_not_guessed() {
        let failed = HookPayload {
            tool_response: Some(serde_json::json!({"exit_code": 101})),
            ..Default::default()
        };
        assert_eq!(test_outcome(&failed), "failed");
        let passed = HookPayload {
            tool_response: Some(serde_json::json!({"exit_code": 0})),
            ..Default::default()
        };
        assert_eq!(test_outcome(&passed), "passed");
    }

    #[test]
    fn an_empty_payload_never_panics() {
        let p: HookPayload = serde_json::from_str("{}").unwrap();
        assert!(p.session_id.is_none());
    }
}

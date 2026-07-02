//! AI agent lifecycle hook handler (`cairn hook <event>`).
//!
//! Supports Claude Code, Codex CLI, and OpenCode (via plugin bridge).
//! Reads JSON payload from stdin, calls the Cairn server HTTP API, and
//! emits additionalContext JSON on stdout per the agent's hook contract.
//!
//! Hooks must never break the agent: errors go to stderr, exit code is
//! always 0.

use crate::project::{current_dir_str, detect_project};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::Read;

pub fn run(event: &str) -> Result<()> {
    if let Err(e) = run_inner(event) {
        eprintln!("cairn hook: {e}");
    }
    Ok(())
}

fn run_inner(event: &str) -> Result<()> {
    let server = std::env::var("CAIRN_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let token = std::env::var("CAIRN_TOKEN").ok().filter(|t| !t.is_empty());

    let (Some(server), Some(token)) = (server, token) else {
        eprintln!("cairn hook: CAIRN_SERVER or CAIRN_TOKEN not set. Hook skipped.");
        return Ok(());
    };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(input.trim()).unwrap_or(Value::Null);

    // v0.8.0 Sprint 3: detect the project on every hook invocation (each `cairn hook <event>`
    // call is its own process - there's no in-memory state to carry a project id across
    // SessionStart and later events), so every request this process makes carries
    // `X-Cairn-Project` and gets scoped recall/remember for free (Sprint 2's scope model).
    let (project_id, project_name) = detect_project();
    let mut rc = RemoteClient::new(&server, &token);
    rc.project_id = project_id.clone();

    if event == "SessionStart" {
        if let Some(pid) = &project_id {
            let _ = rc.post("/api/projects/upsert").send_json(json!({
                "id": pid,
                "name": project_name,
                "path": current_dir_str(),
            }));
        }
    }

    rc.dispatch(event, &payload)
}

struct RemoteClient {
    server: String,
    token: String,
    project_id: Option<String>,
}

impl RemoteClient {
    fn new(server: &str, token: &str) -> Self {
        Self {
            server: server.trim_end_matches('/').to_string(),
            token: token.to_string(),
            project_id: None,
        }
    }

    fn get(&self, path: &str) -> ureq::Request {
        let req = ureq::get(&format!("{}{}", self.server, path))
            .set("Authorization", &format!("Bearer {}", self.token));
        self.with_project_header(req)
    }

    fn post(&self, path: &str) -> ureq::Request {
        let req = ureq::post(&format!("{}{}", self.server, path))
            .set("Authorization", &format!("Bearer {}", self.token));
        self.with_project_header(req)
    }

    fn with_project_header(&self, req: ureq::Request) -> ureq::Request {
        match &self.project_id {
            Some(pid) => req.set("X-Cairn-Project", pid),
            None => req,
        }
    }

    fn dispatch(&self, event: &str, payload: &Value) -> Result<()> {
        match event {
            "SessionStart" => {
                let mut ctx = String::new();
                if let Ok(resp) = self.get("/api/guard/anchor").call() {
                    if let Ok(v) = resp.into_json::<Value>() {
                        if let Some(anchor) = v.get("anchor").and_then(Value::as_str) {
                            ctx.push_str(&format!("Current task: {anchor}\n\n"));
                        }
                    }
                }
                if let Ok(resp) = self.get("/api/profile").call() {
                    if let Ok(mems) = resp.into_json::<Vec<Value>>() {
                        if !mems.is_empty() {
                            ctx.push_str("Standing preferences:\n");
                            for m in &mems {
                                if let Some(c) = m.get("content").and_then(Value::as_str) {
                                    ctx.push_str(&format!("- {c}\n"));
                                }
                            }
                            ctx.push('\n');
                        }
                    }
                }
                if let Ok(resp) = self.get("/api/memory/wakeup").query("limit", "12").call() {
                    if let Ok(mems) = resp.into_json::<Vec<Value>>() {
                        let non_pref: Vec<_> = mems
                            .iter()
                            .filter(|m| m.get("kind").and_then(Value::as_str) != Some("preference"))
                            .collect();
                        if !non_pref.is_empty() {
                            ctx.push_str("Cairn memory:\n");
                            for m in non_pref {
                                let kind = m.get("kind").and_then(Value::as_str).unwrap_or("note");
                                let content =
                                    m.get("content").and_then(Value::as_str).unwrap_or("");
                                ctx.push_str(&format!("- ({kind}) {content}\n"));
                            }
                        }
                    }
                }
                if !ctx.is_empty() {
                    emit(event, &ctx);
                }
            }
            "UserPromptSubmit" => {
                let prompt = payload.get("prompt").and_then(Value::as_str).unwrap_or("");
                if prompt.trim().is_empty() {
                    return Ok(());
                }
                // P1.8: default-off context injection. Opt-in via `CAIRN_INJECT_CONTEXT=true`.
                // Without this gate, every prompt burns ~1000 tokens on a /api/context/assemble
                // call - silent burn. Recording the prompt to memory still happens below
                // regardless, so the system stays useful even when injection is off.
                if inject_context_enabled() {
                    if let Ok(resp) = self
                        .get("/api/context/assemble")
                        .query("q", prompt)
                        .query("budget", "1200")
                        .call()
                    {
                        if let Ok(v) = resp.into_json::<Value>() {
                            if v.get("included")
                                .and_then(Value::as_array)
                                .is_some_and(|a| !a.is_empty())
                            {
                                if let Some(ctx) = v.get("context").and_then(Value::as_str) {
                                    if !ctx.is_empty() {
                                        emit(event, ctx);
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = self.post("/api/memory").send_json(json!({
                    "content": prompt,
                    "kind": "note",
                    "tier": "episodic",
                    "importance": 0.3
                }));
            }
            "SessionEnd" => {
                let _ = self.post("/api/memory/consolidate").send_json(json!({}));
            }
            _ => {
                // PostToolUse and other events are not proxied in remote-only mode.
            }
        }
        Ok(())
    }
}

/// Emit a context-injection payload on stdout per the agent hook contract.
fn emit(event: &str, context: &str) {
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "additionalContext": context,
        }
    });
    println!("{out}");
}

/// P1.8: opt-in context injection gate. Returns `true` only when the user has explicitly
/// enabled it via `CAIRN_INJECT_CONTEXT=true|1|yes|on`. Defaults to OFF so that the hook
/// doesn't silently burn ~1000 tokens per prompt when the user hasn't asked for it.
fn inject_context_enabled() -> bool {
    matches!(
        std::env::var("CAIRN_INJECT_CONTEXT").ok().as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env-var manipulation is process-global and not thread-safe in Rust.
    /// Serialize tests that touch the environment through this mutex.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env<T>(key: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var(key).ok();
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        let result = f();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        result
    }

    #[test]
    fn injection_disabled_when_env_unset() {
        with_env("CAIRN_INJECT_CONTEXT", None, || {
            assert!(!inject_context_enabled(), "default must be off");
        });
    }

    #[test]
    fn injection_enabled_when_env_true() {
        for v in ["true", "1", "yes", "on"] {
            with_env("CAIRN_INJECT_CONTEXT", Some(v), || {
                assert!(inject_context_enabled(), "{v} should enable injection");
            });
        }
    }

    #[test]
    fn injection_disabled_for_unrecognized_values() {
        for v in ["", "false", "0", "no", "off", "TRUE"] {
            with_env("CAIRN_INJECT_CONTEXT", Some(v), || {
                assert!(
                    !inject_context_enabled(),
                    "{v:?} should NOT enable injection (case-sensitive; only true/1/yes/on)"
                );
            });
        }
    }
}

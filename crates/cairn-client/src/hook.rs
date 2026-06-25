//! Claude Code lifecycle hook handler (`cairn hook <event>`).
//!
//! Claude Code invokes the configured command with a JSON payload on stdin and reads JSON on
//! stdout. We use that to make Cairn work automatically:
//!
//! - `SessionStart` injects your preferences + wakeup memory as additionalContext (never start
//!   cold). It also fires after a compaction (`source: "compact"`), so memory survives compaction.
//! - `UserPromptSubmit` injects an assembled, budgeted context block, records the prompt as
//!   episodic memory, and learns standing preferences stated in it.
//! - `PostToolUse` (Edit/Write) runs the silent-corruption guard against the version Cairn recorded
//!   when the file was read, warning if a large unreplaced deletion slipped in.
//! - `SessionEnd` consolidates the session's memory across tiers.
//!
//! Hooks must never break the agent: any internal error is logged to stderr and we still exit 0.

use crate::State;
use anyhow::Result;
use cairn_core::{Config, MemoryKind, MemoryTier, NewMemory};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;

pub fn run(cfg: &Config, event: &str) -> Result<()> {
    if let Err(e) = run_inner(cfg, event) {
        eprintln!("cairn hook: {e}");
    }
    Ok(())
}

fn run_inner(cfg: &Config, event: &str) -> Result<()> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(input.trim()).unwrap_or(Value::Null);

    if let Some(rc) = RemoteClient::new() {
        return run_remote(&rc, event, &payload);
    }

    let state = State::open(cfg)?;

    match event {
        "SessionStart" => {
            let mut ctx = String::new();
            if let Some(goal) = state.guard.anchor()? {
                ctx.push_str(&format!("Current task: {goal}\n\n"));
            }
            let prof = state.profile.block()?;
            if !prof.is_empty() {
                ctx.push_str(&prof);
                ctx.push('\n');
            }
            // Preferences are already shown in the profile block above; list the rest.
            let lines: Vec<String> = state
                .mem
                .wakeup(12)?
                .into_iter()
                .filter(|m| m.kind != MemoryKind::Preference)
                .map(|m| format!("- ({}) {}\n", m.kind.as_str(), m.content))
                .collect();
            if !lines.is_empty() {
                ctx.push_str("Cairn memory - what you already know here:\n");
                for l in lines {
                    ctx.push_str(&l);
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
            // Inject prior knowledge relevant to the prompt (assembled before recording the
            // current prompt, so we surface history rather than echoing the prompt back).
            let report = state.asm.assemble(prompt, 1200)?;
            if !report.included.is_empty() {
                emit(event, &report.context);
            }
            // Record the intent as a low-importance episodic memory (dedup handles repeats).
            let mut nm = NewMemory::new(prompt);
            nm.kind = Some(MemoryKind::Note);
            nm.tier = Some(MemoryTier::Episodic);
            nm.importance = Some(0.3);
            let _ = state.mem.remember(nm);
            // Learn standing preferences stated in the prompt ("always use X", ...).
            let _ = state.profile.capture_from_prompt(prompt);
        }
        "PostToolUse" => {
            let tool = payload
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("");
            if matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "StrReplace") {
                if let Some(file) = payload
                    .get("tool_input")
                    .and_then(|t| t.get("file_path"))
                    .and_then(Value::as_str)
                {
                    if let Some(report) = state.guard.verify_against_baseline(Path::new(file))? {
                        // Record the outcome (clean or not) so the reliability score reflects it.
                        let _ = state.guard.note_verify(&report);
                        if !report.is_clean() {
                            let ctx = format!(
                                "[!] Cairn guard ({:?}): {}. The pre-edit original is retained - recover it with Cairn `expand {}` if this was unintended.",
                                report.risk,
                                report.message,
                                report.baseline_hash.as_deref().unwrap_or("")
                            );
                            emit(event, &ctx);
                        }
                    }
                }
            }
        }
        "SessionEnd" => {
            // Turn the session's transient working memory into durable tiers.
            let _ = state.mem.consolidate();
        }
        _ => {}
    }
    Ok(())
}

// -------------------------------------------------------------------------
// Remote-proxy mode: when CAIRN_SERVER is set the client has no local store.
// We replicate the hook behaviour over HTTP instead.
// -------------------------------------------------------------------------

struct RemoteClient {
    server: String,
    token: Option<String>,
}

impl RemoteClient {
    fn new() -> Option<Self> {
        let server = std::env::var("CAIRN_SERVER").ok()?;
        if server.trim().is_empty() {
            return None;
        }
        Some(Self {
            server,
            token: std::env::var("CAIRN_TOKEN")
                .ok()
                .filter(|t| !t.is_empty()),
        })
    }

    fn get(&self, path: &str) -> ureq::Request {
        let url = format!("{}{}", self.server, path);
        let req = ureq::get(&url);
        if let Some(t) = &self.token {
            req.set("Authorization", &format!("Bearer {t}"))
        } else {
            req
        }
    }

    fn post(&self, path: &str) -> ureq::Request {
        let url = format!("{}{}", self.server, path);
        let req = ureq::post(&url);
        if let Some(t) = &self.token {
            req.set("Authorization", &format!("Bearer {t}"))
        } else {
            req
        }
    }
}

fn run_remote(rc: &RemoteClient, event: &str, payload: &Value) -> Result<()> {
    match event {
        "SessionStart" => {
            let mut ctx = String::new();
            if let Ok(resp) = rc.get("/api/guard/anchor").call() {
                if let Ok(v) = resp.into_json::<Value>() {
                    if let Some(anchor) = v.get("anchor").and_then(Value::as_str) {
                        ctx.push_str(&format!("Current task: {anchor}\n\n"));
                    }
                }
            }
            if let Ok(resp) = rc.get("/api/profile").call() {
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
            if let Ok(resp) = rc.get("/api/memory/wakeup").query("limit", "12").call() {
                if let Ok(mems) = resp.into_json::<Vec<Value>>() {
                    let non_pref: Vec<_> = mems
                        .iter()
                        .filter(|m| {
                            m.get("kind").and_then(Value::as_str) != Some("preference")
                        })
                        .collect();
                    if !non_pref.is_empty() {
                        ctx.push_str("Cairn memory - what you already know here:\n");
                        for m in non_pref {
                            let kind =
                                m.get("kind").and_then(Value::as_str).unwrap_or("note");
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
            if let Ok(resp) = rc
                .get("/api/context/assemble")
                .query("q", prompt)
                .query("budget", "1200")
                .call()
            {
                if let Ok(v) = resp.into_json::<Value>() {
                    let has_results = v
                        .get("included")
                        .and_then(Value::as_array)
                        .is_some_and(|a| !a.is_empty());
                    if has_results {
                        if let Some(ctx) = v.get("context").and_then(Value::as_str) {
                            if !ctx.is_empty() {
                                emit(event, ctx);
                            }
                        }
                    }
                }
            }
            let _ = rc.post("/api/memory").send_json(json!({
                "content": prompt,
                "kind": "note",
                "tier": "episodic",
                "importance": 0.3
            }));
        }
        // PostToolUse: no local baseline in remote mode; skip silently.
        "SessionEnd" => {
            let _ = rc.post("/api/memory/consolidate").send_json(json!({}));
        }
        _ => {}
    }
    Ok(())
}

/// Emit a context-injection payload on stdout per the Claude Code hook contract.
fn emit(event: &str, context: &str) {
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "additionalContext": context,
        }
    });
    println!("{out}");
}

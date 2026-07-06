//! AI agent lifecycle hook handler (`cairn hook <event>`).
//!
//! Supports Claude Code, Codex CLI, and OpenCode (via plugin bridge).
//! Reads JSON payload from stdin, calls the Cairn server HTTP API, and
//! emits additionalContext JSON on stdout per the agent's hook contract.
//!
//! Hooks must never break the agent: errors go to stderr, exit code is
//! always 0.

use crate::debuglog::DebugLog;
use crate::project::{current_dir_str, detect_project};
use crate::spool;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Instant;

pub fn run(event: &str) -> Result<()> {
    if let Err(e) = run_inner(event) {
        eprintln!("cairn hook: {e}");
    }
    Ok(())
}

fn run_inner(event: &str) -> Result<()> {
    let overall_start = Instant::now();

    // v0.8.0 Sprint 3: detect the project on every hook invocation (each `cairn hook <event>`
    // call is its own process - there's no in-memory state to carry a project id across
    // SessionStart and later events), so every request this process makes carries
    // `X-Cairn-Project` and gets scoped recall/remember for free (Sprint 2's scope model). It
    // also feeds `config::resolve`'s per-project `[projects."<id>"]` override lookup.
    let (project_id, project_name) = detect_project();
    if project_id.is_none() {
        // Only possible when `std::env::current_dir()` itself fails (cwd deleted mid-process) -
        // `detect_project` already falls back through git-root and cwd-basename first. Memories
        // this hook writes will land in Global scope instead of Project scope.
        eprintln!("cairn hook: no project detected (cwd missing?) - writing to GLOBAL scope");
    }
    let resolved = crate::config::resolve(project_id.as_deref());
    let mut debug_log = DebugLog::new(resolved.debug);

    let (Some((server, _)), Some((token, _))) = (&resolved.server, &resolved.token) else {
        // v0.8.0 client redesign: before `~/.cairn/config.toml` existed, `cairn setup` only
        // ever embedded CAIRN_SERVER/CAIRN_TOKEN into the *agent's* MCP entry env - invisible
        // to this separately-spawned hook process - so hooks were silently broken unless the
        // user also exported the vars in their shell profile. `config::resolve` closes that
        // gap for anyone who has re-run `setup`/`onboard`/`pair` since; this message is now
        // the true "nothing is configured at all" case.
        eprintln!(
            "cairn hook: no server/token configured (env, ~/.cairn/config.toml, or agent config). Hook skipped."
        );
        return Ok(());
    };

    if event == "SessionStart" {
        // v0.8.0 Sprint 9: drain anything queued by `post_spooled` while the server was
        // unreachable. Only on `SessionStart` - it's the one event guaranteed to happen before a
        // session's other hooks fire, and there's no value in paying a network round-trip for this
        // on every single prompt.
        crate::spool::replay(server, token);
        // A session that never reached SessionEnd/PreCompact (crash, force-quit) leaks its
        // touched-file buffer forever otherwise; sweep once per new session instead of on a timer.
        crate::sessionbuf::sweep_orphans();
    }

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload: Value = serde_json::from_str(input.trim()).unwrap_or(Value::Null);

    let mut rc = RemoteClient::new(
        server,
        token,
        std::time::Duration::from_millis(resolved.timeout_ms),
    );
    rc.project_id = project_id.clone();
    // v0.8.0 Sprint 5: every hook JSON payload carries the agent's session id - forward it as
    // `X-Cairn-Session` so `SessionEnd`'s session-summary call (and any future session-scoped
    // request) hits the right session without a second round-trip to look it up.
    rc.session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(String::from);

    if event == "SessionStart" {
        if let Some(pid) = &project_id {
            rc.patch_spooled(
                "/api/projects/upsert",
                json!({
                    "id": pid,
                    "name": project_name,
                    "path": current_dir_str(),
                }),
                &mut debug_log,
            );
        }
    }

    let result = rc.dispatch(
        event,
        &payload,
        resolved.inject_context.0,
        resolved.guard,
        &mut debug_log,
    );
    debug_log.flush(
        event,
        project_id.as_deref(),
        overall_start.elapsed().as_millis(),
    );
    result
}

struct RemoteClient {
    agent: ureq::Agent,
    server: String,
    token: String,
    project_id: Option<String>,
    session_id: Option<String>,
}

impl RemoteClient {
    fn new(server: &str, token: &str, timeout: std::time::Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(timeout)
            .timeout(timeout)
            .build();
        Self {
            agent,
            server: server.trim_end_matches('/').to_string(),
            token: token.to_string(),
            project_id: None,
            session_id: None,
        }
    }

    fn get(&self, path: &str) -> ureq::Request {
        let req = self
            .agent
            .get(&format!("{}{}", self.server, path))
            .set("Authorization", &format!("Bearer {}", self.token));
        self.with_scope_headers(req)
    }

    fn post(&self, path: &str) -> ureq::Request {
        let req = self
            .agent
            .post(&format!("{}{}", self.server, path))
            .set("Authorization", &format!("Bearer {}", self.token));
        self.with_scope_headers(req)
    }

    fn patch(&self, path: &str) -> ureq::Request {
        let req = self
            .agent
            .patch(&format!("{}{}", self.server, path))
            .set("Authorization", &format!("Bearer {}", self.token));
        self.with_scope_headers(req)
    }

    fn with_scope_headers(&self, req: ureq::Request) -> ureq::Request {
        let req = match &self.project_id {
            Some(pid) => req.set("X-Cairn-Project", pid),
            None => req,
        };
        match &self.session_id {
            Some(sid) => req.set("X-Cairn-Session", sid),
            None => req,
        }
    }

    /// `GET path?query...`, timed and (when enabled) recorded to `log`. Takes
    /// `&self` (like `get`/`post`) and an independent `&mut DebugLog` - the
    /// two never alias, so no interior mutability is needed here.
    fn timed_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        log: &mut DebugLog,
    ) -> Result<ureq::Response, Box<ureq::Error>> {
        let start = Instant::now();
        let mut req = self.get(path);
        for (k, v) in query {
            req = req.query(k, v);
        }
        let result = req.call();
        if log.is_enabled() {
            let ms = start.elapsed().as_millis();
            let line = match &result {
                Ok(resp) => format!("GET {path} -> {} ({ms}ms)", resp.status()),
                Err(e) => format!("GET {path} -> error: {e} ({ms}ms)"),
            };
            log.record(line);
        }
        result.map_err(Box::new)
    }

    /// POST `body` to `path` - see `send_spooled` for the shared behavior.
    fn post_spooled(&self, path: &str, body: Value, log: &mut DebugLog) {
        self.send_spooled("POST", path, body, log);
    }

    /// PATCH `body` to `path` - see `send_spooled` for the shared behavior.
    /// (`/api/projects/upsert` is a PATCH endpoint; using `post_spooled` on it
    /// used to 405 on every single `SessionStart`, silently, since nothing
    /// ever surfaced hook stderr - caught by the new debug-log timing output.)
    fn patch_spooled(&self, path: &str, body: Value, log: &mut DebugLog) {
        self.send_spooled("PATCH", path, body, log);
    }

    /// Send `body` to `path` via `method` (`POST` or `PATCH`), scoped to this client's current
    /// project/session. Identical to a plain synchronous call on success or an HTTP error
    /// response - either way the server was reached, so there's nothing more to do. Only a
    /// genuine connectivity failure (`ureq::Error::Transport` - no response at all) queues the
    /// request to `~/.cairn/spool.jsonl` (v0.8.0 Sprint 9) for a future `SessionStart` to replay,
    /// instead of the content being silently and permanently dropped like every other
    /// best-effort hook call here.
    fn send_spooled(&self, method: &str, path: &str, body: Value, log: &mut DebugLog) {
        let start = Instant::now();
        let req = match method {
            "PATCH" => self.patch(path),
            _ => self.post(path),
        };
        let result = req.send_json(body.clone());
        if log.is_enabled() {
            let ms = start.elapsed().as_millis();
            let line = match &result {
                Ok(resp) => format!("{method} {path} -> {} ({ms}ms)", resp.status()),
                Err(e) => format!("{method} {path} -> error: {e} ({ms}ms)"),
            };
            log.record(line);
        }
        if let Err(ureq::Error::Transport(_)) = result {
            spool::append(&spool::SpoolEntry {
                path: path.to_string(),
                method: method.to_string(),
                body,
                project_id: self.project_id.clone(),
                session_id: self.session_id.clone(),
                ts: chrono::Utc::now(),
            });
        }
    }

    /// Shared by `SessionEnd` and `PreCompact`: consolidate, flush this
    /// session's touched-file buffer as one session-scoped memory, and ask
    /// the server to synthesize a session summary. `PreCompact` fires this
    /// BEFORE Claude Code destroys/summarizes its context window - the point
    /// of Cairn's memory model is that it survives context death, so this is
    /// where that promise gets made real instead of aspirational (previously
    /// nothing fired at compaction at all).
    fn flush_session_state(&self, log: &mut DebugLog) {
        self.post_spooled("/api/memory/consolidate", json!({}), log);
        if let Some(sid) = &self.session_id {
            let files = crate::sessionbuf::drain(sid);
            if !files.is_empty() {
                let content = format!(
                    "Files touched this session:\n{}",
                    files
                        .iter()
                        .map(|f| format!("- {f}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                self.post_spooled(
                    "/api/memory",
                    json!({
                        "title": format!("Files touched this session ({})", files.len()),
                        "content": content,
                        "kind": "note",
                        "tier": "episodic",
                        "importance": 0.3,
                        "scope_type": "session"
                    }),
                    log,
                );
            }
        }
        // v0.8.0 Sprint 5: ask the server to synthesize this session's memories into a
        // project-scoped summary. `X-Cairn-Session`/`X-Cairn-Project` (set above) tell
        // the server which session/project - no body needed. Best-effort like every
        // other hook call: a disabled LLM just skips it; a network hiccup spools it
        // (v0.8.0 Sprint 9) for the next `SessionStart` to retry.
        self.post_spooled("/api/memory/session-summary", json!({}), log);
    }

    /// POST `body` to `path` and parse the JSON response - unlike `post_spooled`, this is a
    /// synchronous permission-decision input, not a fire-and-forget mutation, so a failure
    /// (timeout, non-2xx, unparseable body) has nothing meaningful to retry later: `None` here
    /// always means "couldn't get an answer," and every caller treats that as fail-open.
    fn post_json(&self, path: &str, body: Value, log: &mut DebugLog) -> Option<Value> {
        let start = Instant::now();
        let result = self.post(path).send_json(body);
        if log.is_enabled() {
            let ms = start.elapsed().as_millis();
            let line = match &result {
                Ok(resp) => format!("POST {path} -> {} ({ms}ms)", resp.status()),
                Err(e) => format!("POST {path} -> error: {e} ({ms}ms)"),
            };
            log.record(line);
        }
        result.ok()?.into_json().ok()
    }

    /// v0.8.0 Sprint 10 (C-2): opt-in real-time guard for `PreToolUse`. Returns
    /// `Some((decision, reason))` only when the caller should actually say something
    /// (`"ask"` - Claude Code has no `"warn"` decision, so a `Risk::Warn`/borderline sanitize
    /// result is deliberately treated as `None`/allow, matching the spec's "danger verdict ->
    /// ask" rather than escalating every warning). `None` covers every fail-open path:
    /// unrecognized tool, a file that doesn't exist yet, or a server that didn't answer in time.
    fn guard_check(&self, payload: &Value, log: &mut DebugLog) -> Option<(&'static str, String)> {
        let tool_name = payload.get("tool_name").and_then(Value::as_str)?;
        let tool_input = payload.get("tool_input")?;

        match tool_name {
            "Write" => {
                let path = tool_input.get("file_path").and_then(Value::as_str)?;
                let content = tool_input.get("content").and_then(Value::as_str)?;
                self.verify_content(path, content, log)
            }
            "Edit" => {
                let path = tool_input.get("file_path").and_then(Value::as_str)?;
                let old_string = tool_input.get("old_string").and_then(Value::as_str)?;
                let new_string = tool_input.get("new_string").and_then(Value::as_str)?;
                // Simulate the edit's result to verify BEFORE it happens. This is an
                // approximation, not a re-implementation of Claude Code's own Edit semantics
                // (e.g. it doesn't require old_string to be unique) - good enough to catch a
                // genuinely large unreplaced deletion, which is what `verify_edit` actually
                // measures. If old_string isn't even present, the real tool call would itself
                // fail, so there's nothing useful to check - fail open.
                let current = std::fs::read_to_string(path).ok()?;
                if !current.contains(old_string) {
                    return None;
                }
                let hypothetical = current.replacen(old_string, new_string, 1);
                self.verify_content(path, &hypothetical, log)
            }
            "Bash" => {
                let command = tool_input.get("command").and_then(Value::as_str)?;
                self.sanitize_command(command, log)
            }
            // MultiEdit/NotebookEdit and anything else: not supported in this first pass -
            // fail open rather than guess at a multi-step edit's combined result.
            _ => None,
        }
    }

    fn verify_content(
        &self,
        path: &str,
        content: &str,
        log: &mut DebugLog,
    ) -> Option<(&'static str, String)> {
        let resp = self.post_json(
            "/api/guard/verify",
            json!({ "path": path, "content": content }),
            log,
        )?;
        if resp.get("risk").and_then(Value::as_str)? != "danger" {
            return None;
        }
        let message = resp
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("large unreplaced deletion detected");
        Some((
            "ask",
            format!("Cairn guard flagged this edit as high-risk: {message}"),
        ))
    }

    fn sanitize_command(
        &self,
        command: &str,
        log: &mut DebugLog,
    ) -> Option<(&'static str, String)> {
        let resp = self.post_json("/api/share/sanitize", json!({ "text": command }), log)?;
        if resp.get("sensitivity").and_then(Value::as_str)? == "shareable" {
            return None;
        }
        let count = resp
            .get("findings")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        Some((
            "ask",
            format!(
                "Cairn guard detected {count} possible secret(s)/PII in this command before it runs"
            ),
        ))
    }

    fn dispatch(
        &self,
        event: &str,
        payload: &Value,
        inject_context: bool,
        guard: bool,
        log: &mut DebugLog,
    ) -> Result<()> {
        match event {
            // v0.8.0 Sprint 10 (C-2): opt-in real-time guard, default off. Silence (no
            // stdout) is an implicit "allow" per the hook contract, so every early-return
            // path below - gate off, unsupported tool, network failure - fails open without
            // needing to say so explicitly.
            "PreToolUse" if guard => {
                if let Some((decision, reason)) = self.guard_check(payload, log) {
                    emit_permission_decision(event, decision, &reason);
                }
            }
            "SessionStart" => {
                let mut ctx = String::new();
                if let Ok(resp) = self.timed_get("/api/guard/anchor", &[], log) {
                    if let Ok(v) = resp.into_json::<Value>() {
                        if let Some(anchor) = v.get("anchor").and_then(Value::as_str) {
                            ctx.push_str(&format!("Current task: {anchor}\n\n"));
                        }
                    }
                }
                // v0.8.0 Sprint 8: "since you were away" - what autopilot (promotion, drift)
                // did overnight, so a human learns about it here instead of having to visit
                // the dashboard. Omitted entirely when every count is zero - a quiet night
                // shouldn't manufacture a line of noise in every session's context.
                if let Ok(resp) = self.timed_get("/api/memory/autopilot-digest", &[], log) {
                    if let Ok(v) = resp.into_json::<Value>() {
                        let promoted = v.get("promoted").and_then(Value::as_u64).unwrap_or(0);
                        let demoted = v.get("demoted").and_then(Value::as_u64).unwrap_or(0);
                        let drift = v
                            .get("drift_auto_approved")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        if promoted + demoted + drift > 0 {
                            ctx.push_str(&format!(
                                "Since you were away: {promoted} memories auto-promoted, \
                                 {demoted} demoted, {drift} drift events auto-approved.\n\n"
                            ));
                        }
                    }
                }
                if let Ok(resp) = self.timed_get("/api/profile", &[], log) {
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
                if let Ok(resp) = self.timed_get("/api/memory/wakeup", &[("limit", "12")], log) {
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
                // v0.8.0 Sprint 8: derive a task anchor from the first prompt if none is set
                // yet. Cheap to call on every prompt - the server no-ops immediately once an
                // anchor already exists (manual or auto-derived), so this is a live network
                // call only on the very first prompt of a session.
                self.post_spooled("/api/guard/anchor/auto", json!({ "prompt": prompt }), log);
                // P1.8: default-off context injection unless `config::resolve` says otherwise
                // (env `CAIRN_INJECT_CONTEXT`, a per-project override, or the global
                // `[hooks] inject_context` a fresh `pair`/`onboard`/`setup` now writes to
                // `~/.cairn/config.toml`). Without this gate, every prompt burns ~1000 tokens
                // on a /api/context/assemble call - silent burn. Recording the prompt to memory
                // still happens below regardless, so the system stays useful either way.
                if inject_context {
                    if let Ok(resp) = self.timed_get(
                        "/api/context/assemble",
                        &[("q", prompt), ("budget", "1200")],
                        log,
                    ) {
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
                self.post_spooled(
                    "/api/memory",
                    json!({
                        "content": prompt,
                        "kind": "note",
                        "tier": "episodic",
                        "importance": 0.3
                    }),
                    log,
                );
            }
            "PostToolUse" => {
                // v0.8.0 client redesign: previously a documented no-op fired on every single
                // edit. Zero network cost on this hot path - just append to a local per-session
                // buffer; `SessionEnd`/`PreCompact` flush the deduped list as one memory.
                if let (Some(sid), Some(path)) = (
                    &self.session_id,
                    crate::sessionbuf::extract_touched_path(payload),
                ) {
                    crate::sessionbuf::record_touch(sid, &path);
                }
            }
            "SessionEnd" | "PreCompact" => {
                self.flush_session_state(log);
            }
            _ => {}
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

/// Emit a `PreToolUse` permission decision on stdout. `decision` is `"ask"` or `"deny"` -
/// Cairn's guard (C-2) only ever emits `"ask"`, never `"deny"`: it surfaces a risk for a human
/// or the agent to weigh in on, it doesn't unilaterally block anything.
fn emit_permission_decision(event: &str, decision: &str, reason: &str) {
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }
    });
    println!("{out}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;

    /// Accepts exactly one HTTP connection, fully drains the request, and replies with a fixed
    /// 200 JSON body. No new dev-dependency: a bare `std::net::TcpListener` is enough to stand
    /// in for `/api/guard/verify` / `/api/share/sanitize` for these tests.
    ///
    /// Draining matters: `ureq` can write the request in more than one `write()` call, and
    /// responding after only a partial read raced the client's own write on occasion (observed
    /// as an intermittent, fast - not timed-out - failure). A short read timeout here means
    /// "no more bytes arriving right now" reliably signals the client finished sending, without
    /// needing to actually parse Content-Length out of the request.
    fn canned_json_server(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_millis(200)))
                    .ok();
                let mut buf = [0u8; 4096];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => continue,
                        Err(_) => break, // timed out with no more data - request is fully sent
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn client(server: &str) -> RemoteClient {
        RemoteClient::new(server, "test-token", std::time::Duration::from_secs(3))
    }

    #[test]
    fn verify_content_asks_on_danger_risk() {
        let server =
            canned_json_server(r#"{"risk":"danger","message":"large unreplaced deletion"}"#);
        let rc = client(&server);
        let mut log = DebugLog::new(false);
        assert_eq!(
            rc.verify_content("/tmp/foo.rs", "new content", &mut log),
            Some((
                "ask",
                "Cairn guard flagged this edit as high-risk: large unreplaced deletion".to_string()
            ))
        );
    }

    #[test]
    fn verify_content_allows_on_ok_or_warn_risk() {
        for risk in ["ok", "warn"] {
            let server = canned_json_server(Box::leak(
                format!(r#"{{"risk":"{risk}","message":"fine"}}"#).into_boxed_str(),
            ));
            let rc = client(&server);
            let mut log = DebugLog::new(false);
            assert_eq!(
                rc.verify_content("/tmp/foo.rs", "new content", &mut log),
                None,
                "risk={risk} must not trigger ask - only danger does"
            );
        }
    }

    #[test]
    fn sanitize_command_asks_when_not_shareable() {
        let server = canned_json_server(
            r#"{"text":"[REDACTED]","findings":[{"kind":"api_key","start":0,"end":5}],"sensitivity":"private"}"#,
        );
        let rc = client(&server);
        let mut log = DebugLog::new(false);
        assert_eq!(
            rc.sanitize_command("export KEY=abc123", &mut log),
            Some((
                "ask",
                "Cairn guard detected 1 possible secret(s)/PII in this command before it runs"
                    .to_string()
            ))
        );
    }

    #[test]
    fn sanitize_command_allows_when_shareable() {
        let server =
            canned_json_server(r#"{"text":"ls -la","findings":[],"sensitivity":"shareable"}"#);
        let rc = client(&server);
        let mut log = DebugLog::new(false);
        assert_eq!(rc.sanitize_command("ls -la", &mut log), None);
    }

    #[test]
    fn guard_check_fails_open_when_server_is_unreachable() {
        // Port 1 is reserved and refuses connections immediately (no timeout wait).
        let rc = client("http://127.0.0.1:1");
        let mut log = DebugLog::new(false);
        let payload = json!({
            "tool_name": "Write",
            "tool_input": { "file_path": "/tmp/x.rs", "content": "hello" }
        });
        assert_eq!(rc.guard_check(&payload, &mut log), None);
    }

    #[test]
    fn guard_check_fails_open_for_an_unsupported_tool() {
        let rc = client("http://127.0.0.1:1");
        let mut log = DebugLog::new(false);
        let payload = json!({ "tool_name": "MultiEdit", "tool_input": {} });
        assert_eq!(rc.guard_check(&payload, &mut log), None);
    }

    #[test]
    fn guard_check_edit_simulates_the_replacement_before_verifying() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.rs");
        std::fs::write(&file, "line one\nline two\nline three\n").unwrap();
        let server = canned_json_server(r#"{"risk":"danger","message":"test"}"#);
        let rc = client(&server);
        let mut log = DebugLog::new(false);
        let payload = json!({
            "tool_name": "Edit",
            "tool_input": {
                "file_path": file.to_string_lossy(),
                "old_string": "line two",
                "new_string": "line TWO edited"
            }
        });
        assert_eq!(
            rc.guard_check(&payload, &mut log),
            Some((
                "ask",
                "Cairn guard flagged this edit as high-risk: test".to_string()
            ))
        );
    }

    #[test]
    fn guard_check_edit_fails_open_when_old_string_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.rs");
        std::fs::write(&file, "actual content\n").unwrap();
        let rc = client("http://127.0.0.1:1");
        let mut log = DebugLog::new(false);
        let payload = json!({
            "tool_name": "Edit",
            "tool_input": {
                "file_path": file.to_string_lossy(),
                "old_string": "not present anywhere",
                "new_string": "x"
            }
        });
        assert_eq!(rc.guard_check(&payload, &mut log), None);
    }

    #[test]
    fn guard_check_bash_routes_to_sanitize() {
        let server = canned_json_server(
            r#"{"text":"x","findings":[{"kind":"secret"}],"sensitivity":"private"}"#,
        );
        let rc = client(&server);
        let mut log = DebugLog::new(false);
        let payload = json!({
            "tool_name": "Bash",
            "tool_input": { "command": "curl -H 'Authorization: Bearer sk-secret'" }
        });
        assert!(rc.guard_check(&payload, &mut log).is_some());
    }
}

//! A minimal Model Context Protocol server over stdio.
//!
//! MCP's stdio transport is newline-delimited JSON-RPC 2.0: one JSON message per line on stdin,
//! one per line on stdout. (Logs must go to stderr so they don't corrupt the channel.) We
//! hand-roll it to avoid taking a heavy SDK dependency this early; the surface is small and the
//! protocol is stable.
//!
//! Tools exposed: `read`, `expand`, `remember`, `recall`, `wakeup`.

use cairn_assemble::Assembler;
use cairn_context::{ContextEngine, ReadMode};
use cairn_core::{Config, NewMemory, Result};
use cairn_guard::Guard;
use cairn_memory::MemoryEngine;
use cairn_profile::Profile;
use cairn_shell::ShellCompressor;
use cairn_store::Store;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::Arc;

/// Default protocol version we advertise if the client doesn't specify one.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub struct McpServer {
    ctx: Arc<ContextEngine>,
    guard: Arc<Guard>,
    asm: Arc<Assembler>,
    shell: Arc<ShellCompressor>,
    profile: Arc<Profile>,
    mem: Arc<MemoryEngine>,
}

impl McpServer {
    pub fn new(cfg: &Config) -> Result<Self> {
        let store = Arc::new(Store::open(cfg)?);
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        Ok(Self {
            ctx: Arc::new(ContextEngine::new(store.clone())),
            guard: Arc::new(Guard::new(store.clone())),
            asm: Arc::new(Assembler::new(mem.clone())),
            shell: Arc::new(ShellCompressor::new(store.clone())),
            profile: Arc::new(Profile::new(mem.clone())),
            mem,
        })
    }

    /// Run the stdio loop until stdin closes. Never writes anything but protocol JSON to stdout.
    pub fn serve_stdio(&self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let mut locked = stdin.lock();
        let mut line = String::new();
        loop {
            line.clear();
            if locked.read_line(&mut line)? == 0 {
                break; // EOF
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let req: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("cairn-mcp: ignoring unparseable message: {e}");
                    continue;
                }
            };
            if let Some(resp) = self.handle(&req) {
                stdout.write_all(serde_json::to_string(&resp)?.as_bytes())?;
                stdout.write_all(b"\n")?;
                stdout.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one JSON-RPC message. Returns `None` for notifications (no reply expected).
    fn handle(&self, req: &Value) -> Option<Value> {
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        match method {
            "initialize" => {
                let ver = req
                    .get("params")
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION)
                    .to_string();
                Some(ok(
                    id,
                    json!({
                        "protocolVersion": ver,
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "cairn", "version": env!("CARGO_PKG_VERSION") }
                    }),
                ))
            }
            "notifications/initialized" | "initialized" => None,
            "ping" => Some(ok(id, json!({}))),
            "tools/list" => Some(ok(id, json!({ "tools": tool_defs() }))),
            "tools/call" => Some(self.call_tool(id, req.get("params"))),
            other => id.map(|id| err(Some(id), -32601, &format!("method not found: {other}"))),
        }
    }

    fn call_tool(&self, id: Option<Value>, params: Option<&Value>) -> Value {
        let Some(params) = params else {
            return err(id, -32602, "missing params");
        };
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        match self.dispatch(name, &args) {
            Ok(text) => ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
            Err(msg) => ok(
                id,
                json!({ "content": [{ "type": "text", "text": format!("error: {msg}") }], "isError": true }),
            ),
        }
    }

    fn dispatch(&self, name: &str, args: &Value) -> std::result::Result<String, String> {
        match name {
            "read" => {
                let path = str_arg(args.get("path")).ok_or("missing 'path'")?;
                let mode = ReadMode::parse(str_arg(args.get("mode")));
                let r = self
                    .ctx
                    .read(std::path::Path::new(path), mode)
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())
            }
            "expand" => {
                let hash = str_arg(args.get("hash")).ok_or("missing 'hash'")?;
                self.ctx
                    .expand(hash)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "unknown handle".to_string())
            }
            "remember" => {
                let content = str_arg(args.get("content")).ok_or("missing 'content'")?;
                let mut nm = NewMemory::new(content);
                nm.kind = str_arg(args.get("kind")).and_then(|k| k.parse().ok());
                nm.tier = str_arg(args.get("tier")).and_then(|t| t.parse().ok());
                nm.importance = args
                    .get("importance")
                    .and_then(Value::as_f64)
                    .map(|i| i as f32);
                let m = self.mem.remember(nm).map_err(|e| e.to_string())?;
                Ok(format!(
                    "remembered {} ({}/{})",
                    m.id,
                    m.kind.as_str(),
                    m.tier.as_str()
                ))
            }
            "recall" => {
                let q = str_arg(args.get("query")).ok_or("missing 'query'")?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
                let hits = self.mem.recall(q, limit).map_err(|e| e.to_string())?;
                if hits.is_empty() {
                    return Ok("(no matches)".into());
                }
                let mut out = String::new();
                for h in hits {
                    out.push_str(&format!(
                        "[{:.2}] ({}) {}\n",
                        h.score,
                        h.memory.kind.as_str(),
                        h.memory.content
                    ));
                }
                Ok(out)
            }
            "wakeup" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(12) as usize;
                let ms = self.mem.wakeup(limit).map_err(|e| e.to_string())?;
                if ms.is_empty() {
                    return Ok("(no memories yet)".into());
                }
                let mut out = String::from("Cairn wakeup — what you already know:\n");
                for m in ms {
                    out.push_str(&format!("· ({}) {}\n", m.kind.as_str(), m.content));
                }
                Ok(out)
            }
            "prefer" => {
                let rule = str_arg(args.get("rule")).ok_or("missing 'rule'")?;
                let m = self.profile.prefer(rule).map_err(|e| e.to_string())?;
                Ok(format!("noted preference: {}", m.content))
            }
            "profile" => {
                let block = self.profile.block().map_err(|e| e.to_string())?;
                if block.is_empty() {
                    Ok("(no preferences recorded yet)".into())
                } else {
                    Ok(block)
                }
            }
            "compress" => {
                let command = str_arg(args.get("command")).ok_or("missing 'command'")?;
                let output = str_arg(args.get("output")).ok_or("missing 'output'")?;
                let c = self
                    .shell
                    .compress(command, output)
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&c).map_err(|e| e.to_string())
            }
            "consolidate" => {
                let n = self.mem.consolidate().map_err(|e| e.to_string())?;
                Ok(format!("consolidated memory: {n} promoted across tiers"))
            }
            "assemble" => {
                let query = str_arg(args.get("query")).ok_or("missing 'query'")?;
                let budget = args.get("budget").and_then(Value::as_u64).unwrap_or(2000) as usize;
                let r = self
                    .asm
                    .assemble(query, budget)
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())
            }
            "verify" => {
                let path = str_arg(args.get("path")).ok_or("missing 'path'")?;
                let content = str_arg(args.get("content")).ok_or("missing 'content'")?;
                let r = self
                    .guard
                    .verify_edit(std::path::Path::new(path), content)
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown tool: {other}")),
        }
    }
}

fn tool_defs() -> Value {
    json!([
        {
            "name": "read",
            "description": "Read a file through Cairn. Re-reading an unchanged file is nearly free; after edits you get only the diff. Returns a handle you can pass to `expand` for the full original — no context is ever lost.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read." },
                    "mode": { "type": "string", "enum": ["auto", "full"], "description": "auto (cache-aware) or full." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "expand",
            "description": "Recover the exact, byte-identical original for a handle returned by `read` (or any Cairn content hash).",
            "inputSchema": {
                "type": "object",
                "properties": { "hash": { "type": "string", "description": "The handle / content hash." } },
                "required": ["hash"]
            }
        },
        {
            "name": "remember",
            "description": "Save a durable memory so future sessions on any device recall it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "kind": { "type": "string", "enum": ["fact", "decision", "task", "preference", "gotcha", "note"] },
                    "tier": { "type": "string", "enum": ["working", "episodic", "semantic", "procedural"] },
                    "importance": { "type": "number", "minimum": 0, "maximum": 1 }
                },
                "required": ["content"]
            }
        },
        {
            "name": "recall",
            "description": "Recall relevant memories for a query (ranked by relevance + recency + importance).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 }
                },
                "required": ["query"]
            }
        },
        {
            "name": "assemble",
            "description": "Assemble a lean, edge-ordered working set for a query under a token budget — the anti-context-rot context block. Reports what was included and dropped (dropped items remain recoverable via recall).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "budget": { "type": "integer", "minimum": 1, "description": "Token budget (default 2000)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "wakeup",
            "description": "Session-start bootstrap: the highest-value memories (decisions, tasks, preferences) so you never start cold.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1 } }
            }
        },
        {
            "name": "prefer",
            "description": "Record a standing user preference (preferred stack, style, do/don'ts). Injected at session start so any model honors how you work.",
            "inputSchema": {
                "type": "object",
                "properties": { "rule": { "type": "string" } },
                "required": ["rule"]
            }
        },
        {
            "name": "profile",
            "description": "Show the user's recorded preferences (the profile block).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "compress",
            "description": "Compress verbose command/tool output (cargo, git, build logs, listings) into a compact view, retaining the exact original (recover with `expand`).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "output": { "type": "string" }
                },
                "required": ["command", "output"]
            }
        },
        {
            "name": "consolidate",
            "description": "Consolidate memory across the four tiers (working → episodic → semantic → procedural). Run at session end to turn transient notes into durable knowledge.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "verify",
            "description": "Verify a proposed new version of a file against the current one before writing. Flags large, unreplaced deletions (silent corruption) and retains the original so nothing is lost.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string", "description": "The proposed new full file content." }
                },
                "required": ["path", "content"]
            }
        }
    ])
}

/// Extract a string argument, if present.
fn str_arg(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::Config;

    fn server() -> (McpServer, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::resolve(Some(dir.path().join("data"))).unwrap();
        (McpServer::new(&cfg).unwrap(), dir)
    }

    #[test]
    fn initialize_echoes_version_and_lists_tools() {
        let (s, _d) = server();
        let init = s
            .handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}))
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "cairn");

        let list = s
            .handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "read"));
        assert!(tools.iter().any(|t| t["name"] == "remember"));
    }

    #[test]
    fn remember_then_recall_via_tools_call() {
        let (s, _d) = server();
        s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"remember","arguments":{"content":"cairn uses sqlite plus a blob store","kind":"decision"}}}))
            .unwrap();
        let resp = s
            .handle(
                &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"recall","arguments":{"query":"sqlite blob","limit":5}}}),
            )
            .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("sqlite"), "recall text was: {text}");
    }

    #[test]
    fn notifications_get_no_reply() {
        let (s, _d) = server();
        assert!(s
            .handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none());
    }
}

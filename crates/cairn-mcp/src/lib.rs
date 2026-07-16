//! A minimal Model Context Protocol server over stdio.
//!
//! MCP's stdio transport is newline-delimited JSON-RPC 2.0: one JSON message per line on stdin,
//! one per line on stdout. (Logs must go to stderr so they don't corrupt the channel.) We
//! hand-roll it to avoid taking a heavy SDK dependency this early; the surface is small and the
//! protocol is stable.
//!
//! Tools exposed: read/expand, remember/recall/wakeup/consolidate, assemble,
//! prefer/profile, anchor, verify/checkpoint/rollback, compress, sanitize,
//! search, memory_*, metrics, registry_search, proactive_recall.

#[cfg(feature = "engine")]
use cairn_assemble::Assembler;
#[cfg(feature = "engine")]
use cairn_context::{ContextEngine, ReadMode};
use cairn_core::ContentHash;
#[cfg(feature = "engine")]
use cairn_core::{Config, NewMemory, Result, ScopeCtx};
#[cfg(feature = "engine")]
use cairn_guard::Guard;
#[cfg(feature = "engine")]
use cairn_memory::MemoryEngine;
#[cfg(feature = "engine")]
use cairn_profile::Profile;
#[cfg(feature = "engine")]
use cairn_shell::ShellCompressor;
#[cfg(feature = "engine")]
use cairn_store::Store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
#[cfg(feature = "engine")]
use std::sync::Arc;
use std::sync::Mutex;

/// Default protocol version we advertise if the client doesn't specify one.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub mod guidance;
pub mod prompts;
pub mod resources;

#[cfg(feature = "engine")]
pub struct McpServer {
    pub ctx: Arc<ContextEngine>,
    pub guard: Arc<Guard>,
    pub asm: Arc<Assembler>,
    pub shell: Arc<ShellCompressor>,
    pub profile: Arc<Profile>,
    pub san: cairn_share::Sanitizer,
    pub mem: Arc<MemoryEngine>,
    pub store: Arc<cairn_store::Store>,
    pub registry: Option<Arc<cairn_registry::Registry>>,
    pub config: Config,
}

#[cfg(feature = "engine")]
impl McpServer {
    /// P3.3: is the LLM consolidation flag on? Cached to avoid repeated config reads.
    fn llm_consolidation_enabled(&self) -> bool {
        self.config.llm_consolidation.enabled
    }

    /// P3.3: clone of the LLM consolidation config (used to construct `QueryExpander`).
    fn llm_consolidation_config(&self) -> cairn_core::LlmConsolidationConfig {
        self.config.llm_consolidation.clone()
    }

    pub fn new(cfg: &Config) -> Result<Self> {
        let store = Arc::new(Store::open(cfg)?);
        Self::with_store(cfg, store)
    }

    /// Construct an `McpServer` from a caller-supplied `Arc<Store>`. Used by the hermetic
    /// test bucket to wire a fully in-memory store; production callers should keep using
    /// `new`.
    pub fn with_store(cfg: &Config, store: Arc<Store>) -> Result<Self> {
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        Ok(Self {
            ctx: Arc::new(ContextEngine::new_with_root(
                store.clone(),
                cfg.workspace_root.clone(),
            )),
            guard: Arc::new(Guard::new(store.clone())),
            asm: Arc::new(Assembler::new(mem.clone(), store.clone())),
            shell: Arc::new(ShellCompressor::new(store.clone())),
            profile: Arc::new(Profile::new(mem.clone())),
            san: cairn_share::Sanitizer::new(),
            mem,
            store: store.clone(),
            registry: cairn_registry::Registry::open(&cfg.data_dir)
                .map(Arc::new)
                .ok(),
            config: cfg.clone(),
        })
    }

    /// Construct an `McpServer` from pre-built shared engines — the production path for
    /// `/api/tools/call`. This avoids opening a fresh `Store` + `ContextEngine` + `Guard`
    /// per request (which would lose the in-memory read cache, file version baselines,
    /// and guard workspace scoping). All engines are shared `Arc` handles from `AppState`,
    /// so tool calls see the same cache and file version state as direct API handlers.
    #[allow(clippy::too_many_arguments)]
    pub fn from_engines(
        config: Config,
        store: Arc<Store>,
        ctx: Arc<ContextEngine>,
        guard: Arc<Guard>,
        mem: Arc<MemoryEngine>,
        asm: Arc<Assembler>,
        shell: Arc<ShellCompressor>,
        profile: Arc<Profile>,
        registry: Option<Arc<cairn_registry::Registry>>,
    ) -> Self {
        Self {
            ctx,
            guard,
            asm,
            shell,
            profile,
            san: cairn_share::Sanitizer::new(),
            mem,
            store,
            registry,
            config,
        }
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
                let capabilities = match &self.config.workspace_root {
                    Some(root) => json!({
                        "tools": {},
                        "workspaceRoot": root.display().to_string()
                    }),
                    None => json!({ "tools": {} }),
                };
                Some(ok(
                    id,
                    json!({
                        "protocolVersion": ver,
                        "capabilities": capabilities,
                        "serverInfo": { "name": "cairn", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": guidance::GUIDANCE_COMPACT
                    }),
                ))
            }
            "notifications/initialized" | "initialized" => None,
            "ping" => Some(ok(id, json!({}))),
            "tools/list" => Some(ok(id, json!({ "tools": tool_defs() }))),
            "tools/call" => Some(self.call_tool(id, req.get("params"))),
            "resources/list" => Some(ok(
                id,
                json!({
                    "resources": resources::resource_defs().iter().map(|r| json!({
                        "uri": r.uri,
                        "name": r.name,
                        "description": r.description,
                        "mimeType": r.mime_type,
                    })).collect::<Vec<_>>()
                }),
            )),
            "resources/read" => {
                let uri = req
                    .get("params")
                    .and_then(|p| p.get("uri"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match resources::read_resource(self, uri) {
                    Ok(v) => Some(ok(
                        id,
                        json!({ "contents": [{
                        "uri": uri,
                        "mimeType": if uri == "cairn://config/toml" { "text/plain" } else { "application/json" },
                        "text": v.to_string(),
                    }] }),
                    )),
                    Err(e) => Some(err(id, -32602, &e)),
                }
            }
            "prompts/list" => Some(ok(
                id,
                json!({
                    "prompts": prompts::prompt_defs().iter().map(|p| json!({
                        "name": p.name,
                        "description": p.description,
                        "arguments": p.arguments,
                    })).collect::<Vec<_>>()
                }),
            )),
            "prompts/get" => {
                let name = req
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match prompts::render_prompt(name) {
                    Ok(v) => Some(ok(id, v)),
                    Err(e) => Some(err(id, -32602, &e)),
                }
            }
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
        let scope = ScopeCtx::default();
        match self.dispatch(name, &args, &scope) {
            Ok(text) => ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
            Err(msg) if msg.contains("outside the workspace root") => {
                err(id, -32602, &format!("workspace root violation: {msg}"))
            }
            Err(msg) => ok(
                id,
                json!({ "content": [{ "type": "text", "text": format!("error: {msg}") }], "isError": true }),
            ),
        }
    }

    /// Confine a tool-supplied path to the workspace root, returning a JSON-RPC-friendly error
    /// string if it escapes. This is the MCP-layer gate; the same check is also enforced inside
    /// the context engine.
    fn resolve_tool_path(&self, raw: &str) -> std::result::Result<std::path::PathBuf, String> {
        self.ctx
            .resolve_path(std::path::Path::new(raw))
            .map_err(|e| e.to_string())
    }

    /// Dispatch a single tool call. Public so the HTTP API can expose the same tool surface.
    /// `scope` carries the current project/session from request headers (or `ScopeCtx::default()`
    /// for stdio-only paths). Write tools default to Project scope when a project is detected.
    pub fn dispatch(
        &self,
        name: &str,
        args: &Value,
        scope: &ScopeCtx,
    ) -> std::result::Result<String, String> {
        match name {
            "read" => {
                let path = str_arg(args.get("path")).ok_or("missing 'path'")?;
                let resolved = self.resolve_tool_path(path)?;
                let mode = ReadMode::parse(str_arg(args.get("mode")));
                let r = self.ctx.read(&resolved, mode).map_err(|e| e.to_string())?;
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
                nm.title = str_arg(args.get("title")).map(String::from);
                nm.reasoning = str_arg(args.get("reasoning")).map(String::from);
                nm.kind = str_arg(args.get("kind")).and_then(|k| k.parse().ok());
                nm.tier = str_arg(args.get("tier")).and_then(|t| t.parse().ok());
                nm.importance = args
                    .get("importance")
                    .and_then(Value::as_f64)
                    .map(|i| i as f32);
                nm.scope_type = str_arg(args.get("scope_type"))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| {
                        if scope.project_id.is_some() {
                            cairn_core::ScopeType::Project
                        } else {
                            cairn_core::ScopeType::Global
                        }
                    });
                nm.scope_id = str_arg(args.get("scope_id")).map(String::from).or_else(|| {
                    if nm.scope_type == cairn_core::ScopeType::Project {
                        scope.project_id.clone()
                    } else if nm.scope_type == cairn_core::ScopeType::Session {
                        scope.session_id.clone()
                    } else {
                        None
                    }
                });
                nm.concepts = args
                    .get("concepts")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                nm.files = args
                    .get("files")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                let m = self.mem.remember(nm).map_err(|e| e.to_string())?;
                Ok(format!(
                    "remembered {} ({}/{}/{})",
                    m.id,
                    m.kind.as_str(),
                    m.tier.as_str(),
                    m.scope_type.as_str()
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
                let mut out = String::from("Cairn wakeup - what you already know:\n");
                for m in ms {
                    out.push_str(&format!("- ({}) {}\n", m.kind.as_str(), m.content));
                }
                Ok(out)
            }
            "checkpoint" => {
                let label = str_arg(args.get("label")).unwrap_or("checkpoint");
                let cp = self.guard.checkpoint(label).map_err(|e| e.to_string())?;
                Ok(format!(
                    "checkpoint {} created ({} files tracked)",
                    cp.id, cp.files
                ))
            }
            "rollback" => {
                let id = str_arg(args.get("id")).ok_or("missing 'id'")?;
                let r = self.guard.rollback(id).map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())
            }
            "checkpoints" => {
                let cps = self.guard.list_checkpoints().map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&cps).map_err(|e| e.to_string())
            }
            "anchor" => match str_arg(args.get("goal")) {
                Some(goal) => {
                    self.guard.set_anchor(goal).map_err(|e| e.to_string())?;
                    Ok(format!("task anchor set: {goal}"))
                }
                None => Ok(self
                    .guard
                    .anchor()
                    .map_err(|e| e.to_string())?
                    .unwrap_or_else(|| "(no task anchor set)".to_string())),
            },
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
                let resolved = self.resolve_tool_path(path)?;
                let r = self
                    .guard
                    .verify_edit(&resolved, content)
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&r).map_err(|e| e.to_string())
            }
            "verify_baseline" => {
                let path = str_arg(args.get("path")).ok_or("missing 'path'")?;
                let resolved = self.resolve_tool_path(path)?;
                match self
                    .guard
                    .verify_against_baseline(&resolved)
                    .map_err(|e| e.to_string())?
                {
                    Some(report) => {
                        let _ = self.guard.note_verify(&report);
                        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
                    }
                    None => {
                        Ok("no baseline for this path (file was never read through Cairn)".into())
                    }
                }
            }
            "sanitize" => {
                let text = str_arg(args.get("text")).ok_or("missing 'text'")?;
                let s = self.san.sanitize(text);
                serde_json::to_string_pretty(&s).map_err(|e| e.to_string())
            }
            "proactive_recall" => {
                let prompt = str_arg(args.get("prompt")).ok_or("missing 'prompt'")?;
                let project_root = str_arg(args.get("project_root"));
                let (mems, reason) = proactive_recall(self, prompt, project_root);
                serde_json::to_string_pretty(&serde_json::json!({
                    "matches": mems,
                    "reason": reason,
                }))
                .map_err(|e| e.to_string())
            }
            // -- v0.5.0 Sprint 10: graph + memory CRUD extensions --
            "memory_edit" => {
                let id = str_arg(args.get("id")).ok_or("missing 'id'")?;
                let content = args
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let importance = args
                    .get("importance")
                    .and_then(Value::as_f64)
                    .map(|f| f as f32);
                let concepts = args.get("concepts").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                });
                let files = args.get("files").and_then(Value::as_array).map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                });
                let title = str_arg(args.get("title")).map(String::from);
                let reasoning = str_arg(args.get("reasoning")).map(String::from);
                match self
                    .mem
                    .edit(id, content, importance, concepts, files, title, reasoning)
                    .map_err(|e| e.to_string())?
                {
                    Some(m) => Ok(format!("edited {} (kind={})", m.id, m.kind.as_str())),
                    None => Err("no such memory".into()),
                }
            }
            "memory_delete" => {
                let id = str_arg(args.get("id")).ok_or("missing 'id'")?;
                if self.mem.delete(id).map_err(|e| e.to_string())? {
                    Ok(format!("deleted {id}"))
                } else {
                    Err("no such memory".into())
                }
            }
            "memory_pin" => {
                let id = str_arg(args.get("id")).ok_or("missing 'id'")?;
                let pinned = args.get("pinned").and_then(Value::as_bool).unwrap_or(true);
                if self.mem.pin(id, pinned).map_err(|e| e.to_string())? {
                    Ok(format!(
                        "{pinned_status} {id}",
                        pinned_status = if pinned { "pinned" } else { "unpinned" }
                    ))
                } else {
                    Err("no such memory".into())
                }
            }
            "memory_promote" => {
                let id = str_arg(args.get("id")).ok_or("missing 'id'")?;
                let tier = str_arg(args.get("tier")).ok_or("missing 'tier'")?;
                let target: cairn_core::MemoryTier =
                    tier.parse().map_err(|e: cairn_core::Error| e.to_string())?;
                match self.mem.get(id).map_err(|e| e.to_string())? {
                    Some(mut m) => {
                        m.tier = target;
                        let updated = self.store.upsert_memory(&m).map_err(|e| e.to_string())?;
                        Ok(format!("tier promoted to {}: {}", target.as_str(), updated))
                    }
                    None => Err("no such memory".into()),
                }
            }
            "memory_reinforce" => {
                let id = str_arg(args.get("id")).ok_or("missing 'id'")?;
                self.store.reinforce_memory(id).map_err(|e| e.to_string())?;
                Ok(format!("reinforced {id}"))
            }
            "memory_timeline" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                let mut mems = self.store.all_memories().map_err(|e| e.to_string())?;
                mems.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
                mems.truncate(limit);
                serde_json::to_string_pretty(&mems).map_err(|e| e.to_string())
            }
            "memory_crystallize" => match self.mem.crystallize(None).map_err(|e| e.to_string())? {
                Some(id) => Ok(format!("crystallized: {id}")),
                None => Ok("nothing to crystallize".into()),
            },
            "memory_graph" => {
                let g = self.mem.graph().map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&g).map_err(|e| e.to_string())
            }
            "search" => {
                let query = str_arg(args.get("query")).ok_or("missing 'query'")?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
                let expand = args.get("expand").and_then(Value::as_bool).unwrap_or(false);
                let hits = if expand && self.llm_consolidation_enabled() {
                    let expander =
                        cairn_memory::QueryExpander::new(self.llm_consolidation_config().clone());
                    self.mem
                        .expanded_search(query, limit, 20, &expander)
                        .map_err(|e| e.to_string())?
                } else {
                    self.mem
                        .hybrid_search(query, limit, 20)
                        .map_err(|e| e.to_string())?
                };
                serde_json::to_string_pretty(&hits).map_err(|e| e.to_string())
            }
            "metrics" => {
                let mem_count = self.store.count_memories().map_err(|e| e.to_string())?;
                let cp_count = self
                    .guard
                    .list_checkpoints()
                    .map_err(|e| e.to_string())?
                    .len();
                serde_json::to_string_pretty(&serde_json::json!({
                    "memories": mem_count,
                    "checkpoints": cp_count,
                }))
                .map_err(|e| e.to_string())
            }
            "registry_search" => {
                let query = str_arg(args.get("query")).ok_or("missing 'query'")?;
                let reg = self
                    .registry
                    .as_ref()
                    .ok_or("no registry configured on this server")?;
                let results = reg.search(query).map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
            }
            // -- v0.9.0: document RAG tools --
            "document_ingest" => {
                let source = str_arg(args.get("source")).ok_or("missing 'source'")?;
                let content = match str_arg(args.get("content")) {
                    Some(c) => c.to_string(),
                    None => cairn_document::read_source(source).map_err(|e| e.to_string())?,
                };
                if content.trim().is_empty() {
                    return Err("content must not be empty".into());
                }
                let title = str_arg(args.get("title")).unwrap_or(source).to_string();
                let chunks =
                    cairn_document::chunk_text(&content, cairn_document::DEFAULT_CHUNK_CHARS);
                self.store
                    .replace_document(source, &title, &chunks, scope.project_id.as_deref())
                    .map_err(|e| e.to_string())?;
                let doc = self
                    .store
                    .list_documents(None)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|d| d.source == source)
                    .ok_or_else(|| "document vanished after ingest".to_string())?;
                Ok(format!(
                    "ingested {} chunks from {}",
                    chunks.len(),
                    doc.source
                ))
            }
            "document_search" => {
                let query = str_arg(args.get("query")).ok_or("missing 'query'")?;
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
                let chunks = self
                    .store
                    .search_documents(query, limit, scope.project_id.as_deref())
                    .map_err(|e| e.to_string())?;
                if chunks.is_empty() {
                    return Ok("(no document chunks match)".into());
                }
                let mut out = String::new();
                for (i, c) in chunks.iter().enumerate() {
                    out.push_str(&format!(
                        "[{}] {} (from {})\n",
                        i + 1,
                        c.content.chars().take(300).collect::<String>(),
                        c.source
                    ));
                }
                Ok(out)
            }
            other => Err(format!("unknown tool: {other}")),
        }
    }
}

/// Tool definitions exposed by this MCP server. Public so the HTTP API can mirror them.
pub fn tool_defs() -> Value {
    json!([
        {
            "name": "read",
            "description": "Read a file through Cairn. Re-reading an unchanged file is nearly free; after edits you get only the diff. Returns a handle you can pass to `expand` for the full original - no context is ever lost.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to read." },
                    "mode": { "type": "string", "enum": ["auto", "full", "signatures", "map"], "description": "auto (cache-aware), full, signatures (AST outline - bodies elided), or map (outline + line numbers). For code files, signatures/map cost a fraction of the tokens; recover the full file with expand." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "expand",
            "description": "Recover the exact, byte-identical original for a handle (short or full) returned by `read`. Handles are content-addressed blobs in the store - they never expire and survive across sessions. Pass the `handle` field from a `read` result or the full `hash`.",
            "inputSchema": {
                "type": "object",
                "properties": { "hash": { "type": "string", "description": "The handle (short, 12 chars) or full content hash returned by `read`." } },
                "required": ["hash"]
            }
        },
        {
            "name": "remember",
            "description": "Save a durable memory so future sessions on any device recall it. Include title and reasoning when the memory isn't self-explanatory from content alone - they show up as the scannable headline and the \"why\" in the dashboard's Memory Browser. Defaults to project scope when a project is detected; promote good ones to global later.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "title": { "type": "string", "description": "Short scannable label, e.g. 'Use ripgrep over grep'. Falls back to the first line of content if omitted." },
                    "reasoning": { "type": "string", "description": "Why this matters or why this decision was made, kept separate from content - e.g. 'grep missed matches in binary-adjacent files last time'." },
                    "kind": { "type": "string", "enum": ["fact", "decision", "task", "preference", "gotcha", "note"] },
                    "tier": { "type": "string", "enum": ["working", "episodic", "semantic", "procedural"] },
                    "importance": { "type": "number", "minimum": 0, "maximum": 1 },
                    "scope_type": { "type": "string", "enum": ["global", "project", "session"], "description": "Visibility boundary. Defaults to 'project' when a project is detected, 'global' otherwise. Write at project scope first, then promote to global when the memory proves useful across projects." },
                    "scope_id": { "type": "string", "description": "Project or session id for scoped memories. Auto-filled from the current context when omitted (default)." },
                    "concepts": { "type": "array", "items": { "type": "string" }, "description": "Comma-separated key concepts" },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Comma-separated relevant file paths" }
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
            "description": "Assemble a lean, edge-ordered working set for a query under a token budget - the anti-context-rot context block. Reports what was included and dropped (dropped items remain recoverable via recall).",
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
            "name": "checkpoint",
            "description": "Snapshot the files Cairn has tracked (those read through Cairn) so you can roll back to this state. Optional `label`.",
            "inputSchema": { "type": "object", "properties": { "label": { "type": "string" } } }
        },
        {
            "name": "rollback",
            "description": "Restore every tracked file to a checkpoint's state from the blob store (undo agent damage). Requires the checkpoint `id`.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "checkpoints",
            "description": "List checkpoints (newest first) with their ids.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "anchor",
            "description": "Set or read the current task anchor - the goal Cairn re-injects at session start to keep you on track. Pass `goal` to set; omit to read the current goal.",
            "inputSchema": {
                "type": "object",
                "properties": { "goal": { "type": "string" } }
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
            "description": "Consolidate memory across the four tiers (working -> episodic -> semantic -> procedural). Run at session end to turn transient notes into durable knowledge.",
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
        },
        {
            "name": "verify_baseline",
            "description": "Compare the current on-disk file against the version Cairn recorded when you last read it. Detects silent corruption introduced after a read (PostToolUse check). Returns 'no baseline' if the file was never read through Cairn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path to check against its read baseline." }
                },
                "required": ["path"]
            }
        },
        // -- v0.5.0 Sprint 18: proactive recall --
        {
            "name": "proactive_recall",
            "description": "Run the proactive-recall hook for a prompt. Classifies whether the prompt is a question or task that would benefit from memory recall, and (if yes) returns up to 3 relevant memories to prepend to your context. Use this at the start of every turn when you suspect prior decisions may apply - saves a round-trip `cairn_recall` call. Honors the per-project opt-out: set `cairn prefer cairn.proactive_recall=false --applies-to <project_root>` to disable for a project. Returns `{matches: [...], reason: \"recalled\" | \"<skip reason>\"}` so callers can distinguish 'no relevant memories' from 'the classifier skipped this prompt'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "The pending user prompt or task description." },
                    "project_root": { "type": "string", "description": "Optional workspace root to check against opt-out preferences." }
                },
                "required": ["prompt"]
            }
        },
        {
            "name": "sanitize",
            "description": "Check text for secrets/PII before you share, log, or commit it. Redacts API keys, tokens, private keys, JWTs, secret=value assignments, emails, IPs, and home-directory paths, and classifies the result as shareable, needs_review, or private. Returns the redacted text plus the findings. Note: secret patterns require minimum lengths (e.g. 20+ chars after `sk-` for OpenAI keys) to avoid false positives on prose - short fragments like `sk-abc123` are deliberately not flagged.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to scan and redact." }
                },
                "required": ["text"]
            }
        },
        // -- v0.5.0 Sprint 10: memory CRUD + graph + search + metrics --
        {
            "name": "memory_edit",
            "description": "Edit an existing memory's mutable fields (content, title, reasoning, importance, concepts, files). Fields omitted from the input are left unchanged.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory id to edit." },
                    "content": { "type": "string", "description": "New content (optional)." },
                    "title": { "type": "string", "description": "New scannable label (optional)." },
                    "reasoning": { "type": "string", "description": "New rationale/why (optional)." },
                    "importance": { "type": "number", "description": "0.0--1.0, clamped." },
                    "concepts": { "type": "array", "items": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id"]
            }
        },
        {
            "name": "memory_delete",
            "description": "Delete a memory by id. Returns true if the memory existed and was removed.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "memory_pin",
            "description": "Pin or unpin a memory. Pinned memories always surface first in wakeup regardless of score/decay.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "pinned": { "type": "boolean", "default": true }
                },
                "required": ["id"]
            }
        },
        {
            "name": "memory_promote",
            "description": "Promote a memory to a specific tier (working / episodic / semantic / procedural).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "tier": { "type": "string", "enum": ["working", "episodic", "semantic", "procedural"] }
                },
                "required": ["id", "tier"]
            }
        },
        {
            "name": "memory_reinforce",
            "description": "Manually nudge a memory's confidence upward (agentmemory reinforcement curve) and bump access_count.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }
        },
        {
            "name": "memory_timeline",
            "description": "Newest-first memory timeline (by updated_at).",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1 } }
            }
        },
        {
            "name": "memory_crystallize",
            "description": "Promote all working-tier memories into one semantic crystal (agentmemory pattern). Each input gets a supersedes edge back to the crystal; the crystal gets derived_from edges to each input.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "memory_graph",
            "description": "Return the full memory provenance graph (nodes + edges) for the dashboard.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "search",
            "description": "Hybrid search (BM25 + HNSW + memory provenance graph, fused with RRF, reranked with MMR for diversity). With expand=true the engine first asks an LLM for 3-5 reformulations and merges results by max score (gated by CAIRN_LLM_CONSOLIDATION).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 },
                    "expand": { "type": "boolean", "description": "Enable LLM-driven query expansion (P3.3)." }
                },
                "required": ["query"]
            }
        },
        {
            "name": "metrics",
            "description": "Return local metrics (memory and checkpoint counts). Live savings ledger: GET /api/metrics.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        // -- registry tools (local embedded registry) --
        {
            "name": "registry_search",
            "description": "Search the local pack registry for published packs by name or description.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }
        },
        // -- v0.9.0: document RAG tools --
        {
            "name": "document_ingest",
            "description": "Ingest a file or URL into the document store for semantic search. Reads the source locally (or uses provided content), chunks it, and stores for later retrieval via document_search or assemble. Project-scoped when a project is detected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "File path or URL to read. Use with content omitted for local files." },
                    "content": { "type": "string", "description": "Optional pre-read content. When omitted, the tool reads source from the local filesystem." },
                    "title": { "type": "string", "description": "Defaults to source when omitted." }
                },
                "required": ["source"]
            }
        },
        {
            "name": "document_search",
            "description": "Search ingested document chunks by semantic similarity. Returns the most relevant chunks with scores. Project-scoped when a project is detected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 }
                },
                "required": ["query"]
            }
        }
    ])
}

/// Extract a string argument, if present.
#[cfg(feature = "engine")]
fn str_arg(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

// -- v0.5.0 Sprint 18: Proactive Recall --

/// Run the proactive recall hook for `prompt`. Returns the list of memories to
/// prepend to the agent's context, plus a short reason string explaining why
/// recall was or wasn't performed (intent didn't match, project opted out, or
/// no matches). The reason is surfaced in the MCP tool response so callers can
/// distinguish "no relevant memories" from "the classifier skipped this prompt".
#[cfg(feature = "engine")]
pub fn proactive_recall(
    server: &McpServer,
    prompt: &str,
    project_root: Option<&str>,
) -> (Vec<cairn_core::Memory>, &'static str) {
    let store = server.store.clone();
    let mem = server.mem.clone();

    // Build the opt-out preference set from any preference memories that
    // contain `cairn.proactive_recall=false`.
    let all_prefs: Vec<(String, Vec<String>)> = store
        .all_memories()
        .unwrap_or_default()
        .into_iter()
        .map(|m| (m.content, m.applies_to))
        .collect();
    let pref = cairn_proactive::ProactivePref::from_memories(&all_prefs);

    let hook = cairn_proactive::ProactiveHook::new(move |prompt: &str, k: usize| {
        let hits = mem.recall(prompt, k.max(1)).unwrap_or_default();
        hits.into_iter().map(|h| h.memory).collect()
    })
    .with_pref(pref)
    .with_max_inject(3)
    .with_threshold(0.4);

    match hook.on_turn(prompt, project_root) {
        cairn_proactive::HookOutcome::Recalled(mems) => (mems, "recalled"),
        cairn_proactive::HookOutcome::Skipped { reason } => {
            tracing::debug!(reason, "proactive recall skipped");
            (Vec::new(), reason)
        }
    }
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Start the right MCP backend for the current environment. If `CAIRN_SERVER` is set, the MCP
/// server forwards tool calls to that remote Cairn HTTP API; otherwise it opens the local store.
///
/// The local-store branch requires the `engine` feature (see `Cargo.toml`) - `cairn-client`
/// builds without it, since its own `Cmd::Mcp` handler already requires a server to be
/// configured before this function is ever reached (see `require_server()` in `main.rs`), so
/// that branch is provably unreachable there. `cairn-api`, which genuinely runs a local
/// `McpServer` per `/api/tools/call` request, builds with `engine` enabled.
pub fn serve_stdio(cfg: &cairn_core::Config) -> std::io::Result<()> {
    if let Some(server) = cfg.default_server.as_deref() {
        let token = std::env::var("CAIRN_TOKEN").ok();
        let mut proxy = RemoteProxy::new(server, token);
        // Use the configured workspace root (or cwd) as the host project dir for path rewriting.
        if let Some(root) = &cfg.workspace_root {
            proxy.host_workspace = root.clone();
            proxy.reader.lock().unwrap().workspace = root.clone();
        }
        proxy.serve_stdio()
    } else {
        #[cfg(feature = "engine")]
        {
            McpServer::new(cfg)
                .map_err(|e| std::io::Error::other(e.to_string()))?
                .serve_stdio()
        }
        #[cfg(not(feature = "engine"))]
        {
            Err(std::io::Error::other(
                "no server configured, and this build has no local engine support -- \
                  set CAIRN_SERVER or run `cairn setup --all --server <url> --token <jwt>`",
            ))
        }
    }
}

/// A stateful local file reader with mtime cache + diff support (Bug #2 fix).
///
/// Replaces the old stateless `read_file_local` that always returned `Full`. This reader:
/// - Tracks file mtime — if unchanged since last read, returns a tiny `Cached` view (~13 tokens).
/// - If the file changed, returns only the `Diff` (added/removed lines).
/// - Falls back to `Full` for first reads or when the diff would be ≥60% of the full file.
/// - Caches content by content hash so `expand` recovers the original without a blob store.
///
/// Does NOT support `signatures`/`map` modes (those need tree-sitter, only available with
/// the `engine` feature). Structural mode requests fall back to `Full` on this path.
struct LocalReader {
    workspace: std::path::PathBuf,
    /// path → (mtime_ns, content_hash, content, lines)
    cache: HashMap<String, LocalCacheEntry>,
}

struct LocalCacheEntry {
    mtime_ns: u128,
    hash: String,
    content: String,
    lines: usize,
}

impl LocalReader {
    fn new(workspace: std::path::PathBuf) -> Self {
        Self {
            workspace,
            cache: HashMap::new(),
        }
    }

    fn read(
        &mut self,
        path: &str,
        mode: &str,
        blob_cache: &Mutex<HashMap<String, String>>,
    ) -> std::result::Result<String, String> {
        let p = std::path::Path::new(path);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(p)
        };
        let key = resolved.to_string_lossy().to_string();

        let meta =
            std::fs::metadata(&resolved).map_err(|e| format!("{}: {e}", resolved.display()))?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);

        // Re-read killer: if the file hasn't changed (mtime match), return a tiny Cached view.
        if mode == "auto" || mode == "full" {
            if let Some(entry) = self.cache.get(&key) {
                if entry.mtime_ns == mtime_ns && mode == "auto" {
                    let note = format!(
                        "unchanged since last read; {} lines; `expand {}` for the full file",
                        entry.lines, entry.hash
                    );
                    let result = json!({
                        "path": key,
                        "hash": entry.hash,
                        "handle": &entry.hash[..12.min(entry.hash.len())],
                        "status": "cached",
                        "lines": entry.lines,
                        "bytes": entry.content.len(),
                        "view": "",
                        "note": note,
                        "est_tokens": note.len() / 4,
                    });
                    return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
                }
            }
        }

        // Read fresh content.
        let content = std::fs::read_to_string(&resolved)
            .map_err(|e| format!("{}: {e}", resolved.display()))?;
        let bytes = content.len();
        let lines = content.lines().count();
        let hash = ContentHash::of(content.as_bytes());
        let handle = hash.short().to_string();

        // Cache by hash for expand().
        {
            let mut bc = blob_cache.lock().unwrap();
            bc.insert(hash.0.clone(), content.clone());
            bc.insert(handle.clone(), content.clone());
        }

        let original_tokens = bytes / 4;

        // Check for diff (Auto mode, file changed).
        let prev = self.cache.get(&key).map(|e| e.content.clone());
        let (status, view, note, est_tokens) = match (&prev, mode) {
            (Some(prev_content), "auto") if *prev_content != content => {
                let diff = local_diff_only(prev_content, &content);
                let diff_tokens = diff.len() / 4;
                if diff_tokens >= (original_tokens * 3) / 5 {
                    // Diff ≥ 60% of full → ship full instead.
                    (
                        "full",
                        content.clone(),
                        format!("full file; {lines} lines; handle {handle}"),
                        original_tokens,
                    )
                } else {
                    (
                        "diff",
                        diff,
                        format!(
                            "changed since last read; showing diff only; `expand {}` for the full file",
                            handle
                        ),
                        diff_tokens,
                    )
                }
            }
            _ => (
                "full",
                content.clone(),
                format!("full file; {lines} lines; handle {handle}"),
                original_tokens,
            ),
        };

        // Update cache.
        self.cache.insert(
            key.clone(),
            LocalCacheEntry {
                mtime_ns,
                hash: hash.0.clone(),
                content,
                lines,
            },
        );

        let result = json!({
            "path": key,
            "hash": hash.0,
            "handle": handle,
            "status": status,
            "lines": lines,
            "bytes": bytes,
            "view": view,
            "note": note,
            "est_tokens": est_tokens,
        });
        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }
}

/// A compact diff: only added/removed lines (prefixed `+`/`-`), equal lines omitted.
fn local_diff_only(old: &str, new: &str) -> String {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut out = String::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Delete => {
                out.push('-');
                out.push_str(change.value());
            }
            similar::ChangeTag::Insert => {
                out.push('+');
                out.push_str(change.value());
            }
            similar::ChangeTag::Equal => {}
        }
    }
    out
}

/// An MCP stdio server that forwards tool calls to a remote Cairn HTTP API.
///
/// File-local tools (`read`, `verify`, `checkpoint`, `rollback`) get their `path` argument
/// rewritten: if the path is absolute and inside the proxy's current working directory, it is
/// made relative to that directory before forwarding. The remote server has its
/// `CAIRN_WORKSPACE_ROOT` pointed at the mounted project, so relative paths resolve correctly
/// inside the container.
pub struct RemoteProxy {
    server: String,
    token: Option<String>,
    /// The host directory where `cairn mcp` was launched (the project root from the agent).
    host_workspace: std::path::PathBuf,
    /// Content cache for files read locally: sha256 hex -> file content.
    /// Enables `expand` to recover the original without a blob store.
    file_cache: Mutex<HashMap<String, String>>,
    /// Stateful local reader with mtime cache + diff support (Bug #2 fix).
    reader: Mutex<LocalReader>,
}

impl RemoteProxy {
    pub fn new(server: &str, token: Option<String>) -> Self {
        let workspace = std::env::current_dir().unwrap_or_default();
        Self {
            server: server.trim_end_matches('/').to_string(),
            token,
            host_workspace: workspace.clone(),
            file_cache: Mutex::new(HashMap::new()),
            reader: Mutex::new(LocalReader::new(workspace)),
        }
    }

    pub fn serve_stdio(&self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let mut locked = stdin.lock();
        let mut line = String::new();
        loop {
            line.clear();
            if locked.read_line(&mut line)? == 0 {
                break;
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
                        "capabilities": {
                            "tools": {},
                            "workspaceRoot": self.host_workspace.display().to_string()
                        },
                        "serverInfo": { "name": "cairn", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": guidance::GUIDANCE_COMPACT
                    }),
                ))
            }
            "notifications/initialized" | "initialized" => None,
            "ping" => Some(ok(id, json!({}))),
            "tools/list" => Some(self.list_tools(id)),
            "tools/call" => Some(self.call_tool(id, req.get("params"))),
            other => id.map(|id| err(Some(id), -32601, &format!("method not found: {other}"))),
        }
    }

    fn list_tools(&self, id: Option<Value>) -> Value {
        match self.get("/api/tools/list") {
            Ok(v) => ok(id, v),
            Err(e) => err(id, -32603, &format!("failed to list tools: {e}")),
        }
    }

    fn call_tool(&self, id: Option<Value>, params: Option<&Value>) -> Value {
        let Some(params) = params else {
            return err(id, -32602, "missing params");
        };
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args = params
            .get("arguments")
            .and_then(|a| a.as_object())
            .cloned()
            .unwrap_or_default();
        // Handle file-local tools on the host so reads always work regardless of whether the
        // project is mounted on the remote server.
        match name {
            "read" => {
                let path = args.get("path").and_then(Value::as_str).unwrap_or("");
                let mode = args.get("mode").and_then(Value::as_str).unwrap_or("auto");
                let mut reader = self.reader.lock().unwrap();
                match reader.read(path, mode, &self.file_cache) {
                    Ok(text) => ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
                    Err(msg) => ok(
                        id,
                        json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
                    ),
                }
            }
            "expand" => {
                let hash = args.get("hash").and_then(Value::as_str).unwrap_or("");
                // Try local cache first, fall back to remote proxy.
                match self.expand_local(hash) {
                    Ok(text) => ok(id, json!({ "content": [{ "type": "text", "text": text }] })),
                    Err(_) => {
                        // Forward to remote server as fallback.
                        match self.post("/api/tools/call", params) {
                            Ok(v) => ok(id, v),
                            Err(e) => {
                                let msg = format!("expand failed: {e}");
                                ok(
                                    id,
                                    json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
                                )
                            }
                        }
                    }
                }
            }
            "verify" | "checkpoint" | "rollback" => {
                // Rewrite absolute host paths to workspace-relative before forwarding.
                let rewritten = self.rewrite_file_path(params);
                match self.post("/api/tools/call", &rewritten) {
                    Ok(v) => ok(id, v),
                    Err(e) => {
                        let msg = format!("tool call failed: {e}");
                        ok(
                            id,
                            json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
                        )
                    }
                }
            }
            _ => match self.post("/api/tools/call", params) {
                Ok(v) => ok(id, v),
                Err(e) => {
                    let msg = format!("tool call failed: {e}");
                    ok(
                        id,
                        json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
                    )
                }
            },
        }
    }

    /// If `params.arguments.path` is an absolute path inside `host_workspace`, replace it with
    /// the workspace-relative form. Returns a cloned params Value with the rewritten path.
    fn rewrite_file_path(&self, params: &Value) -> Value {
        let mut out = params.clone();
        if let Some(args) = out.get_mut("arguments").and_then(|v| v.as_object_mut()) {
            if let Some(path_val) = args.get("path").and_then(Value::as_str) {
                let p = std::path::Path::new(path_val);
                if p.is_absolute() {
                    if let Ok(rel) = p.strip_prefix(&self.host_workspace) {
                        args.insert(
                            "path".into(),
                            Value::String(rel.to_string_lossy().into_owned()),
                        );
                    }
                }
            }
        }
        out
    }

    /// Read a file from the host filesystem with mtime cache + diff support.
    /// Deprecated — use `LocalReader::read` instead. Kept for backward compat.
    #[allow(dead_code)]
    fn read_file_local(&self, path: &str, _mode: &str) -> std::result::Result<String, String> {
        self.reader
            .lock()
            .unwrap()
            .read(path, "auto", &self.file_cache)
    }

    /// Look up a file previously read by `read_file_local` and return its content.
    fn expand_local(&self, hash: &str) -> std::result::Result<String, String> {
        self.file_cache
            .lock()
            .unwrap()
            .get(hash)
            .cloned()
            .ok_or_else(|| format!("unknown handle: {hash}"))
    }

    fn get(&self, path: &str) -> std::result::Result<Value, String> {
        let url = format!("{}{path}", self.server);
        let mut req = ureq::get(&url);
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let resp = req.call().map_err(|e| e.to_string())?;
        resp.into_json().map_err(|e| e.to_string())
    }

    fn post(&self, path: &str, body: &Value) -> std::result::Result<Value, String> {
        let url = format!("{}{path}", self.server);
        let mut req = ureq::post(&url);
        if let Some(t) = &self.token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        let resp = req.send_json(body).map_err(|e| e.to_string())?;
        resp.into_json().map_err(|e| e.to_string())
    }
}

// Requires `--features engine`: every test here except one constructs a real `McpServer`.
// `RemoteProxy` (the path `cairn-client` actually takes) has no engine dependency at all, but
// splitting it out of this shared module isn't worth the churn - it's exercised indirectly by
// `cairn-client`'s own test suite regardless.
#[cfg(all(test, feature = "engine"))]
mod tests {
    use super::*;

    /// `None` when `CAIRN_DB_URL` is unset or the database is unreachable (tests skip gracefully).
    fn server() -> Option<McpServer> {
        let cfg = cairn_store::Store::test_config()?;
        McpServer::new(&cfg).ok()
    }

    #[test]
    fn initialize_echoes_version_and_lists_tools() {
        let Some(s) = server() else { return };
        let init = s
            .handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}))
            .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "cairn");
        assert_eq!(
            init["result"]["instructions"].as_str(),
            Some(guidance::GUIDANCE_COMPACT),
            "initialize must surface the tool playbook via `instructions` (B6) so every \
             MCP-speaking agent gets it automatically at connect time"
        );

        let list = s
            .handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
            .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "read"));
        assert!(tools.iter().any(|t| t["name"] == "remember"));
    }

    /// `RemoteProxy` is the path `cairn mcp` actually takes against a configured server (the
    /// thin-client v0.8.0 design has no in-process engines) - its `initialize` branch builds
    /// the response locally with no HTTP call, so this needs no live server to verify it also
    /// carries `instructions`.
    #[test]
    fn remote_proxy_initialize_also_carries_instructions() {
        let proxy = RemoteProxy::new("http://unused.invalid", None);
        let init = proxy
            .handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}))
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "cairn");
        assert_eq!(
            init["result"]["instructions"].as_str(),
            Some(guidance::GUIDANCE_COMPACT)
        );
    }

    #[test]
    fn remember_then_recall_via_tools_call() {
        let Some(s) = server() else { return };
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
        let Some(s) = server() else { return };
        assert!(s
            .handle(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .is_none());
    }

    #[test]
    fn sanitize_tool_redacts_and_classifies() {
        let Some(s) = server() else { return };
        // Assembled at runtime so the repo stores no verbatim credential (push protection).
        let token = format!("ghp_{}", "0123456789abcdefghijklmnopqrstuvwxyz");
        let resp = s
            .handle(
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"sanitize","arguments":{"text": format!("deploy token={token}")}}}),
            )
            .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["sensitivity"], "private");
        assert!(v["text"]
            .as_str()
            .unwrap()
            .contains("[redacted:github_token]"));
        assert!(!text.contains(&token), "raw secret leaked in tool output");
    }

    #[test]
    fn tools_list_exposes_v050_tool_set() {
        let Some(s) = server() else { return };
        let list = s
            .handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        // v0.5.0 advertises 29 tools (Sprint 10 landed the 40+ claim in earlier sprints,
        // but a number of those tools were consolidated into resource URIs and graph
        // helpers; the v0.5.0 MCP surface is the 29 below). Update both this number AND
        // the representative subset assertion if you add or remove a tool.
        assert!(
            tools.len() >= 29,
            "expected >=29 tools in v0.5.0, got {}",
            tools.len()
        );
        for name in [
            "memory_edit",
            "memory_delete",
            "memory_pin",
            "memory_promote",
            "memory_reinforce",
            "memory_timeline",
            "memory_crystallize",
            "memory_graph",
            "graph",
            "search",
            "metrics",
            "proactive_recall",
        ] {
            assert!(
                tools.iter().any(|t| t["name"] == name),
                "missing tool {name} in tools/list"
            );
        }
    }

    #[test]
    fn memory_edit_pin_delete_pin_round_trip() {
        let Some(s) = server() else { return };
        let create = s
            .handle(
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"remember","arguments":{"content":"sprint 10 round trip"}}}),
            )
            .unwrap();
        let create_text = create["result"]["content"][0]["text"].as_str().unwrap();
        // "remembered <id> ..." - extract the id.
        let id = create_text
            .split_whitespace()
            .nth(1)
            .expect("id present in remember output");
        let id = id.trim_end_matches(&['(', ')', '.', ','][..]);

        // memory_edit
        let edited = s
            .handle(
                &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"memory_edit","arguments":{"id": id, "content":"sprint 10 EDITED"}}}),
            )
            .unwrap();
        assert!(edited["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("edited"));

        // memory_pin
        let pinned = s
            .handle(
                &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"memory_pin","arguments":{"id": id, "pinned": true}}}),
            )
            .unwrap();
        assert!(pinned["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("pinned"));

        // memory_reinforce
        let reinforced = s
            .handle(
                &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"memory_reinforce","arguments":{"id": id}}}),
            )
            .unwrap();
        assert!(reinforced["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("reinforced"));

        // memory_graph returns nodes + edges (serialized JSON).
        let graph = s
            .handle(
                &json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{
            "name":"memory_graph","arguments":{}}}),
            )
            .unwrap();
        let body = graph["result"]["content"][0]["text"].as_str().unwrap();
        let v: Value = serde_json::from_str(body).unwrap();
        assert!(v["nodes"].as_array().is_some());
        assert!(v["edges"].as_array().is_some());

        // memory_delete
        let deleted = s
            .handle(
                &json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{
            "name":"memory_delete","arguments":{"id": id}}}),
            )
            .unwrap();
        assert!(deleted["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("deleted"));

        // memory_delete again -> tool error
        let err = s
            .handle(
                &json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{
            "name":"memory_delete","arguments":{"id": id}}}),
            )
            .unwrap();
        assert!(err["result"]["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn proactive_recall_tool_returns_memories_for_recall_cue() {
        let Some(s) = server() else { return };
        // Seed a memory with a recognizable token.
        s.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"remember","arguments":{"content":"the team decided tabs over spaces last time"}}}))
            .unwrap();

        // A recall cue prompt - the hook should fire and return at least one memory.
        let resp = s
            .handle(
                &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"proactive_recall","arguments":{"prompt":"What did we decide last time about formatting?"}}}),
            )
            .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("tabs"),
            "expected the seeded decision to be recalled, got: {text}"
        );
    }

    #[test]
    fn proactive_recall_skips_plain_imperative_prompt() {
        let Some(s) = server() else { return };
        let resp = s
            .handle(
                &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
                "name":"proactive_recall","arguments":{"prompt":"Add a print statement to foo.rs"}}}),
            )
            .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        // Plain imperative -> hook returns no memories -> JSON `[]`.
        assert_eq!(text.trim(), "[]", "expected empty recall, got: {text}");
    }

    #[test]
    fn proactive_recall_respects_per_project_opt_out() {
        let Some(s) = server() else { return };
        // Remember a decision so recall would otherwise fire.
        s.handle(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"remember","arguments":{"content":"unique-token-foo-bar-baz last time"}}}),
        )
        .unwrap();
        // Opt out the project.
        s.handle(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"remember","arguments":{
                "content":"cairn.proactive_recall=false for this loud project",
                "applies_to": ["/work/loud"]
            }}}),
        )
        .unwrap();

        let resp = s
            .handle(
                &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":"proactive_recall","arguments":{
                    "prompt":"What did we decide last time about unique-token-foo-bar-baz?",
                    "project_root": "/work/loud"
                }}}),
            )
            .unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            text.trim(),
            "[]",
            "expected opt-out to suppress recall, got: {text}"
        );
    }
}

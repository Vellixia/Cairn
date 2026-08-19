//! The MCP server: exactly six tools, no more (FR-040).
//!
//! Speaks JSON-RPC 2.0 over stdio — `initialize`, `tools/list`, `tools/call` —
//! and forwards every call to the local daemon. Each tool takes an `action`
//! discriminator rather than exploding into one tool per database operation,
//! which is what keeps the agent's tool list short enough to be useful.

use crate::client;
use crate::render;
use cairn_core::wire::{ContextPayload, MemoryQuery, Request, WireError};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// Versions this server understands, newest first.
///
/// A client asking for something else gets our newest — echoing an unknown
/// version back would claim support Cairn does not have.
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// The six tools. Anything beyond this list is a scope violation (FR-040).
pub const TOOL_NAMES: &[&str] = &[
    "cairn_context",
    "cairn_search",
    "cairn_remember",
    "cairn_session",
    "cairn_task",
    "cairn_handoff",
];

pub async fn serve() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        // Malformed input gets the JSON-RPC parse error, not silence.
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(m) => m,
            Err(e) => {
                let body = serde_json::to_string(&error_response(
                    Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                ))?;
                stdout.write_all(format!("{body}\n").as_bytes()).await?;
                stdout.flush().await?;
                continue;
            }
        };
        let Some(method) = message.get("method").and_then(|m| m.as_str()) else {
            continue;
        };
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        // Notifications carry no id and expect no reply.
        let Some(id) = id else { continue };

        let response = match method {
            "initialize" => success(id, initialize(&params)),
            "tools/list" => success(id, json!({ "tools": tool_definitions() })),
            "tools/call" => success(id, call(&params).await),
            "ping" => success(id, json!({})),
            other => error_response(id, -32601, &format!("unknown method: {other}")),
        };

        let mut body = serde_json::to_string(&response)?;
        body.push('\n');
        stdout.write_all(body.as_bytes()).await?;
        stdout.flush().await?;
    }
    Ok(())
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(|v| v.as_str());
    let negotiated = match requested {
        Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v,
        _ => PROTOCOL_VERSION,
    };
    // The compact universal rendering of the usage contract, delivered
    // through the protocol's own server-instructions mechanism, so a client
    // with no native adapter still receives correct behavior (FR-129).
    //
    // Delivery is best-effort: the specification calls `instructions` a hint
    // clients *may* add to the system prompt, so Cairn never reports the
    // contract as *delivered* through this path.
    json!({
        "protocolVersion": negotiated,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "cairn", "version": env!("CARGO_PKG_VERSION") },
        "instructions": cairn_integrate::render::Contract::canonical().mcp_instructions(),
    })
}

fn cwd_property() -> Value {
    json!({ "type": "string", "description": "Directory to resolve the repository from" })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "cairn_context",
            "description": "Build the bounded briefing for the current repository: project, \
                            branch, commit, working tree, task goal, previous handoff, and \
                            relevant scoped memory — plus the minimum safe continuity, drift \
                            and conflict warnings, and whether your checkpoint diverged.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cwd": cwd_property(),
                    // `post_compaction` is how an agent whose adapter has no
                    // post-compaction event restores continuity itself. An
                    // unknown value still falls back to `refresh` (D57, FR-426).
                    "reason": { "type": "string", "enum": ["session_start", "continuation", "refresh", "post_compaction"] },
                    "depth": { "type": "string", "enum": ["minimum", "standard"], "description": "`minimum` is Level 0 only. Level 2 is never automatic." },
                    "include_patterns": { "type": "boolean", "description": "Signal-matched patterns from other projects, always labelled unverified here" },
                    "explain": { "type": "boolean", "description": "Return the selection diagnostics. Costs no budget when false." },
                    "token_budget": { "type": "integer", "description": "Cairn-estimated tokens" },
                    "agent_session_key": { "type": "string", "description": "Your own session identifier. Required when more than one session is open in this worktree." },
                    "session_id": { "type": "string", "description": "Cairn session id, as an alternative to agent_session_key" }
                },
                "required": ["cwd"]
            }
        }),
        json!({
            "name": "cairn_search",
            "description": "Search durable memory. Results are ranked scope-first — current \
                            task, then branch, then project — with lexical relevance and \
                            recency breaking ties within a scope. Filter by verification state \
                            or subject; ask for reusable patterns explicitly.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cwd": cwd_property(),
                    "query": { "type": "string" },
                    "scope": { "type": "string", "enum": ["project", "branch", "task", "session"] },
                    "scope_key": { "type": "string" },
                    "type": { "type": "string", "enum": ["fact", "decision", "convention", "failure", "procedure"] },
                    "state": { "type": "string", "enum": ["active", "stale", "superseded"] },
                    "limit": { "type": "integer" },
                    // A `drifted` memory is lifecycle-`active` and is returned
                    // by default, with its verification state visible (FR-373).
                    "verification": { "type": "string", "enum": ["unverified", "verified", "needs_recheck", "drifted", "conflicted"] },
                    "authority": { "type": "string", "enum": ["cairn", "attested", "remote_cairn", "remote_attested"], "description": "What established the verification" },
                    "corroborated": { "type": "boolean" },
                    "conflicted": { "type": "boolean" },
                    "topic_key": { "type": "string", "description": "Exact, or a prefix when it ends in a dot" },
                    "as_of": { "type": "string", "description": "What was effective at this instant, RFC 3339" },
                    "pinned": { "type": "boolean" },
                    "include_patterns": { "type": "boolean", "description": "Patterns in a separate array, never mixed into results" },
                    "agent_session_key": { "type": "string", "description": "Your own session identifier, so scope precedence uses your task" },
                    "session_id": { "type": "string", "description": "Cairn session id, as an alternative to agent_session_key" }
                },
                "required": ["cwd"]
            }
        }),
        json!({
            "name": "cairn_remember",
            "description": "Record durable knowledge, replace it, or forget it. Supporting \
                            observations are optional and are never invented. Give durable \
                            project facts a `topic_key` and a `value_key` specific enough to \
                            state the whole claim. Attach evidence rather than asserting \
                            importance. If Cairn reports a corroborating member and it is the \
                            same claim, reinforce it. Record a conflict rather than overwriting.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cwd": cwd_property(),
                    // One discriminator, and no action takes a sub-operation (D70).
                    "action": { "type": "string", "enum": [
                        "create", "supersede", "forget",
                        "reinforce", "attach_evidence", "verify", "pin",
                        "reconcile", "promote", "record_outcome"
                    ] },
                    "type": { "type": "string", "enum": ["fact", "decision", "convention", "failure", "procedure"] },
                    "scope": { "type": "string", "enum": ["project", "branch", "task", "session"] },
                    "scope_key": { "type": "string" },
                    "content": { "type": "string" },
                    "evidence_observation_ids": { "type": "array", "items": { "type": "string" } },
                    "local_only": { "type": "boolean" },
                    "memory_id": { "type": "string" },
                    // create / supersede
                    "topic_key": { "type": "string", "description": "The subject this states something about. A key that will not normalize is reported and the memory is stored free-form." },
                    "value_key": { "type": "string", "description": "The comparable value it asserts. Needs a topic_key." },
                    "importance": { "type": "string", "enum": ["low", "normal", "high"], "description": "Ranks within a bucket, and nothing more" },
                    // attach_evidence
                    "kind": { "type": "string", "enum": ["observation", "file", "git_ref", "configuration", "test_outcome", "command_outcome", "runtime_state", "schema_version"] },
                    "subject": { "type": "string" },
                    "observed_value": { "type": "string" },
                    "source_locator": { "type": "string", "description": "Repository-relative or a Git ref. Never absolute." },
                    "observation_id": { "type": "string" },
                    "role": { "type": "string", "enum": ["supports", "contradicts"] },
                    "collector": { "type": "string", "enum": ["cairn", "agent"], "description": "`agent` when you are attesting rather than Cairn checking" },
                    // pin
                    "pinned": { "type": "boolean" },
                    "reason": { "type": "string" },
                    // reconcile
                    "from_memory_id": { "type": "string" },
                    "to_memory_id": { "type": "string" },
                    "relation": { "type": "string", "enum": ["narrows", "not_applicable_to", "supersedes"] },
                    "basis": { "type": "string", "enum": ["explicit_agent", "evidence"] },
                    "basis_evidence_id": { "type": "string" },
                    "rationale": { "type": "string" },
                    // promote / record_outcome
                    "signals": { "type": "array", "items": { "type": "string" } },
                    "applicability": { "type": "array", "items": { "type": "string" } },
                    "root_cause": { "type": "string" },
                    "approach": { "type": "string" },
                    "constraints": { "type": "array", "items": { "type": "string" } },
                    "dry_run": { "type": "boolean" },
                    "pattern_id": { "type": "string" },
                    "outcome": { "type": "string", "enum": ["resolved", "not_applicable", "failed"] },
                    "alternative_cause": { "type": "string" },
                    "evidence_id": { "type": "string" },
                    "agent_session_key": { "type": "string", "description": "Your own session identifier. Required when more than one session is open in this worktree." },
                    "session_id": { "type": "string", "description": "Cairn session id, as an alternative to agent_session_key" }
                },
                "required": ["cwd", "action"]
            }
        }),
        json!({
            "name": "cairn_session",
            "description": "Inspect or steer the current session. Starting is idempotent per \
                            agent session, so two agents in one checkout get two sessions — or \
                            write a continuity checkpoint before you compact.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cwd": cwd_property(),
                    "action": { "type": "string", "enum": ["current", "start", "bind_task", "end", "checkpoint"] },
                    "agent": { "type": "string" },
                    "agent_session_key": { "type": "string" },
                    "session_id": { "type": "string" },
                    "task_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["completed", "interrupted"] },
                    // checkpoint
                    "next_action": { "type": "string", "description": "What you were about to do" },
                    "relevant_paths": { "type": "array", "items": { "type": "string" }, "description": "Repository-relative paths this work depends on" }
                },
                "required": ["cwd", "action"]
            }
        }),
        json!({
            "name": "cairn_task",
            "description": "List, read, create or update tasks — title, goal, acceptance \
                            criteria and status. Update one criterion at a time and pass the \
                            `expected_revision` you read.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cwd": cwd_property(),
                    "action": { "type": "string", "enum": [
                        "list", "get", "create", "update",
                        "add_criterion", "update_criterion", "blocker", "readiness"
                    ] },
                    "task_id": { "type": "string" },
                    "title": { "type": "string" },
                    "goal": { "type": "string" },
                    "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
                    "status": { "type": "string", "enum": ["todo", "in_progress", "done", "blocked"] },
                    // add_criterion / update_criterion
                    "text": { "type": "string" },
                    "criterion_id": { "type": "string" },
                    "state": { "type": "string", "enum": ["pending", "satisfied", "blocked", "waived"] },
                    "verification": { "type": "string", "enum": ["unverified", "verified", "failed"] },
                    "evidence_observation_id": { "type": "string" },
                    // This store's local concurrency token, read from `get`. It
                    // is not a version anyone else shares (D80).
                    "expected_revision": { "type": "integer" },
                    // blocker
                    "description": { "type": "string" },
                    "blocker_id": { "type": "string" },
                    "clear": { "type": "boolean" }
                },
                "required": ["cwd", "action"]
            }
        }),
        json!({
            "name": "cairn_handoff",
            "description": "Read the latest handoff, generate one at a boundary, or attach a \
                            bounded note beside the derived record — optionally with its \
                            continuity checkpoint.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cwd": cwd_property(),
                    "action": { "type": "string", "enum": ["latest", "generate", "annotate"] },
                    "session_id": { "type": "string" },
                    "agent_session_key": { "type": "string" },
                    // `stop` is deliberately absent: a turn checkpoint is not a
                    // handoff boundary (Feature 001 D16).
                    "trigger": { "type": "string", "enum": ["pre_compact", "session_end"] },
                    "note": { "type": "string" },
                    "include_checkpoint": { "type": "boolean", "description": "Add the anchored checkpoint and its staleness assessment" }
                },
                "required": ["cwd", "action"]
            }
        }),
    ]
}

async fn call(params: &Value) -> Value {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match dispatch(name, &args).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("{}: {}", e.code, e.message) }],
            "isError": true
        }),
    }
}

async fn dispatch(name: &str, args: &Value) -> Result<String, WireError> {
    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(crate::cwd);
    let key = str_arg(args, "agent_session_key");

    match name {
        "cairn_context" => {
            let value = client::send(&Request::Context {
                cwd,
                agent_session_key: key,
                session_id: uuid_arg(args, "session_id").ok(),
                reason: args
                    .get("reason")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
                token_budget: args
                    .get("token_budget")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
                // The six tools are fixed; diagnostics stay a CLI affordance.
                explain: false,
            })
            .await?;
            // The agent gets the rendered briefing plus the raw envelope, so it
            // can read either.
            let payload: ContextPayload = serde_json::from_value(value.clone())
                .map_err(|e| WireError::invalid(e.to_string()))?;
            Ok(format!(
                "{}\n\n```json\n{}\n```",
                render::briefing(&payload),
                pretty(&value)
            ))
        }

        "cairn_search" => {
            let query = MemoryQuery {
                query: str_arg(args, "query"),
                scope: enum_arg(args, "scope"),
                scope_key: str_arg(args, "scope_key"),
                kind: enum_arg(args, "type"),
                state: enum_arg(args, "state"),
                limit: args.get("limit").and_then(|v| v.as_i64()),
                topic_key: str_arg(args, "topic_key"),
                as_of: str_arg(args, "as_of").and_then(|t| {
                    chrono::DateTime::parse_from_rfc3339(&t)
                        .ok()
                        .map(|d| d.with_timezone(&chrono::Utc))
                }),
                conflicted: args
                    .get("conflicted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                corroborated: args
                    .get("corroborated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                include_patterns: args
                    .get("include_patterns")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                verification: enum_arg(args, "verification"),
                authority: enum_arg(args, "authority"),
            };
            let value = client::send(&Request::MemorySearch {
                cwd,
                agent_session_key: key,
                session_id: uuid_arg(args, "session_id").ok(),
                query,
            })
            .await?;
            Ok(pretty(&value))
        }

        "cairn_remember" => {
            let action = required_action(args)?;
            let value = match action.as_str() {
                "create" | "supersede" => {
                    let kind = enum_arg(args, "type")
                        .ok_or_else(|| WireError::invalid("type is required"))?;
                    let content = str_arg(args, "content")
                        .ok_or_else(|| WireError::invalid("content is required"))?;
                    let evidence = uuid_list(args, "evidence_observation_ids");
                    let local_only = args
                        .get("local_only")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if action == "create" {
                        client::send(&Request::MemoryCreate {
                            cwd,
                            agent_session_key: key,
                            session_id: uuid_opt(args, "session_id"),
                            kind,
                            scope: enum_arg(args, "scope"),
                            scope_key: str_arg(args, "scope_key"),
                            content,
                            evidence_observation_ids: evidence,
                            local_only,
                            topic_key: str_arg(args, "topic_key"),
                            value_key: str_arg(args, "value_key"),
                            importance: enum_arg(args, "importance"),
                        })
                        .await?
                    } else {
                        client::send(&Request::MemorySupersede {
                            cwd,
                            agent_session_key: key,
                            session_id: uuid_opt(args, "session_id"),
                            memory_id: uuid_arg(args, "memory_id")?,
                            kind,
                            scope: enum_arg(args, "scope"),
                            scope_key: str_arg(args, "scope_key"),
                            content,
                            evidence_observation_ids: evidence,
                            local_only,
                            topic_key: str_arg(args, "topic_key"),
                            value_key: str_arg(args, "value_key"),
                            importance: enum_arg(args, "importance"),
                        })
                        .await?
                    }
                }
                "forget" => {
                    client::send(&Request::MemoryForget {
                        cwd,
                        memory_id: uuid_arg(args, "memory_id")?,
                    })
                    .await?
                }
                other => return Err(WireError::invalid(format!("unknown action: {other}"))),
            };
            Ok(pretty(&value))
        }

        "cairn_session" => {
            let action = required_action(args)?;
            let value = match action.as_str() {
                "current" => {
                    client::send(&Request::SessionShow {
                        cwd,
                        session_id: None,
                        agent_session_key: key,
                    })
                    .await?
                }
                "start" => {
                    client::send(&Request::SessionStart {
                        cwd,
                        agent: str_arg(args, "agent").unwrap_or_else(|| "mcp-client".into()),
                        agent_session_key: key,
                        task_id: uuid_arg(args, "task_id").ok(),
                    })
                    .await?
                }
                "bind_task" => {
                    client::send(&Request::SessionBindTask {
                        cwd,
                        session_id: None,
                        agent_session_key: key,
                        task_id: uuid_arg(args, "task_id")?,
                    })
                    .await?
                }
                "end" => {
                    client::send(&Request::SessionEnd {
                        cwd,
                        session_id: None,
                        agent_session_key: key,
                        status: enum_arg(args, "status")
                            .unwrap_or(cairn_core::domain::SessionStatus::Completed),
                        reason: str_arg(args, "reason"),
                        // An agent tool call has no vendor handler deadline
                        // over it, so it keeps Feature 001's behavior and
                        // waits for the durable handoff (D22).
                        wait_for_handoff: true,
                    })
                    .await?
                }
                other => return Err(WireError::invalid(format!("unknown action: {other}"))),
            };
            Ok(pretty(&value))
        }

        "cairn_task" => {
            let action = required_action(args)?;
            let value = match action.as_str() {
                "list" => {
                    client::send(&Request::TaskList {
                        cwd,
                        status: enum_arg(args, "status"),
                    })
                    .await?
                }
                "get" => {
                    client::send(&Request::TaskGet {
                        cwd,
                        task_id: uuid_arg(args, "task_id")?,
                    })
                    .await?
                }
                "create" => {
                    client::send(&Request::TaskCreate {
                        cwd,
                        title: str_arg(args, "title")
                            .ok_or_else(|| WireError::invalid("title is required"))?,
                        goal: str_arg(args, "goal")
                            .ok_or_else(|| WireError::invalid("goal is required"))?,
                        acceptance_criteria: string_list(args, "acceptance_criteria"),
                    })
                    .await?
                }
                "update" => {
                    client::send(&Request::TaskUpdate {
                        cwd,
                        task_id: uuid_arg(args, "task_id")?,
                        title: str_arg(args, "title"),
                        goal: str_arg(args, "goal"),
                        acceptance_criteria: args
                            .get("acceptance_criteria")
                            .map(|_| string_list(args, "acceptance_criteria")),
                        status: enum_arg(args, "status"),
                    })
                    .await?
                }
                other => return Err(WireError::invalid(format!("unknown action: {other}"))),
            };
            Ok(pretty(&value))
        }

        "cairn_handoff" => {
            let action = required_action(args)?;
            let value = match action.as_str() {
                "latest" => {
                    client::send(&Request::HandoffLatest {
                        cwd,
                        session_id: uuid_arg(args, "session_id").ok(),
                        agent_session_key: key,
                    })
                    .await?
                }
                "generate" => {
                    // `stop` is deliberately absent: a turn checkpoint is not a
                    // handoff boundary (FR-032).
                    client::send(&Request::HandoffGenerate {
                        cwd,
                        session_id: uuid_arg(args, "session_id").ok(),
                        agent_session_key: key,
                        trigger: enum_arg(args, "trigger")
                            .unwrap_or(cairn_core::domain::HandoffTrigger::SessionEnd),
                    })
                    .await?
                }
                "annotate" => {
                    client::send(&Request::HandoffAnnotate {
                        cwd,
                        session_id: uuid_arg(args, "session_id").ok(),
                        agent_session_key: key,
                        note: str_arg(args, "note")
                            .ok_or_else(|| WireError::invalid("note is required"))?,
                    })
                    .await?
                }
                other => return Err(WireError::invalid(format!("unknown action: {other}"))),
            };
            Ok(pretty(&value))
        }

        other => Err(WireError::invalid(format!(
            "unknown tool `{other}`; Cairn exposes {}",
            TOOL_NAMES.join(", ")
        ))),
    }
}

fn required_action(args: &Value) -> Result<String, WireError> {
    args.get("action")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| WireError::invalid("action is required"))
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn enum_arg<T: std::str::FromStr>(args: &Value, key: &str) -> Option<T> {
    args.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
}

fn uuid_arg(args: &Value, key: &str) -> Result<uuid::Uuid, WireError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| WireError::invalid(format!("{key} must be a uuid")))
}

/// A uuid argument that is allowed to be absent.
fn uuid_opt(args: &Value, key: &str) -> Option<uuid::Uuid> {
    args.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
}

fn uuid_list(args: &Value, key: &str) -> Vec<uuid::Uuid> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|s| uuid::Uuid::parse_str(s).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn string_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_exactly_six_tools() {
        let tools = tool_definitions();
        assert_eq!(tools.len(), 6, "FR-040 caps the tool surface at six");
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, TOOL_NAMES);
    }

    #[test]
    fn every_tool_declares_an_object_schema_requiring_cwd() {
        for tool in tool_definitions() {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{}", tool["name"]);
            let required = schema["required"].as_array().unwrap();
            assert!(
                required.iter().any(|r| r == "cwd"),
                "{} does not require cwd",
                tool["name"]
            );
        }
    }

    #[test]
    fn handoff_generate_cannot_be_triggered_by_a_turn_checkpoint() {
        let handoff = tool_definitions()
            .into_iter()
            .find(|t| t["name"] == "cairn_handoff")
            .expect("cairn_handoff");
        let triggers = handoff["inputSchema"]["properties"]["trigger"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        assert!(
            !triggers.contains(&"stop"),
            "stop is a turn boundary, not a handoff trigger"
        );
        assert!(triggers.contains(&"pre_compact"));
        assert!(triggers.contains(&"session_end"));
    }

    #[test]
    fn initialize_carries_the_usage_contract() {
        // FR-129, SC-107: the same rules the managed block states, in the
        // tool-facing form.
        let out = initialize(&json!({}));
        let instructions = out["instructions"].as_str().expect("instructions");
        assert!(instructions.contains("Cairn"));
        assert!(instructions.contains("1. "));
        assert_eq!(
            instructions,
            cairn_integrate::render::Contract::canonical().mcp_instructions()
        );
        // A generic client has neither hooks nor Skills, so neither is
        // mentioned.
        let lower = instructions.to_lowercase();
        assert!(!lower.contains("hook"));
        assert!(!lower.contains("skill"));
    }

    #[test]
    fn the_protocol_revision_does_not_change() {
        // D34, FR-130: `instructions` is a field 2025-06-18 already defines,
        // so adding it is not a protocol bump.
        assert_eq!(PROTOCOL_VERSION, "2025-06-18");
        assert_eq!(initialize(&json!({}))["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn the_surface_is_still_exactly_six_tools() {
        // FR-128, SC-106: a test fails if a seventh appears.
        assert_eq!(TOOL_NAMES.len(), 6, "the MCP surface grew a seventh tool");
        assert_eq!(tool_definitions().len(), 6);
        for forbidden in [
            "cairn_doctor",
            "cairn_repair",
            "cairn_connect",
            "cairn_agents",
            "cairn_integration",
        ] {
            assert!(
                !TOOL_NAMES.contains(&forbidden),
                "{forbidden} is a developer operation, not an agent tool"
            );
        }
    }

    #[test]
    fn initialize_negotiates_a_supported_protocol_version() {
        // A version we support is honoured.
        let out = initialize(&json!({ "protocolVersion": "2025-03-26" }));
        assert_eq!(out["protocolVersion"], "2025-03-26");
        assert!(out["capabilities"]["tools"].is_object());
        assert_eq!(out["serverInfo"]["name"], "cairn");

        // One we do not is answered with ours, not echoed back.
        let future = initialize(&json!({ "protocolVersion": "2999-01-01" }));
        assert_eq!(future["protocolVersion"], PROTOCOL_VERSION);
        let missing = initialize(&json!({}));
        assert_eq!(missing["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn read_tools_advertise_session_identity() {
        // Without this an agent cannot scope a briefing to its own session (M1).
        for name in ["cairn_context", "cairn_search"] {
            let tool = tool_definitions()
                .into_iter()
                .find(|t| t["name"] == name)
                .expect(name);
            let props = &tool["inputSchema"]["properties"];
            assert!(
                props["agent_session_key"].is_object(),
                "{name} hides agent_session_key"
            );
            assert!(props["session_id"].is_object(), "{name} hides session_id");
        }
    }
}

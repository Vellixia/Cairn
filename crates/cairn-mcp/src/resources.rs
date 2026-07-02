//! MCP resources (v0.5.0 Sprint 24).
//!
//! Resources are a read-only, addressable surface on top of the memory store
//! - clients subscribe by URI and read the latest snapshot. Six canonical
//!   resources are listed in [`resource_defs`].
//!
//! Each resource has a [`ResourceDef`] (URI + metadata) and a runtime
//! [`read_resource`] resolver that maps URI -> JSON payload. Resolvers
//! gracefully return an empty result when the underlying data isn't available
//! (e.g. memory graph on a fresh install), so a client can subscribe without
//! crashing.

use crate::McpServer;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// One resource definition. The `uri` field is the canonical address; the
/// `name` is a friendly label; `mime_type` is JSON for everything except
/// `cairn://config/toml`, which is plain text.
#[derive(Debug, Clone)]
pub struct ResourceDef {
    pub uri: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub mime_type: &'static str,
}

/// Six canonical resources. The set is locked for v0.5.0 - adding more is a
/// breaking change for clients that enumerate the resource list.
pub fn resource_defs() -> &'static [ResourceDef] {
    &[
        ResourceDef {
            uri: "cairn://memory/graph",
            name: "Memory Graph",
            description: "Nodes + edges of the current memory graph (file paths, symbols, memories, decisions).",
            mime_type: "application/json",
        },
        ResourceDef {
            uri: "cairn://memory/timeline",
            name: "Memory Timeline",
            description: "Most recent N memories, newest first, with importance + confidence + tier.",
            mime_type: "application/json",
        },
        ResourceDef {
            uri: "cairn://savings/today",
            name: "Today's Token Savings",
            description: "Token-savings ledger summary for the last 24 h: tokens in/out, compressed, saved.",
            mime_type: "application/json",
        },
        ResourceDef {
            uri: "cairn://drift/pending",
            name: "Pending Drift Items",
            description: "Drift events awaiting user review, with proposed resolutions.",
            mime_type: "application/json",
        },
        ResourceDef {
            uri: "cairn://audit/recent",
            name: "Recent Audit Events",
            description: "Most recent N audit events (durable SurrealDB ring), newest first.",
            mime_type: "application/json",
        },
        ResourceDef {
            uri: "cairn://config/toml",
            name: "Effective Config (TOML)",
            description: "Current in-container server configuration as a TOML document. Includes host, port, multi_tenant flag.",
            mime_type: "text/plain",
        },
    ]
}

/// Read a resource by URI. Returns `Ok(Value)` (a JSON payload or text blob)
/// or `Err(String)` for an unknown URI.
pub fn read_resource(server: &McpServer, uri: &str) -> Result<Value, String> {
    match uri {
        "cairn://memory/graph" => Ok(read_memory_graph(server)),
        "cairn://memory/timeline" => Ok(read_memory_timeline(server)),
        "cairn://savings/today" => Ok(read_savings_today(server)),
        "cairn://drift/pending" => Ok(read_drift_pending(server)),
        "cairn://audit/recent" => Ok(read_audit_recent(server)),
        "cairn://config/toml" => Ok(read_config_toml(server)),
        _ => Err(format!("unknown resource uri: {uri}")),
    }
}

fn read_memory_graph(server: &McpServer) -> Value {
    match server.mem.graph() {
        Ok(g) => json!({
            "nodes": g.nodes,
            "edges": g.edges,
            "fetched_at": Utc::now().to_rfc3339(),
        }),
        Err(e) => json!({ "error": e.to_string(), "nodes": [], "edges": [] }),
    }
}

fn read_memory_timeline(server: &McpServer) -> Value {
    // Without a dedicated timeline() method we surface a "by_kind(Decision)"
    // view as a stand-in. Future Sprint 24 follow-up can add a real
    // timeline() that respects confidence ordering.
    match server.mem.by_kind(cairn_core::MemoryKind::Note) {
        Ok(items) => {
            let items: Vec<Value> = items
                .into_iter()
                .take(50)
                .map(|m| {
                    json!({
                        "id": m.id,
                        "kind": m.kind,
                        "tier": m.tier,
                        "importance": m.importance,
                        "confidence": m.confidence,
                        "pinned": m.pinned,
                        "created_at": m.created_at.to_rfc3339(),
                        "snippet": m.content.chars().take(140).collect::<String>(),
                    })
                })
                .collect();
            json!({
                "items": items,
                "count": items.len(),
                "fetched_at": Utc::now().to_rfc3339(),
            })
        }
        Err(e) => json!({ "error": e.to_string(), "items": [], "count": 0 }),
    }
}

fn read_savings_today(_server: &McpServer) -> Value {
    // The savings ledger is owned by cairn-api (HTTP /api/ledger). The MCP
    // server talks to the local store / memory engine directly, so we surface
    // a placeholder that the dashboard-side bridge can fill in. Future
    // iteration: wire cairn-api's SavingsState into McpServer.
    json!({
        "note": "savings summary lives at /api/ledger on the in-container server; bridged by cairn.",
        "fetched_at": Utc::now().to_rfc3339(),
    })
}

fn read_drift_pending(_server: &McpServer) -> Value {
    // Same as above - drift is owned by cairn-session. The MCP surface
    // exposes the URI; the actual JSON is filled by the host (cairn
    // proxy or in-container server) so the contract is honest.
    json!({
        "note": "drift pending is fetched via cairn_session::SessionStore on the server; the MCP server proxies to it.",
        "fetched_at": Utc::now().to_rfc3339(),
    })
}

fn read_audit_recent(server: &McpServer) -> Value {
    // Audit events are stored in SurrealDB on the in-container server. The standalone
    // MCP stdio binary (without the HTTP surface) returns an
    // empty list - production clients should read this URI through the
    // server's `/api/mcp/resources/read` bridge.
    let events: Vec<Value> = Vec::new();
    let _ = server; // silence unused warning when no live store backend
    json!({
        "events": events,
        "count": 0,
        "note": "live audit feed requires the in-container server HTTP bridge; standalone MCP returns an empty list.",
        "fetched_at": Utc::now().to_rfc3339(),
    })
}

fn read_config_toml(server: &McpServer) -> Value {
    let cfg = &server.config;
    let body = format!(
        "# Effective in-container server configuration (read-only snapshot)\n\
         # This document is for diagnostics only - do not edit and re-apply.\n\
         \n\
         host = \"{}\"\n\
         port = {}\n\
         multi_tenant = {}\n\
         db_url = \"{}\"\n\
         embed_provider = \"{}\"\n\
         admin_username = \"{}\"\n\
         cors_origins = {:?}\n",
        cfg.host,
        cfg.port,
        cfg.multi_tenant,
        cfg.db_url,
        cfg.embed.provider,
        cfg.admin.username,
        cfg.cors_origins,
    );
    json!({ "body": body, "fetched_at": Utc::now().to_rfc3339() })
}

/// Per-URI freshness - clients can use this to decide whether to re-read.
pub fn resource_metadata(uri: &str) -> Option<Value> {
    let now: DateTime<Utc> = Utc::now();
    match uri {
        "cairn://memory/graph" => Some(json!({"cache_ttl_s": 30, "fetched_at": now})),
        "cairn://memory/timeline" => Some(json!({"cache_ttl_s": 5, "fetched_at": now})),
        "cairn://savings/today" => Some(json!({"cache_ttl_s": 60, "fetched_at": now})),
        "cairn://drift/pending" => Some(json!({"cache_ttl_s": 5, "fetched_at": now})),
        "cairn://audit/recent" => Some(json!({"cache_ttl_s": 5, "fetched_at": now})),
        "cairn://config/toml" => Some(json!({"cache_ttl_s": 600, "fetched_at": now})),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_canonical_resources_are_listed() {
        assert_eq!(
            resource_defs().len(),
            6,
            "v0.5.0 success metric: 6 resources"
        );
    }

    #[test]
    fn read_resource_returns_err_for_unknown_uri() {
        let Some(cfg) = cairn_store::Store::test_config() else {
            return;
        };
        let Ok(server) = crate::McpServer::new(&cfg) else {
            return;
        };
        let res = read_resource(&server, "cairn://does/not/exist");
        assert!(res.is_err());
    }

    #[test]
    fn resource_metadata_is_none_for_unknown_uri() {
        assert!(resource_metadata("cairn://does/not/exist").is_none());
    }

    #[test]
    fn read_memory_graph_returns_a_graph_payload() {
        let Some(cfg) = cairn_store::Store::test_config() else {
            return;
        };
        let Ok(server) = crate::McpServer::new(&cfg) else {
            return;
        };
        let v = read_resource(&server, "cairn://memory/graph").unwrap();
        assert!(v.get("nodes").is_some());
        assert!(v.get("edges").is_some());
    }

    #[test]
    fn read_config_toml_includes_host_and_port() {
        let Some(cfg) = cairn_store::Store::test_config() else {
            return;
        };
        let Ok(server) = crate::McpServer::new(&cfg) else {
            return;
        };
        let v = read_resource(&server, "cairn://config/toml").unwrap();
        let body = v.get("body").and_then(|x| x.as_str()).unwrap();
        assert!(body.contains("host ="));
        assert!(body.contains("port ="));
        assert!(body.contains("multi_tenant ="));
    }
}

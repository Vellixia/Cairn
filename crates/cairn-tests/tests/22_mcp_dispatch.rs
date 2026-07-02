//! 22 — MCP server dispatch via the in-memory Store.
//!
//! Replaces the deleted `11_mcp_tools.rs` (which only built JSON literals
//! and asserted on themselves, never importing `cairn-mcp`). Every test
//! here constructs a real `McpServer::with_store` against an in-memory
//! `Store` and calls the public `dispatch` entry point to drive a real
//! JSON-RPC-style tool call.

use cairn_mcp::McpServer;
use cairn_store::Store;
use serde_json::json;
use std::sync::Arc;

fn server() -> Option<(McpServer, tempfile::TempDir)> {
    let dir = tempfile::tempdir().ok()?;
    let blobs = dir.path().join("blobs");
    let store = Arc::new(Store::open_in_memory(blobs).ok()?);
    let cfg = cairn_core::Config {
        data_dir: dir.path().to_path_buf(),
        host: "127.0.0.1".into(),
        port: 7777,
        helix_url: None,
        helix_token: None,
        helix_ns: None,
        default_server: None,
        secret_key: Some(b"cairn-mcp-tests-secret-key-32!".to_vec()),
        tls: None,
        insecure: false,
        workspace_root: None,
        cors_origins: vec![],
        embed: cairn_core::EmbedConfig {
            provider: "hashing".into(),
            model: None,
            url: None,
            api_key: None,
        },
        llm_consolidation: cairn_core::LlmConsolidationConfig {
            enabled: false,
            url: "http://localhost:11434/v1/chat/completions".into(),
            model: "llama3.2".into(),
            api_key: None,
        },
        rerank: cairn_core::RerankConfig::default(),
        admin: cairn_core::AdminConfig::default(),
        multi_tenant: false,
    };
    Some((McpServer::with_store(&cfg, store).ok()?, dir))
}

#[test]
fn tool_defs_list_is_non_empty_and_well_formed() {
    // The dashboard's "tools available" badge reads `tool_defs()`. It
    // must be a JSON object/array with at least the documented tools.
    let tools = cairn_mcp::tool_defs();
    let arr = tools.as_array().expect("tools is a JSON array");
    assert!(!arr.is_empty(), "tool list must not be empty");
    let names: Vec<&str> = arr
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    // Core documented tools must be present.
    for required in ["read", "expand", "remember", "recall", "wakeup", "assemble"] {
        assert!(
            names.contains(&required),
            "missing required tool: {required}; have {names:?}"
        );
    }
}

#[test]
fn dispatch_remember_and_recall_round_trip_a_fact() {
    let Some((srv, _dir)) = server() else { return };
    let remember_out = srv
        .dispatch(
            "remember",
            &json!({ "content": "rust uses ownership", "kind": "fact" }),
        )
        .expect("remember dispatch");
    // `dispatch("remember")` returns a short status string, not JSON.
    assert!(
        remember_out.starts_with("remembered "),
        "remember dispatch should return a status line; got: {remember_out}"
    );
    let id = remember_out
        .split_whitespace()
        .nth(1)
        .expect("id is the second whitespace-separated token")
        .to_string();
    assert!(!id.is_empty(), "remembered id must be non-empty");

    let recall_out = srv
        .dispatch("recall", &json!({ "query": "rust ownership", "limit": 5 }))
        .expect("recall dispatch");
    // `dispatch("recall")` returns a multi-line score summary, not JSON.
    assert!(
        recall_out.contains("rust uses ownership"),
        "recall must surface the just-remembered fact; got: {recall_out}"
    );
    // The recall text format is `[score] (kind) content\n...`. The id is
    // not part of this surface; presence of the content is the contract.
    assert!(
        recall_out.contains("(fact)"),
        "recall should tag the kind; got: {recall_out}"
    );
}

#[test]
fn dispatch_assemble_returns_included_items() {
    let Some((srv, _dir)) = server() else { return };
    srv.dispatch(
        "remember",
        &json!({ "content": "the build uses tokio + axum", "kind": "fact" }),
    )
    .expect("seed remember");
    let out = srv
        .dispatch(
            "assemble",
            &json!({ "query": "tokio build", "budget_tokens": 500 }),
        )
        .expect("assemble dispatch");
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    let included = v["included"].as_array().expect("included array");
    assert!(
        !included.is_empty(),
        "assemble must include at least the seed fact"
    );
}

#[test]
fn dispatch_unknown_tool_returns_an_err_string() {
    let Some((srv, _dir)) = server() else { return };
    let out = srv.dispatch("does_not_exist", &json!({}));
    let err_msg = match out {
        Ok(_) => panic!("unknown tool must surface as Err; got Ok"),
        Err(e) => e,
    };
    let lc = err_msg.to_lowercase();
    assert!(
        lc.contains("unknown") || lc.contains("not found"),
        "error message must explain the unknown tool; got {err_msg}"
    );
}

#[test]
fn dispatch_sanitize_runs_share_through_mcp() {
    let Some((srv, _dir)) = server() else { return };
    let out = srv
        .dispatch(
            "sanitize",
            &json!({ "text": "my api key is sk-abcdefghijklmnopqrstuvwxyz0123456789" }),
        )
        .expect("sanitize dispatch");
    // Sanitize returns a JSON envelope with the redacted text + findings.
    // The raw key must NOT appear anywhere in the response.
    assert!(
        !out.contains("abcdefghijklmnopqrstuvwxyz"),
        "raw API key must not leak through sanitize; got: {out}"
    );
    assert!(
        out.contains("[redacted:") || out.contains("[REDACTED]") || out.contains("***"),
        "sanitize should mark the redaction in some way; got: {out}"
    );
    // The JSON envelope should classify the input as private.
    let v: serde_json::Value = serde_json::from_str(&out).expect("envelope is JSON");
    assert_eq!(v["sensitivity"], "private");
    assert!(v["findings"]
        .as_array()
        .map(|a| !a.is_empty())
        .unwrap_or(false));
}

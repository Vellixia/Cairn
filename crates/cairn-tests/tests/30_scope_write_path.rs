//! 30 — Scope-aware write path: MCP dispatch with ScopeCtx, project-default scoping,
//! promotion pipeline, document RAG tools, and graph edge feeding via files.
//!
//! Exercises the real `McpServer::dispatch` with a `ScopeCtx` carrying a project_id,
//! verifying that:
//! 1. `remember` defaults to Project scope when scope.project_id is Some.
//! 2. `remember` with explicit scope_type=global overrides the default.
//! 3. `remember` with files populates applies_to edges in the graph.
//! 4. `document_ingest` + `document_search` round-trip with project scoping.
//! 5. Promotion candidates surface Project-scoped memories.

use cairn_core::{ScopeCtx, ScopeType};
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
        db_url: "ws://localhost:8000".into(),
        db_user: "root".into(),
        db_pass: String::new(),
        db_ns: "cairn".into(),
        db_timeout_secs: 10,
        default_server: None,
        secret_key: Some("cairn-scope-tests-secret-key-32!".as_bytes().to_vec()),
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
        session_ttl_days: 2,
        decay_period_days: 30,
        access_log_retention_days: 90,
        cron_enabled: true,
        promote_threshold: 0.85,
        demote_idle_days: 45,
        drift_autopilot: "safe".to_string(),
        drift_safe_globs: vec![
            "docs/**".to_string(),
            "*.md".to_string(),
            "**/tests/**".to_string(),
            "**/*.test.*".to_string(),
        ],
        auto_anchor: true,
        llm_daily_budget: 200_000,
        selftune: true,
        max_working_per_project: 500,
    };
    Some((McpServer::with_store(&cfg, store).ok()?, dir))
}

fn project_scope(pid: &str) -> ScopeCtx {
    ScopeCtx {
        project_id: Some(pid.to_string()),
        session_id: None,
    }
}

#[test]
fn remember_defaults_to_project_when_scope_has_project_id() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-alpha");
    let out = srv
        .dispatch(
            "remember",
            &json!({ "content": "project-scoped decision", "kind": "decision" }),
            &scope,
        )
        .expect("remember dispatch");
    assert!(out.contains("remembered"), "got: {out}");
    assert!(
        out.contains("project"),
        "should be project-scoped; got: {out}"
    );

    // Verify the memory is actually Project-scoped in the store.
    let mems = srv.store.all_memories().expect("all_memories");
    let m = mems
        .iter()
        .find(|m| m.content == "project-scoped decision")
        .expect("memory exists");
    assert_eq!(m.scope_type, ScopeType::Project);
    assert_eq!(m.scope_id.as_deref(), Some("proj-alpha"));
}

#[test]
fn remember_explicit_global_overrides_project_default() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-beta");
    let out = srv
        .dispatch(
            "remember",
            &json!({
                "content": "global fact despite project context",
                "kind": "fact",
                "scope_type": "global"
            }),
            &scope,
        )
        .expect("remember dispatch");
    assert!(out.contains("global"), "should be global; got: {out}");

    let mems = srv.store.all_memories().expect("all_memories");
    let m = mems
        .iter()
        .find(|m| m.content == "global fact despite project context")
        .expect("memory exists");
    assert_eq!(m.scope_type, ScopeType::Global);
    assert!(m.scope_id.is_none());
}

#[test]
fn remember_defaults_to_global_when_no_project() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = ScopeCtx::default();
    let out = srv
        .dispatch(
            "remember",
            &json!({ "content": "no project context", "kind": "note" }),
            &scope,
        )
        .expect("remember dispatch");
    assert!(out.contains("global"), "should be global; got: {out}");

    let mems = srv.store.all_memories().expect("all_memories");
    let m = mems
        .iter()
        .find(|m| m.content == "no project context")
        .expect("memory exists");
    assert_eq!(m.scope_type, ScopeType::Global);
}

#[test]
fn remember_with_files_creates_applies_to_edges() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-gamma");
    srv.dispatch(
        "remember",
        &json!({
            "content": "fixed auth bug in login handler",
            "kind": "decision",
            "files": ["src/auth.rs", "src/login.rs"]
        }),
        &scope,
    )
    .expect("remember dispatch");

    let g = srv.mem.graph().expect("graph");
    // The memory should have applies_to edges pointing at the file paths.
    let file_edges: Vec<_> = g.edges.iter().filter(|e| e.kind == "applies_to").collect();
    assert!(
        !file_edges.is_empty(),
        "should have applies_to edges for files"
    );
    let targets: Vec<_> = file_edges.iter().map(|e| e.target.as_str()).collect();
    assert!(targets.contains(&"src/auth.rs"));
    assert!(targets.contains(&"src/login.rs"));
}

#[test]
fn document_ingest_and_search_round_trip() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-delta");
    let content = "Rust ownership model: each value has a single owner. \
                   When the owner goes out of scope, the value is dropped. \
                   Borrowing allows temporary access without transferring ownership.";

    let out = srv
        .dispatch(
            "document_ingest",
            &json!({
                "source": "test-doc.md",
                "content": content,
                "title": "Rust Ownership Guide"
            }),
            &scope,
        )
        .expect("ingest dispatch");
    assert!(out.contains("ingested"), "got: {out}");
    assert!(out.contains("chunks"), "got: {out}");

    let search_out = srv
        .dispatch(
            "document_search",
            &json!({ "query": "ownership", "limit": 5 }),
            &scope,
        )
        .expect("search dispatch");
    assert!(
        search_out.contains("ownership") || search_out.contains("owner"),
        "search should find the ingested content; got: {search_out}"
    );
}

#[test]
fn document_ingest_without_content_reads_from_filesystem() {
    let Some((srv, dir)) = server() else {
        return;
    };
    let file_path = dir.path().join("local-doc.txt");
    std::fs::write(
        &file_path,
        "This is a test document about dependency injection patterns.",
    )
    .expect("write file");

    let scope = project_scope("proj-epsilon");
    let out = srv
        .dispatch(
            "document_ingest",
            &json!({ "source": file_path.to_str().unwrap() }),
            &scope,
        )
        .expect("ingest dispatch");
    assert!(out.contains("ingested"), "got: {out}");
}

#[test]
fn project_scoped_memory_visible_in_promotion_candidates() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-zeta");

    // Write a project-scoped semantic memory with high importance.
    let out = srv
        .dispatch(
            "remember",
            &json!({
                "content": "important architectural decision for this project",
                "kind": "decision",
                "tier": "semantic",
                "importance": 0.9
            }),
            &scope,
        )
        .expect("remember dispatch");
    let id = out
        .split_whitespace()
        .nth(1)
        .expect("id in output")
        .to_string();

    // Manually set promo_score into the review band so it surfaces as a candidate.
    if let Some(mut m) = srv.mem.get(&id).expect("get") {
        m.promo_score = 0.75;
        srv.store.upsert_memory(&m).expect("upsert");
    }

    // Promotion candidates filters for Project-scoped, Semantic/Procedural, score in [0.70, 0.90].
    let candidates = srv
        .mem
        .promotion_candidates()
        .expect("promotion_candidates");
    assert!(
        candidates
            .iter()
            .any(|m| m.content == "important architectural decision for this project"),
        "project-scoped memory should appear in promotion candidates"
    );
}

#[test]
fn project_scoped_memory_promotes_to_global() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-eta");

    let out = srv
        .dispatch(
            "remember",
            &json!({
                "content": "candidate for global promotion",
                "kind": "decision",
                "importance": 0.8
            }),
            &scope,
        )
        .expect("remember dispatch");
    let id = out
        .split_whitespace()
        .nth(1)
        .expect("id in output")
        .to_string();

    // Manually promote it.
    let promoted = srv.mem.promote_memory(&id).expect("promote_memory");
    assert!(promoted, "promotion should succeed");

    // Verify it's now Global.
    let m = srv.mem.get(&id).expect("get").expect("memory exists");
    assert_eq!(m.scope_type, ScopeType::Global);
    assert!(m.scope_id.is_none());
    assert!(m.promo_locked, "should be locked after promotion");
}

#[test]
fn remember_with_concepts_populates_concept_fields() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-theta");
    srv.dispatch(
        "remember",
        &json!({
            "content": "use tokio for async runtime",
            "kind": "decision",
            "concepts": ["async", "tokio", "runtime"]
        }),
        &scope,
    )
    .expect("remember dispatch");

    let mems = srv.store.all_memories().expect("all_memories");
    let m = mems
        .iter()
        .find(|m| m.content == "use tokio for async runtime")
        .expect("memory exists");
    assert_eq!(m.concepts, vec!["async", "tokio", "runtime"]);
}

#[test]
fn document_search_returns_no_matches_when_empty() {
    let Some((srv, _dir)) = server() else {
        return;
    };
    let scope = project_scope("proj-iota");
    let out = srv
        .dispatch(
            "document_search",
            &json!({ "query": "nonexistent topic" }),
            &scope,
        )
        .expect("search dispatch");
    assert!(
        out.contains("no document chunks"),
        "should report no matches; got: {out}"
    );
}

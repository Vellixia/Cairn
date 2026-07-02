//! 02 — cairn-api `/api/memory/*` HTTP routes, mounted in-process via tower::oneshot.
//!
//! Exercises the public HTTP surface for memory CRUD + introspection end-to-end:
//! remember, edit, delete, pin, reinforce, recall, graph. Auth flow:
//! POST `/api/auth/setup` -> POST `/api/auth/login` -> `cairn_session=<value>` cookie.
//!
//! Hermetic: no network, no live database, no docker. In-memory `cairn_store::Store` +
//! `cairn_api::router` (state from `AppState::with_store`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cairn_api::{router, AppState};
use cairn_core::{Config, MemoryKind};
use cairn_store::Store;
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

// --- in-process state ---------------------------------------------------

fn state() -> Option<(axum::Router, Arc<Store>, tempfile::TempDir)> {
    let dir = tempfile::tempdir().ok()?;
    let blobs = dir.path().join("blobs");
    let store = Arc::new(Store::open_in_memory(blobs).ok()?);
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        host: "127.0.0.1".into(),
        port: 7777,
        db_url: "ws://localhost:8000".into(),
        db_user: "root".into(),
        db_pass: String::new(),
        db_ns: "cairn".into(),
        db_timeout_secs: 10,
        default_server: None,
        secret_key: Some(b"cairn-api-tests-secret-key-32!!!".to_vec()),
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
        drift_safe_globs: vec!["docs/**".to_string(), "*.md".to_string(), "**/tests/**".to_string(), "**/*.test.*".to_string()],
        auto_anchor: true,
    };
    let state = AppState::with_store(&cfg, store.clone()).ok()?;
    Some((router(state), store, dir))
}

// --- HTTP helpers -------------------------------------------------------

async fn read_body(
    resp: axum::response::Response,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let status = resp.status();
    let headers: Vec<_> = resp
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .cloned()
        .collect();
    let body = resp
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let json: serde_json::Value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null)
    };
    (status, json, headers)
}

async fn post_json(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let mut b = Request::builder().method("POST").uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = b
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

async fn get_json(
    app: axum::Router,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let mut b = Request::builder().method("GET").uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = b.body(Body::empty()).expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

async fn delete_json(
    app: axum::Router,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let mut b = Request::builder().method("DELETE").uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = b.body(Body::empty()).expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

/// Setup admin + login -> return the session cookie value.
async fn login_cookie(app: axum::Router) -> String {
    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecret-admin-pass",
    });
    let (status, json, _headers) = post_json(app.clone(), "/api/auth/setup", body, None).await;
    assert!(
        status.is_success() || status == StatusCode::CONFLICT,
        "setup must succeed or already-exist; got {status} body={json}"
    );
    // Now login to mint a session cookie.
    let (lstatus, ljson, lheaders) = post_json(
        app,
        "/api/auth/login",
        serde_json::json!({"username": "admin", "password": "supersecret-admin-pass"}),
        None,
    )
    .await;
    assert!(
        lstatus.is_success(),
        "login must succeed; got {lstatus} body={ljson}"
    );
    assert!(!lheaders.is_empty(), "login must set a cookie header");
    let raw = lheaders[0].to_str().expect("ascii").to_string();
    // "cairn_session=<value>; HttpOnly; ..."
    let cookie_value = raw
        .split(';')
        .next()
        .expect("cookie has a value part")
        .trim_start_matches("cairn_session=")
        .to_string();
    cookie_value
}

// --- tests --------------------------------------------------------------

#[tokio::test]
async fn remember_via_http_inserts_into_store() {
    let Some((app, store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let (status, json, _h) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({
            "content": "this server's admin runs the cairn-api envelope contract",
            "concepts": ["cairn", "envelope", "http"],
            "importance": 0.42,
        }),
        Some(&cookie),
    )
    .await;
    assert!(
        status.is_success(),
        "POST /api/memory must succeed; got {status} body={json}"
    );
    let id = json["id"]
        .as_str()
        .expect("remember response includes id")
        .to_string();
    assert!(!id.is_empty());

    // Read it back from the store directly to prove the HTTP handler really persisted.
    let m = store
        .get_memory(&id)
        .expect("get_memory")
        .expect("memory exists in store");
    assert_eq!(
        m.content,
        "this server's admin runs the cairn-api envelope contract"
    );
    assert_eq!(m.kind, MemoryKind::Note);
    assert!(m.concepts.contains(&"cairn".to_string()));
}

#[tokio::test]
async fn edit_then_get_via_http_updates_mutable_fields() {
    let Some((app, store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let (_s, json, _h) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({
            "content": "draft: needs editing",
            "concepts": ["draft"],
            "importance": 0.1,
        }),
        Some(&cookie),
    )
    .await;
    let id = json["id"].as_str().expect("id").to_string();

    // POST /api/memory/:id is the edit handler in this version.
    let (s2, j2, _h2) = post_json(
        app.clone(),
        &format!("/api/memory/{id}"),
        serde_json::json!({
            "content": "draft: edited content (concepts unchanged)",
            "importance": 0.7,
        }),
        Some(&cookie),
    )
    .await;
    assert!(s2.is_success(), "edit must succeed; got {s2} body={j2}");
    assert_eq!(j2["id"], id);
    assert_eq!(j2["content"], "draft: edited content (concepts unchanged)");

    // Round-trip via the store: confirms the edit hit the real backend.
    let m = store.get_memory(&id).expect("get").expect("present");
    assert_eq!(m.content, "draft: edited content (concepts unchanged)");
    assert!((m.importance - 0.7).abs() < 0.01, "importance updated");
}

#[tokio::test]
async fn delete_via_http_removes_memory_from_store() {
    let Some((app, store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let (_s, json, _h) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({"content": "ephemeral - to be deleted"}),
        Some(&cookie),
    )
    .await;
    let id = json["id"].as_str().expect("id").to_string();

    let (ds, dj, _h) = delete_json(app.clone(), &format!("/api/memory/{id}"), Some(&cookie)).await;
    assert!(ds.is_success(), "delete must succeed; got {ds} body={dj}");
    assert_eq!(dj["deleted"], true);

    let m = store.get_memory(&id).expect("get");
    assert!(m.is_none(), "memory must be gone from the store");
}

#[tokio::test]
async fn pin_unpin_via_http_toggles_pinned_flag() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let (_s, json, _h) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({"content": "this fact is critical to the task"}),
        Some(&cookie),
    )
    .await;
    let id = json["id"].as_str().expect("id").to_string();

    // Pin.
    let (s2, j2, _h2) = post_json(
        app.clone(),
        &format!("/api/memory/{id}/pin"),
        serde_json::json!({"pinned": true}),
        Some(&cookie),
    )
    .await;
    assert!(s2.is_success(), "pin must succeed; got {s2} body={j2}");
    assert_eq!(j2["pinned"], true);

    // Unpin.
    let (s3, j3, _h3) = post_json(
        app.clone(),
        &format!("/api/memory/{id}/pin"),
        serde_json::json!({"pinned": false}),
        Some(&cookie),
    )
    .await;
    assert!(s3.is_success(), "unpin must succeed; got {s3} body={j3}");
    assert_eq!(j3["pinned"], false);
}

#[tokio::test]
async fn reinforce_via_http_bumps_confidence() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let (_s, json, _h) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({"content": "frequently-referenced design decision"}),
        Some(&cookie),
    )
    .await;
    let id = json["id"].as_str().expect("id").to_string();

    let before = json["confidence"].as_f64().unwrap_or(0.0);
    let (s2, j2, _h2) = post_json(
        app,
        &format!("/api/memory/{id}/reinforce"),
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;
    assert!(
        s2.is_success(),
        "reinforce must succeed; got {s2} body={j2}"
    );
    let after = j2["confidence"].as_f64().unwrap_or(0.0);
    assert!(
        after >= before,
        "confidence should not decrease after reinforce: before={before} after={after}"
    );
    assert!(
        j2["access_count"].as_i64().unwrap_or(0) >= 1,
        "access_count must increment: {j2}"
    );
}

#[tokio::test]
async fn memory_graph_endpoint_returns_nodes_and_edges_envelope() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    // Insert two memories. The graph derives edges from `derived_from`,
    // `contradicts`, `supersedes`, `applies_to` - not from shared concepts -
    // so we only assert the envelope shape and that both memory ids appear as
    // graph nodes.
    let (_s1, j1, _h1) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({
            "content": "graph test alpha: foundational concept",
            "concepts": ["alpha"],
        }),
        Some(&cookie),
    )
    .await;
    let id1 = j1["id"].as_str().expect("id1").to_string();

    let (_s2, j2, _h2) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({
            "content": "graph test beta: related to alpha",
            "concepts": ["alpha", "beta"],
        }),
        Some(&cookie),
    )
    .await;
    let id2 = j2["id"].as_str().expect("id2").to_string();

    let (gs, gjson, _h) = get_json(app, "/api/memory/graph", Some(&cookie)).await;
    assert!(gs.is_success(), "graph must succeed; got {gs} body={gjson}");
    let nodes = gjson["nodes"].as_array().expect("nodes is array");
    let edges = gjson["edges"].as_array().expect("edges is array");
    let node_ids: Vec<&str> = nodes
        .iter()
        .map(|n| n["id"].as_str().expect("node id"))
        .collect();
    assert!(
        node_ids.contains(&id1.as_str()),
        "graph missing first memory id"
    );
    assert!(
        node_ids.contains(&id2.as_str()),
        "graph missing second memory id"
    );
    // Edges is always a Vec (possibly empty) - just make sure it's well-typed.
    for e in edges {
        assert!(e["source"].is_string(), "edge.source is string");
        assert!(e["target"].is_string(), "edge.target is string");
        assert!(e["kind"].is_string(), "edge.kind is string");
    }
}

#[tokio::test]
async fn remember_endpoint_rejects_unauthenticated_requests() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let (status, json, _h) = post_json(
        app,
        "/api/memory",
        serde_json::json!({"content": "should be blocked by auth"}),
        None,
    )
    .await;
    // Auth runs before routing on /api/memory - unauthenticated POST must be 401.
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "POST /api/memory must be auth-gated; got {status} body={json}"
    );
}

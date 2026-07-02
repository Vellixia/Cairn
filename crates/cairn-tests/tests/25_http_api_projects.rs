//! 25 — cairn-api `/api/projects/*` HTTP routes (v0.8.0 Sprint 3), mounted in-process via
//! tower::oneshot. Mirrors `02_http_api_memory.rs`'s auth flow: POST `/api/auth/setup` ->
//! POST `/api/auth/login` -> `cairn_session=<value>` cookie.
//!
//! Hermetic: no network, no live database, no docker. In-memory `cairn_store::Store` +
//! `cairn_api::router` (state from `AppState::with_store`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cairn_api::{router, AppState};
use cairn_core::Config;
use cairn_store::Store;
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

fn state() -> Option<(axum::Router, tempfile::TempDir)> {
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
    let state = AppState::with_store(&cfg, store).ok()?;
    Some((router(state), dir))
}

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

async fn request_json(
    app: axum::Router,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

/// Setup admin + login -> return the session cookie value.
async fn login_cookie(app: axum::Router) -> String {
    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecret-admin-pass",
    });
    let (status, json, _headers) =
        request_json(app.clone(), "POST", "/api/auth/setup", Some(body), None).await;
    assert!(
        status.is_success() || status == StatusCode::CONFLICT,
        "setup must succeed or already-exist; got {status} body={json}"
    );
    let (lstatus, ljson, lheaders) = request_json(
        app,
        "POST",
        "/api/auth/login",
        Some(serde_json::json!({"username": "admin", "password": "supersecret-admin-pass"})),
        None,
    )
    .await;
    assert!(
        lstatus.is_success(),
        "login must succeed; got {lstatus} body={ljson}"
    );
    assert!(!lheaders.is_empty(), "login must set a cookie header");
    let raw = lheaders[0].to_str().expect("ascii").to_string();
    raw.split(';')
        .next()
        .expect("cookie has a value part")
        .trim_start_matches("cairn_session=")
        .to_string()
}

#[tokio::test]
async fn upsert_then_get_returns_the_project() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = request_json(
        app.clone(),
        "PATCH",
        "/api/projects/upsert",
        Some(serde_json::json!({
            "id": "proj-alpha-hash",
            "name": "proj-alpha",
            "path": "/home/dev/proj-alpha",
        })),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "upsert should succeed; got {status} body={body}");
    assert_eq!(body["id"], "proj-alpha-hash");
    assert_eq!(body["name"], "proj-alpha");
    assert!(body["first_seen"].is_string());
    assert!(body["last_active"].is_string());

    let (status, body, _) = request_json(
        app,
        "GET",
        "/api/projects/proj-alpha-hash",
        None,
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "get should succeed; got {status} body={body}");
    assert_eq!(body["name"], "proj-alpha");
    assert_eq!(body["path"], "/home/dev/proj-alpha");
}

#[tokio::test]
async fn repeated_upsert_preserves_first_seen_and_updates_name() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (_, first, _) = request_json(
        app.clone(),
        "PATCH",
        "/api/projects/upsert",
        Some(serde_json::json!({"id": "proj-x", "name": "old-name", "path": "/p"})),
        Some(&cookie),
    )
    .await;
    let (_, second, _) = request_json(
        app,
        "PATCH",
        "/api/projects/upsert",
        Some(serde_json::json!({"id": "proj-x", "name": "new-name", "path": "/p"})),
        Some(&cookie),
    )
    .await;
    assert_eq!(second["name"], "new-name", "name must update on re-upsert");
    assert_eq!(
        first["first_seen"], second["first_seen"],
        "first_seen must survive a repeated upsert"
    );
}

#[tokio::test]
async fn list_includes_every_upserted_project() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    for id in ["proj-a", "proj-b"] {
        let (status, _, _) = request_json(
            app.clone(),
            "PATCH",
            "/api/projects/upsert",
            Some(serde_json::json!({"id": id, "name": id, "path": "/p"})),
            Some(&cookie),
        )
        .await;
        assert!(status.is_success());
    }

    let (status, body, _) =
        request_json(app, "GET", "/api/projects", None, Some(&cookie)).await;
    assert!(status.is_success());
    let list = body.as_array().expect("list is a JSON array");
    let ids: Vec<&str> = list.iter().filter_map(|p| p["id"].as_str()).collect();
    assert!(ids.contains(&"proj-a"));
    assert!(ids.contains(&"proj-b"));
}

#[tokio::test]
async fn get_unknown_project_returns_404() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, _, _) = request_json(
        app,
        "GET",
        "/api/projects/does-not-exist",
        None,
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn project_routes_require_authentication() {
    let Some((app, _dir)) = state() else { return };
    let (status, _, _) = request_json(app, "GET", "/api/projects", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

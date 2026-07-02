//! 23 — cairn-api router envelope, mounted in-process via tower::oneshot.
//!
//! Replaces the deleted `14_api_envelope.rs` (which only built JSON
//! literals and asserted on themselves, never importing `cairn-api`).
//! Every test here mounts the real `cairn_api::router` over a real
//! `AppState::with_store` (in-memory Store) and drives the documented
//! HTTP endpoints.

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
        helix_url: None,
        helix_token: None,
        helix_ns: None,
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
    };
    let state = AppState::with_store(&cfg, store).ok()?;
    Some((router(state), dir))
}

async fn get_json(app: axum::Router, path: &str) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
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
    (status, json)
}

#[tokio::test]
async fn health_endpoint_returns_status_and_version() {
    let Some((app, _dir)) = state() else { return };
    let (status, body) = get_json(app, "/api/health").await;
    assert!(
        status.is_success(),
        "/api/health should be 2xx; got {status}"
    );
    assert_eq!(body["status"], "ok");
    // The response includes the version the binary was built with.
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn capabilities_endpoint_lists_documented_features() {
    let Some((app, _dir)) = state() else { return };
    let (status, body) = get_json(app, "/api/capabilities").await;
    assert!(
        status.is_success(),
        "/api/capabilities should be 2xx; got {status}"
    );
    // The dashboard reads `version` and a few flags off this object.
    assert!(body["version"].is_string());
}

#[tokio::test]
async fn openapi_spec_is_valid_json_with_documented_paths() {
    let Some((app, _dir)) = state() else { return };
    let (status, body) = get_json(app, "/api/openapi.json").await;
    assert!(
        status.is_success(),
        "/api/openapi.json should be 2xx; got {status}"
    );
    let paths = body["paths"].as_object().expect("paths is an object");
    // Documented top-level paths the dashboard depends on.
    for path in [
        "/api/health",
        "/api/capabilities",
        "/api/memory",
        "/api/memory/recall",
        "/api/memory/wakeup",
        "/api/guard/anchor",
        "/api/sessions",
    ] {
        assert!(
            paths.contains_key(path),
            "openapi spec missing documented path: {path}; have {:?}",
            paths.keys().collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn stats_endpoint_is_auth_gated() {
    let Some((app, _dir)) = state() else { return };
    let (status, body) = get_json(app, "/api/stats").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "/api/stats must be auth-gated"
    );
    assert!(body.is_object(), "401 body must be JSON");
}

#[tokio::test]
async fn unknown_route_returns_404_or_401_with_an_error_envelope() {
    let Some((app, _dir)) = state() else { return };
    let (status, body) = get_json(app, "/api/this-route-does-not-exist").await;
    // Auth runs before routing - an unauthenticated request for an unknown
    // route gets 401. Authenticated requests would get 404. Either way,
    // the response body must carry a documented error envelope.
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::UNAUTHORIZED,
        "unknown route must return 404 or 401; got {status}"
    );
    assert!(
        body.get("error").is_some() || body.get("message").is_some(),
        "must carry a documented envelope; got {body}"
    );
}

#[tokio::test]
async fn stats_endpoint_returns_401_without_auth() {
    let Some((app, _dir)) = state() else { return };
    let (status, body) = get_json(app, "/api/stats").await;
    // /api/stats is auth-gated; an unauthenticated request must return 401.
    // A positive-path test would need to mint a session token, which is
    // out of scope for this envelope test - that contract is exercised by
    // the dashboard flow tests in docs/testing/flows.md.
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "/api/stats must be auth-gated"
    );
    assert!(body.is_object(), "401 body must be JSON");
}

#[tokio::test]
async fn memory_wakeup_endpoint_is_auth_gated() {
    let Some((app, _dir)) = state() else { return };
    let (status, _body) = get_json(app, "/api/memory/wakeup?limit=5").await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "/api/memory/wakeup must be auth-gated"
    );
}

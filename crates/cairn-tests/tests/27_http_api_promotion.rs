//! 27 — cairn-api `/api/memory/promotion-candidates`, `/api/memory/:id/promote`,
//! `/api/memory/:id/dismiss-promotion`, and `/api/memory/session-summary` (v0.8.0 Sprint 5),
//! mounted in-process via tower::oneshot.
//!
//! `promo_score`/`promo_locked` are cron/LLM-computed only - there's no HTTP way to set them
//! directly (by design, same as `access_log`), so tests that need a memory already in a given
//! promotion state seed it through the `Arc<Store>` returned by `state()`, then exercise the
//! HTTP surface against it. This mirrors `02_http_api_memory.rs`'s use of the store handle for
//! post-write assertions, just used for setup instead.
//!
//! Hermetic: no network, no live database, no docker. In-memory `cairn_store::Store` +
//! `cairn_api::router` (state from `AppState::with_store`).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cairn_api::{router, AppState};
use cairn_core::{Config, Memory, MemoryKind, MemoryTier, NewMemory, ScopeType};
use cairn_store::Store;
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

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
    let state = AppState::with_store(&cfg, store.clone()).ok()?;
    Some((router(state), store, dir))
}

/// Seed a `Project`-scoped memory at a given `promo_score`/`promo_locked`, directly through the
/// store (see module docs for why).
fn seed_candidate(store: &Store, content: &str, promo_score: f32, promo_locked: bool) -> Memory {
    let mut m = NewMemory {
        content: content.to_string(),
        kind: Some(MemoryKind::Fact),
        tier: Some(MemoryTier::Semantic),
        scope_type: ScopeType::Project,
        scope_id: Some("proj-alpha".to_string()),
        ..Default::default()
    }
    .into_memory();
    m.promo_score = promo_score;
    m.promo_locked = promo_locked;
    store.insert_memory(&m).expect("seed candidate");
    m
}

async fn request_json(
    app: axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = b.body(Body::empty()).expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
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
    let status = resp.status();
    let headers: Vec<_> = resp
        .headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .cloned()
        .collect();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect")
        .to_bytes();
    let json: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json, headers)
}

async fn login_cookie(app: axum::Router) -> String {
    let (status, json, _) = post_json(
        app.clone(),
        "/api/auth/setup",
        serde_json::json!({"username": "admin", "password": "supersecret-admin-pass"}),
        None,
    )
    .await;
    assert!(
        status.is_success() || status == StatusCode::CONFLICT,
        "setup must succeed or already-exist; got {status} body={json}"
    );
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
    let raw = lheaders[0].to_str().expect("ascii").to_string();
    raw.split(';')
        .next()
        .expect("cookie has a value part")
        .trim_start_matches("cairn_session=")
        .to_string()
}

#[tokio::test]
async fn candidates_lists_only_the_review_band_and_excludes_locked() {
    let Some((app, store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let in_band = seed_candidate(&store, "in the review band", 0.80, false);
    seed_candidate(&store, "below the band", 0.50, false);
    seed_candidate(&store, "in band but locked", 0.85, true);

    let (status, body, _) = request_json(
        app,
        "GET",
        "/api/memory/promotion-candidates",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    let list = body.as_array().expect("array");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], in_band.id);
}

#[tokio::test]
async fn promote_moves_to_global_and_locks() {
    let Some((app, store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;
    let candidate = seed_candidate(&store, "promotable fact", 0.80, false);

    let (status, body, _) = request_json(
        app,
        "POST",
        &format!("/api/memory/{}/promote", candidate.id),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["scope_type"], "global");
    assert!(body["scope_id"].is_null());
    assert_eq!(body["promo_locked"], true);
}

#[tokio::test]
async fn promote_unknown_id_returns_404() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;
    let (status, _, _) = request_json(
        app,
        "POST",
        "/api/memory/does-not-exist/promote",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dismiss_promotion_locks_without_changing_scope() {
    let Some((app, store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;
    let candidate = seed_candidate(&store, "dismissable candidate", 0.80, false);

    let (status, body, _) = request_json(
        app.clone(),
        "POST",
        &format!("/api/memory/{}/dismiss-promotion", candidate.id),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["scope_type"], "project");
    assert_eq!(body["promo_locked"], true);

    // Dismissed candidates no longer show up in the review list.
    let (_, list, _) = request_json(
        app,
        "GET",
        "/api/memory/promotion-candidates",
        Some(&cookie),
    )
    .await;
    assert!(list.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn session_summary_is_a_safe_noop_with_llm_disabled() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) =
        request_json(app, "POST", "/api/memory/session-summary", Some(&cookie)).await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["summarized"], false);
}

#[tokio::test]
async fn promotion_routes_require_authentication() {
    let Some((app, _store, _dir)) = state() else {
        return;
    };
    let (status, _, _) = request_json(app, "GET", "/api/memory/promotion-candidates", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

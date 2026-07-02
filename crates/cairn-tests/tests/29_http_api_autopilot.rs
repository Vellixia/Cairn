//! 29 — cairn-api autopilot endpoints (v0.8.0 Sprint 8): `/api/memory/promotion-log`,
//! `/api/memory/:id/demote`, `/api/guard/anchor/auto`, `/api/memory/autopilot-digest`, and
//! drift autopilot's effect on `/api/guard/verify`.
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

fn state(drift_autopilot: &str) -> Option<(axum::Router, Arc<Store>, tempfile::TempDir)> {
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
        drift_autopilot: drift_autopilot.to_string(),
        drift_safe_globs: vec!["docs/**".to_string(), "*.md".to_string()],
        auto_anchor: true,
    };
    let state = AppState::with_store(&cfg, store.clone()).ok()?;
    Some((router(state), store, dir))
}

fn seed_project_memory(store: &Store, content: &str, promo_score: f32) -> Memory {
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
    store.insert_memory(&m).expect("seed memory");
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
    read_body(resp).await
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

// --- promotion-log / demote ---

#[tokio::test]
async fn promotion_log_lists_promote_then_demote() {
    let Some((app, store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let m = seed_project_memory(&store, "promotable fact", 0.90);

    let (status, body, _) = request_json(
        app.clone(),
        "POST",
        &format!("/api/memory/{}/promote", m.id),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");

    let (status, log, _) =
        request_json(app.clone(), "GET", "/api/memory/promotion-log", Some(&cookie)).await;
    assert!(status.is_success());
    let entries = log.as_array().expect("array");
    assert!(entries.iter().any(|e| e["memory_id"] == m.id && e["action"] == "promote"));

    let (status, body, _) = request_json(
        app.clone(),
        "POST",
        &format!("/api/memory/{}/demote", m.id),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "demote failed: {status} body={body}");
    assert_eq!(body["scope_type"], "project");
    assert_eq!(body["scope_id"], "proj-alpha");

    let (_, log, _) = request_json(app, "GET", "/api/memory/promotion-log", Some(&cookie)).await;
    let entries = log.as_array().expect("array");
    assert!(entries.iter().any(|e| e["memory_id"] == m.id && e["action"] == "demote"));
}

#[tokio::test]
async fn demote_unknown_id_returns_404() {
    let Some((app, _store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, _, _) =
        request_json(app, "POST", "/api/memory/does-not-exist/demote", Some(&cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn demote_never_promoted_returns_404() {
    let Some((app, store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let m = seed_project_memory(&store, "never promoted", 0.5);
    let (status, _, _) = request_json(
        app,
        "POST",
        &format!("/api/memory/{}/demote", m.id),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- auto-anchor ---

#[tokio::test]
async fn auto_anchor_derives_from_first_prompt() {
    let Some((app, _store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = post_json(
        app.clone(),
        "/api/guard/anchor/auto",
        serde_json::json!({ "prompt": "Fix the login bug. Then write tests." }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["derived"], true);
    assert_eq!(body["anchor"], "Fix the login bug");

    let (_, anchor, _) = request_json(app, "GET", "/api/guard/anchor", Some(&cookie)).await;
    assert_eq!(anchor["anchor"], "Fix the login bug");
}

#[tokio::test]
async fn auto_anchor_never_overrides_an_existing_anchor() {
    let Some((app, _store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, _, _) = post_json(
        app.clone(),
        "/api/guard/anchor",
        serde_json::json!({ "goal": "manually set goal" }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());

    let (status, body, _) = post_json(
        app.clone(),
        "/api/guard/anchor/auto",
        serde_json::json!({ "prompt": "a totally different prompt" }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["derived"], false);
    assert_eq!(body["anchor"], "manually set goal");
}

// --- autopilot digest ---

#[tokio::test]
async fn autopilot_digest_is_zero_with_no_activity() {
    let Some((app, _store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, body, _) =
        request_json(app, "GET", "/api/memory/autopilot-digest", Some(&cookie)).await;
    assert!(status.is_success());
    assert_eq!(body["promoted"], 0);
    assert_eq!(body["demoted"], 0);
    assert_eq!(body["drift_auto_approved"], 0);
}

#[tokio::test]
async fn autopilot_digest_counts_a_promotion() {
    let Some((app, store, _dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let m = seed_project_memory(&store, "digest candidate", 0.9);
    let (status, _, _) = request_json(
        app.clone(),
        "POST",
        &format!("/api/memory/{}/promote", m.id),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());

    let (_, body, _) =
        request_json(app, "GET", "/api/memory/autopilot-digest", Some(&cookie)).await;
    assert_eq!(body["promoted"], 1);
}

// --- drift autopilot (via /api/guard/verify) ---

/// A `warn`-risk edit per `cairn-guard`'s diff-size classifier: baseline 10 lines, remove
/// exactly 2 (20% - `removed_ratio >= 0.2` but well under the `>= 0.5` danger threshold).
fn warn_risk_new_content() -> (String, String) {
    let baseline = (1..=10).map(|i| format!("line {i}\n")).collect::<String>();
    let new_content = (1..=8).map(|i| format!("line {i}\n")).collect::<String>();
    (baseline, new_content)
}

#[tokio::test]
async fn safe_mode_auto_approves_warn_under_a_safe_glob() {
    let Some((app, _store, dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let target = dir.path().join("docs").join("guide.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let (baseline, new_content) = warn_risk_new_content();
    std::fs::write(&target, &baseline).unwrap();

    let (status, vj, _) = post_json(
        app.clone(),
        "/api/guard/verify",
        serde_json::json!({ "path": target.to_string_lossy(), "content": new_content }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(vj["risk"], "warn", "must be warn risk: {vj}");

    let (_, drift, _) = request_json(app, "GET", "/api/guard/drift?limit=10", Some(&cookie)).await;
    let events = drift.as_array().expect("array");
    assert_eq!(
        events[0]["status"], "approved",
        "a warn edit under docs/** must auto-approve in safe mode: {events:?}"
    );
}

#[tokio::test]
async fn safe_mode_holds_warn_outside_safe_globs() {
    let Some((app, _store, dir)) = state("safe") else { return };
    let cookie = login_cookie(app.clone()).await;
    let target = dir.path().join("crates").join("lib.rs");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let (baseline, new_content) = warn_risk_new_content();
    std::fs::write(&target, &baseline).unwrap();

    let (status, vj, _) = post_json(
        app.clone(),
        "/api/guard/verify",
        serde_json::json!({ "path": target.to_string_lossy(), "content": new_content }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(vj["risk"], "warn");

    let (_, drift, _) = request_json(app, "GET", "/api/guard/drift?limit=10", Some(&cookie)).await;
    let events = drift.as_array().expect("array");
    assert_eq!(
        events[0]["status"], "pending",
        "a warn edit outside the safe globs must still hold: {events:?}"
    );
}

#[tokio::test]
async fn danger_always_holds_even_in_all_mode() {
    let Some((app, _store, dir)) = state("all") else { return };
    let cookie = login_cookie(app.clone()).await;
    let target = dir.path().join("docs").join("wipe.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let baseline = (1..=10)
        .map(|i| format!("line {i}: important\n"))
        .collect::<String>();
    std::fs::write(&target, &baseline).unwrap();
    let new_content = "# only one line remains\n".to_string();

    let (status, vj, _) = post_json(
        app.clone(),
        "/api/guard/verify",
        serde_json::json!({ "path": target.to_string_lossy(), "content": new_content }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(vj["risk"], "danger");

    let (_, drift, _) = request_json(app, "GET", "/api/guard/drift?limit=10", Some(&cookie)).await;
    let events = drift.as_array().expect("array");
    assert_eq!(
        events[0]["status"], "pending",
        "danger must hold even under CAIRN_DRIFT_AUTOPILOT=all: {events:?}"
    );
}

#[tokio::test]
async fn off_mode_holds_everything() {
    let Some((app, _store, dir)) = state("off") else { return };
    let cookie = login_cookie(app.clone()).await;
    let target = dir.path().join("docs").join("guide.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let (baseline, new_content) = warn_risk_new_content();
    std::fs::write(&target, &baseline).unwrap();

    let (status, _, _) = post_json(
        app.clone(),
        "/api/guard/verify",
        serde_json::json!({ "path": target.to_string_lossy(), "content": new_content }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());

    let (_, drift, _) = request_json(app, "GET", "/api/guard/drift?limit=10", Some(&cookie)).await;
    let events = drift.as_array().expect("array");
    assert_eq!(
        events[0]["status"], "pending",
        "CAIRN_DRIFT_AUTOPILOT=off must reproduce fully-manual behavior: {events:?}"
    );
}

#[tokio::test]
async fn autopilot_routes_require_authentication() {
    let Some((app, _store, _dir)) = state("safe") else { return };
    let (status, _, _) =
        request_json(app, "GET", "/api/memory/promotion-log", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

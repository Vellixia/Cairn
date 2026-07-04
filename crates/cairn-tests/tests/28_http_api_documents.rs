//! 28 — cairn-api `/api/documents/*` HTTP routes (v0.8.0 Sprint 6 - RAG document store),
//! mounted in-process via tower::oneshot.
//!
//! The hermetic in-memory backend has no HNSW vector index, so `search` exercises
//! `memory_backend::search_documents`'s lexical-substring fallback rather than real semantic
//! search - see `crates/cairn-store/src/surreal.rs::live::document_chunks_roundtrip_via_store`
//! for the real embedding-backed search path against a live SurrealDB.
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
    let state = AppState::with_store(&cfg, store).ok()?;
    Some((router(state), dir))
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

async fn ingest(app: axum::Router, source: &str, content: &str, cookie: &str) -> serde_json::Value {
    let (status, body, _) = post_json(
        app,
        "/api/documents/ingest",
        serde_json::json!({ "source": source, "content": content }),
        Some(cookie),
    )
    .await;
    assert!(status.is_success(), "ingest failed: {status} body={body}");
    body
}

/// Like `post_json` but sets `X-Cairn-Project` so the scope middleware picks it up - used to
/// exercise document project-scoping (web redesign v2 follow-up).
async fn post_json_scoped(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
    cookie: &str,
    project_id: &str,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("cookie", format!("cairn_session={cookie}"))
        .header("X-Cairn-Project", project_id)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

/// Like `request_json` but sets `X-Cairn-Project`.
async fn request_json_scoped(
    app: axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    project_id: &str,
) -> (StatusCode, serde_json::Value, Vec<axum::http::HeaderValue>) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", format!("cairn_session={cookie}"))
        .header("X-Cairn-Project", project_id)
        .body(Body::empty())
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

#[tokio::test]
async fn ingest_then_list_shows_the_document() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let summary = ingest(
        app.clone(),
        "docs/readme.md",
        "# Title\n\nCairn gives agents persistent memory across sessions.",
        &cookie,
    )
    .await;
    assert_eq!(summary["source"], "docs/readme.md");
    assert_eq!(summary["title"], "docs/readme.md", "defaults to source");
    assert!(summary["chunk_count"].as_u64().unwrap() >= 1);
    assert!(summary["id"].as_str().is_some());

    let (status, list, _) = request_json(app, "GET", "/api/documents", Some(&cookie)).await;
    assert!(status.is_success());
    let docs = list.as_array().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["source"], "docs/readme.md");
}

#[tokio::test]
async fn ingest_with_explicit_title_is_respected() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, body, _) = post_json(
        app,
        "/api/documents/ingest",
        serde_json::json!({
            "source": "docs/readme.md",
            "content": "some content here",
            "title": "My Readme",
        }),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["title"], "My Readme");
}

#[tokio::test]
async fn ingest_empty_content_returns_400() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, _, _) = post_json(
        app,
        "/api/documents/ingest",
        serde_json::json!({ "source": "empty.md", "content": "   " }),
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn re_ingesting_the_same_source_replaces_not_appends() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let first = ingest(app.clone(), "docs/x.md", "one two three four five", &cookie).await;
    let first_count = first["chunk_count"].as_u64().unwrap();
    assert!(first_count >= 1);

    let second = ingest(app.clone(), "docs/x.md", "just one short line now", &cookie).await;
    assert_eq!(
        second["chunk_count"].as_u64().unwrap(),
        1,
        "short content should be a single chunk"
    );

    let (_, list, _) = request_json(app, "GET", "/api/documents", Some(&cookie)).await;
    let docs = list.as_array().unwrap();
    assert_eq!(docs.len(), 1, "re-ingest must not create a second document");
}

#[tokio::test]
async fn search_finds_a_chunk_by_keyword() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    ingest(
        app.clone(),
        "docs/zephyrium.md",
        "the zephyrium engine is written in rust and uses tokio for async io.",
        &cookie,
    )
    .await;
    ingest(
        app.clone(),
        "docs/unrelated.md",
        "bananas are a good source of potassium.",
        &cookie,
    )
    .await;

    let (status, hits, _) = request_json(
        app,
        "GET",
        "/api/documents/search?q=zephyrium%20tokio",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    let hits = hits.as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|h| h["source"] == "docs/zephyrium.md"));
}

#[tokio::test]
async fn delete_removes_the_document() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    let summary = ingest(
        app.clone(),
        "docs/to-delete.md",
        "content to delete",
        &cookie,
    )
    .await;
    let id = summary["id"].as_str().unwrap();

    let (status, body, _) = request_json(
        app.clone(),
        "DELETE",
        &format!("/api/documents/{id}"),
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["deleted"], true);

    let (_, list, _) = request_json(app, "GET", "/api/documents", Some(&cookie)).await;
    assert!(list.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delete_unknown_id_returns_404() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, _, _) = request_json(
        app,
        "DELETE",
        "/api/documents/does-not-exist",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Web redesign v2 follow-up: a document ingested under one project's `X-Cairn-Project` header
/// is invisible when listing a DIFFERENT project, but still shows up in the unfiltered
/// (no-header) global list - same blend policy memory scoping already uses.
#[tokio::test]
async fn project_scoped_documents_are_isolated_but_visible_globally() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = post_json_scoped(
        app.clone(),
        "/api/documents/ingest",
        serde_json::json!({ "source": "proj-a/notes.md", "content": "alpha project notes" }),
        &cookie,
        "proj-a",
    )
    .await;
    assert!(status.is_success(), "ingest failed: {status} body={body}");
    assert_eq!(body["project_id"], "proj-a");

    // Different project: proj-a's doc must not appear.
    let (_, list_b, _) =
        request_json_scoped(app.clone(), "GET", "/api/documents", &cookie, "proj-b").await;
    assert!(
        !list_b
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["source"] == "proj-a/notes.md"),
        "proj-b must not see proj-a's document; got {list_b}"
    );

    // Same project: visible.
    let (_, list_a, _) =
        request_json_scoped(app.clone(), "GET", "/api/documents", &cookie, "proj-a").await;
    assert!(list_a
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["source"] == "proj-a/notes.md"));

    // No header (global, unfiltered): still visible.
    let (_, list_all, _) = request_json(app, "GET", "/api/documents", Some(&cookie)).await;
    assert!(list_all
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["source"] == "proj-a/notes.md"));
}

/// Same isolation guarantee, exercised through `/api/documents/search` instead of `list`.
#[tokio::test]
async fn project_scoped_search_excludes_other_projects_documents() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    post_json_scoped(
        app.clone(),
        "/api/documents/ingest",
        serde_json::json!({ "source": "proj-a/guide.md", "content": "zephyrium alpha guide content" }),
        &cookie,
        "proj-a",
    )
    .await;
    post_json_scoped(
        app.clone(),
        "/api/documents/ingest",
        serde_json::json!({ "source": "proj-b/guide.md", "content": "zephyrium beta guide content" }),
        &cookie,
        "proj-b",
    )
    .await;

    let (status, hits, _) = request_json_scoped(
        app,
        "GET",
        "/api/documents/search?q=zephyrium%20guide",
        &cookie,
        "proj-a",
    )
    .await;
    assert!(status.is_success());
    let hits = hits.as_array().unwrap();
    assert!(hits.iter().any(|h| h["source"] == "proj-a/guide.md"));
    assert!(
        !hits.iter().any(|h| h["source"] == "proj-b/guide.md"),
        "proj-a search must not surface proj-b's document; got {hits:?}"
    );
}

#[tokio::test]
async fn document_routes_require_authentication() {
    let Some((app, _dir)) = state() else { return };
    let (status, _, _) = request_json(app, "GET", "/api/documents", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

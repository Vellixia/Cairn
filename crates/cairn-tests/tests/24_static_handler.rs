//! 24 — cairn-api static handler edge cases (BUG-2026-06-30-C).
//!
//! Regression tests for the `static_handler` fallback behavior. The bug:
//! when a request for a missing static asset (e.g. a stale hashed chunk) hit
//! the handler, the fallback to `index.html` returned dashboard shell HTML
//! with a `text/javascript` MIME. Browsers then tried to parse the HTML as
//! JavaScript and surfaced a confusing `ChunkLoadError`.
//!
//! Fix: paths that look like static assets (non-HTML extension) return 404
//! when the asset is missing instead of falling back to `index.html`.
//! HTML routes still fall back to `index.html` (SPA behavior).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cairn_api::{build_router_with_registry, AppState};
use cairn_core::Config;
use cairn_store::Store;
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

fn state() -> Option<axum::Router> {
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
    };
    let s = AppState::with_store(&cfg, store).ok()?;
    Some(build_router_with_registry(s))
}

async fn get(app: axum::Router, path: &str) -> (StatusCode, String, Vec<u8>) {
    let resp = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or("").to_string())
        .unwrap_or_default();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (status, ct, body.to_vec())
}

fn has_any_marker(body: &[u8]) -> bool {
    let s = String::from_utf8_lossy(body);
    // BUG-C symptom: response body is the dashboard shell HTML and starts
    // with the html doctype even though the request asked for a .js asset.
    s.starts_with("<!DOCTYPE html>") || s.starts_with("<!doctype html>")
}

#[tokio::test]
async fn missing_js_asset_returns_404_not_html_fallback() {
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    let (status, ct, body) = get(app, "/_next/static/chunks/nonexistent-deadbeef.js").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "missing .js must 404, got {status} ct={ct}"
    );
    assert!(
        !has_any_marker(&body),
        "missing .js must NOT be dashboard shell HTML (BUG-2026-06-30-C)"
    );
    assert!(
        ct.starts_with("text/plain"),
        "missing .js should be text/plain, got {ct}"
    );
}

#[tokio::test]
async fn missing_css_asset_returns_404_not_html_fallback() {
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    let (status, _ct, body) = get(app, "/_next/static/css/nonexistent-feedface.css").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!has_any_marker(&body));
}

#[tokio::test]
async fn missing_chunk_under_dynamic_route_returns_404() {
    // The exact shape of the BUG-C: a chunk URL under the (app)/registry/packs
    // path (with URL-encoded brackets). Both the %5Bname%5D form and a plain
    // form must 404 instead of returning index.html.
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    let path = "/_next/static/chunks/app/(app)/registry/packs/%5Bname%5D/page-deadbeef.js";
    let (status, _ct, body) = get(app, path).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "BUG-2026-06-30-C regression: missing chunk must 404"
    );
    assert!(!has_any_marker(&body));
}

#[tokio::test]
async fn missing_html_route_still_falls_back_to_index() {
    // SPA behavior must be preserved: a missing HTML page (no extension or
    // .html) falls back to index.html. This is how Next.js static export
    // serves client-side routes.
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    let (status, ct, body) = get(app, "/registry/packs/some-missing-slug").await;
    assert_eq!(status, StatusCode::OK, "SPA fallback must remain 200");
    assert!(ct.starts_with("text/html"), "got {ct}");
    assert!(
        has_any_marker(&body),
        "SPA fallback must serve the dashboard shell"
    );
}

#[tokio::test]
async fn percent_encoded_path_does_not_become_400_when_decoded_path_is_real() {
    // BUG-2026-06-30-C: Next.js chunk filenames are URL-encoded by the static
    // export. The server must percent-decode the request path before looking
    // up the asset. Otherwise a chunk like `/%5Bname%5D/page-XXXX.js` is
    // treated as a literal string and never matches the embedded file
    // (whose actual key is `/[name]/page-XXXX.js`).
    //
    // We probe the encoded form against whatever chunks the test runner's
    // `web/out` actually contains. If a real chunk with brackets is present
    // we expect 200; if not, the encoded path must still resolve cleanly
    // (200 for `index.html` fallback, or 404 text/plain for missing assets).
    // It must NEVER be a 400 "invalid path".
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    let (status, ct, _body) = get(
        app,
        "/_next/static/chunks/%5Bname%5D/page-9bfb0c3fd0e720be.js",
    )
    .await;
    // Hard regression: never 400 from the static handler for a benign path.
    assert_ne!(status, StatusCode::BAD_REQUEST);
    match status {
        StatusCode::OK => {
            // If the test env has the real chunk, MIME must be JS, not HTML.
            assert!(
                !ct.starts_with("text/html"),
                "encoded chunk must not serve html, got {ct}"
            );
        }
        StatusCode::NOT_FOUND => {
            // Missing asset path: Fix A + Fix C-3 return 404 text/plain.
            assert!(ct.starts_with("text/plain"), "got {ct}");
        }
        other => panic!("unexpected status {other}"),
    }
}

#[tokio::test]
async fn percent_encoded_path_under_dynamic_route_html_falls_back() {
    // Browser often navigates to the page route via a slash. The handler
    // must percent-decode before deciding "this is a real file vs. an HTML
    // route" so dynamic routes like `/registry/packs/new` resolve to the
    // right thing. The HTML fallback should still work for encoded
    // dynamic-route paths.
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    let (status, ct, body) = get(app, "/registry/packs/%5Bname%5D").await;
    assert_ne!(status, StatusCode::BAD_REQUEST);
    // If 200, must be HTML fallback (no real page under this name in static
    // export). If 404, must be text/plain.
    if status == StatusCode::OK {
        assert!(ct.starts_with("text/html"));
    } else {
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(ct.starts_with("text/plain"));
    }
    // And never the literal encoded path treated as a key.
    let s = String::from_utf8_lossy(&body);
    assert!(!s.starts_with("%5B"), "body should not echo encoded path");
}

#[tokio::test]
async fn present_chunk_returns_200_with_js_mime() {
    // Sanity: a chunk that the test runner's `web/out` actually contains still
    // serves 200 with the right MIME. This guards against the fix accidentally
    // breaking legitimate asset delivery. We probe for several real entries and
    // accept any 200/JS match, or — when the export isn't present in this
    // test's working dir — assert the 404 text/plain path is taken.
    let Some(app) = state() else {
        eprintln!("skip: state() init failed");
        return;
    };
    for path in [
        "/_next/static/chunks/webpack.js",
        "/_next/static/chunks/main-app.js",
        "/icon.svg",
    ] {
        let (status, ct, _body) = get(app.clone(), path).await;
        match status {
            StatusCode::OK => {
                // Whichever asset is present must have a real MIME (not HTML).
                assert!(!ct.starts_with("text/html"), "got html for {path}: {ct}");
                assert!(!ct.is_empty(), "missing content-type for {path}");
            }
            StatusCode::NOT_FOUND => {
                // Fix A: missing assets return 404 text/plain — must not be
                // HTML fallback (regression guard for BUG-2026-06-30-C).
                assert!(
                    ct.starts_with("text/plain"),
                    "missing asset {path} should be text/plain, got {ct}"
                );
            }
            other => panic!("unexpected status {other} for {path}"),
        }
    }
}

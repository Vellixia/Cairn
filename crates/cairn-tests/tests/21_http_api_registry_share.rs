//! 21 — cairn-api registry (`/api/registry/*`) + share (`/api/share/*`) HTTP
//! routes, mounted in-process via tower::oneshot.
//!
//! Exercises the public surface for:
//!  - `POST /api/registry/packs` - publish a tarball
//!  - `GET  /api/registry/packs` - list published packs
//!  - `GET  /api/registry/packs/:name` - list versions for a name
//!  - `GET  /api/registry/packs/:name/:version/download` - download the tarball
//!  - `GET  /api/share/export` - export a sanitized bundle
//!  - `POST /api/share/import` - ingest a bundle as `shared`-tagged memories
//!
//! Hermetic: in-memory `cairn_store::Store`, real `cairn_api::build_router_with_registry`
//! (so `/api/registry/*` is mounted), real `cairn_registry::Registry::open` against a tempdir.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cairn_api::{build_router_with_registry, AppState};
use cairn_core::{Config, MemoryKind};
use cairn_pack::Pack;
use cairn_share::{Sensitivity, ShareableMemory};
use cairn_store::Store;
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

fn state() -> Option<(axum::Router, tempfile::TempDir)> {
    let dir = tempfile::tempdir().ok()?;
    let blobs = dir.path().join("blobs");
    let store = Arc::new(Store::open_in_memory(blobs).ok()?);
    // Registry stores under <data_dir>/registry/ via Registry::open.
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
    // with_store auto-opens the registry from cfg.data_dir/registry/, so
    // build_router_with_registry gets a real registry mounted at /api/registry.
    let state = AppState::with_store(&cfg, store).ok()?;
    Some((build_router_with_registry(state), dir))
}

async fn read_body(
    resp: axum::response::Response,
) -> (
    StatusCode,
    serde_json::Value,
    Vec<u8>,
    Vec<axum::http::HeaderValue>,
) {
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
    let bytes: Vec<u8> = body.to_vec();
    let json: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json, bytes, headers)
}

async fn post_json(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
    cookie: Option<&str>,
) -> (
    StatusCode,
    serde_json::Value,
    Vec<u8>,
    Vec<axum::http::HeaderValue>,
) {
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

async fn post_bytes(
    app: axum::Router,
    path: &str,
    bytes: Vec<u8>,
    cookie: Option<&str>,
    content_type: &str,
) -> (
    StatusCode,
    serde_json::Value,
    Vec<u8>,
    Vec<axum::http::HeaderValue>,
) {
    let mut b = Request::builder().method("POST").uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = b
        .header("content-type", content_type)
        .body(Body::from(bytes))
        .expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

async fn get_json(
    app: axum::Router,
    path: &str,
    cookie: Option<&str>,
) -> (
    StatusCode,
    serde_json::Value,
    Vec<u8>,
    Vec<axum::http::HeaderValue>,
) {
    let mut b = Request::builder().method("GET").uri(path);
    if let Some(c) = cookie {
        b = b.header("cookie", format!("cairn_session={c}"));
    }
    let req = b.body(Body::empty()).expect("build request");
    let resp = app.oneshot(req).await.expect("oneshot");
    read_body(resp).await
}

async fn login_cookie(app: axum::Router) -> String {
    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecret-admin-pass",
    });
    let (status, _json, _b, _h) = post_json(app.clone(), "/api/auth/setup", body, None).await;
    assert!(
        status.is_success() || status == StatusCode::CONFLICT,
        "setup must succeed or already-exist; got {status}"
    );
    let (lstatus, ljson, _b, lheaders) = post_json(
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
    raw.split(';')
        .next()
        .expect("cookie has value part")
        .trim_start_matches("cairn_session=")
        .to_string()
}

/// Build a real `.cairnpkg` tarball in a temp file and return the raw bytes
/// for posting to the registry publish endpoint.
fn build_tarball(
    name: &str,
    version: &str,
    author: &str,
    memories: Vec<ShareableMemory>,
) -> Vec<u8> {
    // Pack::memories is `Vec<serde_json::Value>` (not ShareableMemory directly) -
    // convert via serde so the manifest content is identical to what the runtime
    // sees after sanitization.
    let mem_values: Vec<serde_json::Value> = memories
        .into_iter()
        .map(|m| serde_json::to_value(m).expect("serialize shareable memory"))
        .collect();
    let mut pack = Pack::new(name, version);
    pack.author = author.to_string();
    pack.description = format!("test pack {name}@{version}");
    pack.memories = mem_values;
    let tmp = tempfile::tempdir().expect("tempdir for tarball");
    let path = tmp.path().join("pack.cairnpkg");
    pack.write_tarball(&path).expect("write tarball");
    std::fs::read(&path).expect("read tarball back")
}

#[tokio::test]
async fn publish_then_list_pack_via_http() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let mems = vec![ShareableMemory {
        kind: MemoryKind::Note,
        content: "registry test memory: auth pattern".to_string(),
        concepts: vec!["registry".to_string(), "test".to_string()],
        sensitivity: Sensitivity::Shareable,
        redactions: 0,
    }];
    let tarball = build_tarball("cairn-registry-test", "0.1.0", "test-author", mems);

    let (ps, pj, _pb, _h) = post_bytes(
        app.clone(),
        "/api/registry/packs",
        tarball,
        Some(&cookie),
        "application/octet-stream",
    )
    .await;
    assert!(ps.is_success(), "publish must succeed; got {ps} body={pj}");
    // The publish handler returns a PublishReceipt-shaped JSON object.
    assert_eq!(pj["name"], "cairn-registry-test");
    assert_eq!(pj["version"], "0.1.0");

    // List must include the pack we just published.
    let (ls, lj, _b, _h) = get_json(app, "/api/registry/packs", Some(&cookie)).await;
    assert!(
        ls.is_success(),
        "list packs must succeed; got {ls} body={lj}"
    );
    let packs = lj.as_array().expect("array");
    let names: Vec<&str> = packs
        .iter()
        .map(|p| p["name"].as_str().unwrap_or(""))
        .collect();
    assert!(
        names.contains(&"cairn-registry-test"),
        "published pack must appear in list; got {names:?}"
    );
}

#[tokio::test]
async fn list_versions_returns_published_version() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let mems = vec![ShareableMemory {
        kind: MemoryKind::Note,
        content: "versioning test memory".to_string(),
        concepts: vec![],
        sensitivity: Sensitivity::Shareable,
        redactions: 0,
    }];

    // Publish two versions of the same pack.
    let v1 = build_tarball("cairn-versioning", "1.0.0", "test-author", mems.clone());
    let (s1, j1, _b, _h) = post_bytes(
        app.clone(),
        "/api/registry/packs",
        v1,
        Some(&cookie),
        "application/octet-stream",
    )
    .await;
    assert!(
        s1.is_success(),
        "publish v1.0.0 must succeed; got {s1} body={j1}"
    );

    let v2 = build_tarball("cairn-versioning", "1.0.1", "test-author", mems);
    let (s2, j2, _b, _h) = post_bytes(
        app.clone(),
        "/api/registry/packs",
        v2,
        Some(&cookie),
        "application/octet-stream",
    )
    .await;
    assert!(
        s2.is_success(),
        "publish v1.0.1 must succeed; got {s2} body={j2}"
    );

    // List versions for the name.
    let (ls, lj, _b, _h) =
        get_json(app, "/api/registry/packs/cairn-versioning", Some(&cookie)).await;
    assert!(
        ls.is_success(),
        "list versions must succeed; got {ls} body={lj}"
    );
    let versions = lj.as_array().expect("array");
    let vstrs: Vec<&str> = versions
        .iter()
        .map(|v| v["version"].as_str().unwrap_or(""))
        .collect();
    assert!(vstrs.contains(&"1.0.0"), "missing 1.0.0 in {vstrs:?}");
    assert!(vstrs.contains(&"1.0.1"), "missing 1.0.1 in {vstrs:?}");
}

#[tokio::test]
async fn download_pack_returns_raw_tarball_bytes() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let mems = vec![ShareableMemory {
        kind: MemoryKind::Note,
        content: "download roundtrip memory".to_string(),
        concepts: vec!["download".to_string()],
        sensitivity: Sensitivity::Shareable,
        redactions: 0,
    }];
    let tarball = build_tarball("cairn-download", "0.1.0", "test-author", mems);
    let original_len = tarball.len();
    let (ps, pj, _pb, _h) = post_bytes(
        app.clone(),
        "/api/registry/packs",
        tarball,
        Some(&cookie),
        "application/octet-stream",
    )
    .await;
    assert!(ps.is_success(), "publish must succeed; got {ps} body={pj}");

    let (ds, _dj, db, _h) = get_json(
        app,
        "/api/registry/packs/cairn-download/0.1.0/download",
        Some(&cookie),
    )
    .await;
    assert!(ds.is_success(), "download must succeed; got {ds}");
    assert!(!db.is_empty(), "downloaded body must be non-empty");
    assert!(
        db.len() >= original_len / 2,
        "downloaded tarball is suspiciously small: {} bytes (original {})",
        db.len(),
        original_len
    );
    // The pack format is a tar archive - confirm by looking for the tar magic
    // at offset 257 ("ustar") or by checking the manifest filename appears.
    let text = String::from_utf8_lossy(&db);
    assert!(
        text.contains("manifest.json"),
        "tarball body must contain manifest.json; got first 200 bytes: {}",
        text.chars().take(200).collect::<String>()
    );
}

#[tokio::test]
async fn search_packs_finds_published_pack_by_name_substring_and_misses_on_garbage() {
    // REGRESSION TEST for BUG 10-2: `GET /api/registry/search?q=...` was
    // called from the dashboard at `web/src/app/(app)/registry/packs/PacksContent.tsx:186`
    // with the wrong path (`/registry/search?q=...` — missing `/api` prefix).
    // cairn-api treated the path as a static-asset slug and the Next.js
    // static export returned 200 HTML, which the JSON parser rejected.
    //
    // This test pins the *correct* behaviour end-to-end: a substring of a
    // real pack name must be found, and a query that doesn't match any
    // pack must return an empty array (not 404, not HTML). When the prefix
    // drift regresses, the matcher will hit the wrong path and the GET
    // helper will surface the response as a non-success status.
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let mems = vec![ShareableMemory {
        kind: MemoryKind::Note,
        content: "search regression memory".to_string(),
        concepts: vec![],
        sensitivity: Sensitivity::Shareable,
        redactions: 0,
    }];
    let tarball = build_tarball("cairn-search-target-xyz", "0.1.0", "test-author", mems);
    let (ps, pj, _b, _h) = post_bytes(
        app.clone(),
        "/api/registry/packs",
        tarball,
        Some(&cookie),
        "application/octet-stream",
    )
    .await;
    assert!(ps.is_success(), "publish must succeed; got {ps} body={pj}");

    // 1. Real substring → must find the pack.
    let (ss, sj, _b, _h) = get_json(
        app.clone(),
        "/api/registry/search?q=cairn-search-target",
        Some(&cookie),
    )
    .await;
    assert!(
        ss.is_success(),
        "search for real substring must succeed; got {ss} body={sj}"
    );
    let results = sj.as_array().expect("search must return a JSON array");
    let names: Vec<&str> = results
        .iter()
        .map(|r| r["name"].as_str().unwrap_or(""))
        .collect();
    assert!(
        names.contains(&"cairn-search-target-xyz"),
        "search must find the published pack; got names={names:?}, body={sj}"
    );

    // 2. Garbage query → empty array (not 404, not HTML). Distinguishes the
    //    API endpoint from the old broken `/registry/search` static-asset
    //    path that used to masquerade as JSON.
    let (gs, gj, gb, _h) = get_json(
        app.clone(),
        "/api/registry/search?q=__no_such_pack_xyz_42__",
        Some(&cookie),
    )
    .await;
    assert!(
        gs.is_success(),
        "search for garbage must return 200 + []; got {gs} body={gj}"
    );
    assert!(
        !gb.is_empty(),
        "search response body must be non-empty; got {} bytes",
        gb.len()
    );
    let first = gb[0];
    assert!(
        first == b'[' || first == b'n',
        "search response must be a JSON array or null; got first byte {first:?}"
    );
    let results = gj.as_array().expect("search must return a JSON array");
    assert!(
        results.is_empty(),
        "garbage search must return []; got {results:?}"
    );
}

#[tokio::test]
async fn share_export_returns_sanitized_bundle_envelope() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // Seed a memory that contains a hardcoded secret. The sanitizer should
    // redact it AND withhold the memory entirely (Sensitivity::Private).
    let secret_body = serde_json::json!({
        "content": "api key: sk-test-SECRETKEY12345ABCDEFGHIJKLMNOPQR",
        "concepts": ["api"],
    });
    let (s, j, _b, _h) = post_json(app.clone(), "/api/memory", secret_body, Some(&cookie)).await;
    assert!(s.is_success(), "seed memory must succeed; got {s} body={j}");

    let (es, ej, _b, _h) = get_json(app, "/api/share/export", Some(&cookie)).await;
    assert!(es.is_success(), "export must succeed; got {es} body={ej}");
    // The export envelope wraps the ShareBundle with extra stats keys.
    assert_eq!(ej["schema"], "cairn-share-bundle");
    assert!(
        ej["version"].is_u64(),
        "version is a u32 (serialized as u64)"
    );
    assert!(ej["memories"].is_array(), "memories is an array");
    assert!(
        ej["withheld"].as_u64().unwrap_or(0) >= 1,
        "secret-bearing memory must be withheld from export; got {ej}"
    );
}

#[tokio::test]
async fn share_import_round_trips_sanitized_memories_into_store() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // First seed a memory so export has something to share.
    let (rs, rj, _b, _h) = post_json(
        app.clone(),
        "/api/memory",
        serde_json::json!({
            "content": "shareable: the test suite must round-trip through share/import",
            "concepts": ["share", "test"],
        }),
        Some(&cookie),
    )
    .await;
    assert!(rs.is_success(), "seed must succeed; got {rs} body={rj}");

    // Export first so the import is data-driven: we copy the first memory's
    // content into a fresh bundle that also carries a unique token we'll use
    // to recall it back. This avoids the trap of `Store::open_in_memory`
    // returning a fresh in-memory SQLite on every call.
    let (es, ej, _b, _h) = get_json(app.clone(), "/api/share/export", Some(&cookie)).await;
    assert!(es.is_success(), "export must succeed; got {es} body={ej}");
    assert!(
        !ej["memories"]
            .as_array()
            .expect("memories is array")
            .is_empty(),
        "seed memory must be exported; got {ej}"
    );

    // We can't trivially wipe the live store, so confirm the import is
    // real by recalling for content that exists in the bundle but is unique
    // enough to distinguish the imported copy from the seed. Recall is hybrid
    // search and returns scored memories; imported memories land in the store
    // with session_id="shared" (see cairn-share::ShareableMemory::into_new_memory).
    let bundle_content = ej["memories"][0]["content"]
        .as_str()
        .expect("bundle has content")
        .to_string();
    let unique_token = "import-token-zzzqq";
    let import_body = serde_json::json!({
        "schema": ej["schema"],
        "version": ej["version"],
        "memories": [{
            "kind": "note",
            "content": format!("{unique_token} {bundle_content}"),
            "concepts": ["imported", "roundtrip"],
            "sensitivity": "shareable",
            "redactions": 0,
        }],
    });

    let (is_, ij, _b, _h) =
        post_json(app.clone(), "/api/share/import", import_body, Some(&cookie)).await;
    assert!(is_.is_success(), "import must succeed; got {is_} body={ij}");
    let ingested = ij["ingested"].as_u64().expect("ingested is u64");
    assert!(
        ingested >= 1,
        "import must report >=1 ingested memory; got {ij}"
    );

    // Recall by the unique token; the imported memory must be in the live
    // store, distinct from the seed. Recall response wraps each hit in
    // `{"memory": {...}, "score": ...}`.
    let (rcs, rcj, _b, _h) = get_json(
        app,
        &format!("/api/memory/recall?q={unique_token}&limit=10"),
        Some(&cookie),
    )
    .await;
    assert!(
        rcs.is_success(),
        "recall must succeed; got {rcs} body={rcj}"
    );
    let hits = rcj.as_array().expect("recall body is array");
    let has_token = |hit: &serde_json::Value| -> bool {
        let content = hit
            .get("memory")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .or_else(|| hit.get("content").and_then(|c| c.as_str()));
        content.map(|c| c.contains(unique_token)).unwrap_or(false)
    };
    assert!(
        hits.iter().any(has_token),
        "imported memory with unique token must be recallable; got {rcj}"
    );
}

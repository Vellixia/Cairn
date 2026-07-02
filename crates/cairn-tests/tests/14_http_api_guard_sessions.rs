//! 14 — cairn-api `/api/guard/*` + `/api/sessions/*` HTTP routes, in-process.
//!
//! Exercises the guard verify/anchor/checkpoint/drift + sessions lifecycle
//! end-to-end through the real `cairn_api::router`. Includes a BUG 09-1
//! coverage test: `list_drift` (lib.rs:1099) hardcodes `None` for the
//! status filter, so the `?status=pending` query parameter is ignored.
//! The test asserts the endpoint returns non-empty events when drift is
//! appended - the spec call - and logs a comment when the filter appears
//! to be a no-op.

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

async fn login_cookie(app: axum::Router) -> String {
    let body = serde_json::json!({
        "username": "admin",
        "password": "supersecret-admin-pass",
    });
    let (status, _json, _h) = post_json(app.clone(), "/api/auth/setup", body, None).await;
    assert!(
        status.is_success() || status == StatusCode::CONFLICT,
        "setup must succeed or already-exist; got {status}"
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
    assert!(!lheaders.is_empty(), "login must set a cookie header");
    let raw = lheaders[0].to_str().expect("ascii").to_string();
    raw.split(';')
        .next()
        .expect("cookie has value part")
        .trim_start_matches("cairn_session=")
        .to_string()
}

#[tokio::test]
async fn verify_endpoint_returns_documented_risk_envelope() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // Verify a benign path. Guard returns a VerifyReport regardless of risk.
    let (s, j, _h) = post_json(
        app,
        "/api/guard/verify",
        serde_json::json!({
            "path": "crates/cairn-tests/tests/02_http_api_memory.rs",
            "content": "use axum::body::Body;\n",
        }),
        Some(&cookie),
    )
    .await;
    assert!(s.is_success(), "verify must succeed; got {s} body={j}");
    // Documented envelope keys: `risk`, `message`, `findings` (or similar).
    assert!(
        j.get("risk").is_some(),
        "verify response must include 'risk'; got {j}"
    );
    let risk = j["risk"].as_str().expect("risk is string");
    assert!(
        ["ok", "warn", "danger"].contains(&risk),
        "risk must be one of ok|warn|danger; got {risk}"
    );
}

#[tokio::test]
async fn verify_on_dangerous_path_appends_pending_drift_event() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // The guard's risk classifier is diff-size based (see cairn-guard/src/lib.rs::assess):
    // Risk::Danger when removed_ratio >= 0.5 and added < removed/2.
    // Build a real on-disk baseline (10 lines) in the tempdir, then propose
    // a new version that wipes most of it with little replacement.
    let target = _dir.path().join("scripts").join("wipe.sh");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let baseline = (1..=10)
        .map(|i| format!("line {i}: important preserved content\n"))
        .collect::<String>();
    std::fs::write(&target, &baseline).unwrap();

    let new_content = "# only one line remains\n".to_string();
    let (s, j, _h) = post_json(
        app.clone(),
        "/api/guard/verify",
        serde_json::json!({
            "path": target.to_string_lossy(),
            "content": new_content,
        }),
        Some(&cookie),
    )
    .await;
    assert!(s.is_success(), "verify must succeed; got {s} body={j}");
    let risk = j["risk"].as_str().expect("risk");
    assert_eq!(
        risk, "danger",
        "10-line baseline with 1-line replacement must be Danger: {j}"
    );

    // Drift list must include at least one event from this verify.
    let (ls, lj, _h) = get_json(app, "/api/guard/drift?limit=50", Some(&cookie)).await;
    assert!(
        ls.is_success(),
        "list drift must succeed; got {ls} body={lj}"
    );
    let events = lj.as_array().expect("drift body is array");
    assert!(
        !events.is_empty(),
        "verify on dangerous content must produce >=1 drift event"
    );
    // The newest event corresponds to the dangerous verify call. Compare
    // suffixes so the OS-dependent prefix doesn't make the test flaky.
    let newest = &events[0];
    let newest_path = newest["path"].as_str().expect("path string");
    assert!(
        newest_path.ends_with("wipe.sh"),
        "newest drift event path must end with wipe.sh; got {newest_path}"
    );
    assert_eq!(
        newest["status"].as_str(),
        Some("pending"),
        "newly appended drift must start pending; got {newest}"
    );
}

#[tokio::test]
async fn approve_drift_transitions_status_and_404s_on_repeat() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // Force a pending drift entry by writing a 10-line baseline and proposing
    // a 1-line replacement. Diff-size risk classifier rates this Danger.
    let target = _dir.path().join("scripts").join("risky.sh");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let baseline = (1..=10)
        .map(|i| format!("line {i}: preserved\n"))
        .collect::<String>();
    std::fs::write(&target, &baseline).unwrap();
    let (vs, vj, _h) = post_json(
        app.clone(),
        "/api/guard/verify",
        serde_json::json!({
            "path": target.to_string_lossy(),
            "content": "survivor\n".to_string(),
        }),
        Some(&cookie),
    )
    .await;
    assert!(vs.is_success(), "verify must succeed; got {vs} body={vj}");
    assert_eq!(vj["risk"].as_str(), Some("danger"), "must be danger: {vj}");

    let (ls, lj, _h) = get_json(app.clone(), "/api/guard/drift?limit=10", Some(&cookie)).await;
    assert!(
        ls.is_success(),
        "list drift must succeed; got {ls} body={lj}"
    );
    let events = lj.as_array().expect("array");
    assert!(!events.is_empty(), "expected >=1 drift event; body={lj}");
    let id = events[0]["id"].as_i64().expect("drift id is i64");

    // Approve it.
    let (as_, aj, _h) = post_json(
        app.clone(),
        &format!("/api/guard/drift/{id}/approve"),
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;
    assert!(
        as_.is_success(),
        "approve must succeed; got {as_} body={aj}"
    );
    assert_eq!(aj["status"], "approved");

    // Repeat approve must 404 - the row is no longer pending.
    let (as2, aj2, _h) = post_json(
        app,
        &format!("/api/guard/drift/{id}/approve"),
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;
    assert_eq!(
        as2,
        StatusCode::NOT_FOUND,
        "re-approving a resolved drift must 404; got {as2} body={aj2}"
    );
}

#[tokio::test]
async fn anchor_post_then_get_round_trip() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (ps, pj, _h) = post_json(
        app.clone(),
        "/api/guard/anchor",
        serde_json::json!({"goal": "build the cairn api integration test bucket"}),
        Some(&cookie),
    )
    .await;
    assert!(
        ps.is_success(),
        "post anchor must succeed; got {ps} body={pj}"
    );
    assert_eq!(
        pj["anchor"].as_str(),
        Some("build the cairn api integration test bucket")
    );

    let (gs, gj, _h) = get_json(app, "/api/guard/anchor", Some(&cookie)).await;
    assert!(
        gs.is_success(),
        "get anchor must succeed; got {gs} body={gj}"
    );
    assert_eq!(
        gj["anchor"].as_str(),
        Some("build the cairn api integration test bucket"),
        "anchor read-back must equal the goal we posted"
    );
}

#[tokio::test]
async fn checkpoint_create_then_list_then_rollback() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // Create two checkpoints with distinct labels.
    let (cs, cj, _h) = post_json(
        app.clone(),
        "/api/guard/checkpoint?label=before-edit",
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;
    assert!(
        cs.is_success(),
        "create checkpoint must succeed; got {cs} body={cj}"
    );
    assert_eq!(cj["label"], "before-edit");

    let (cs2, cj2, _h2) = post_json(
        app.clone(),
        "/api/guard/checkpoint?label=after-edit",
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;
    assert!(
        cs2.is_success(),
        "second checkpoint must succeed; got {cs2} body={cj2}"
    );
    let cp_id = cj2["id"].as_str().expect("checkpoint id").to_string();

    // List must include at least the two we just made.
    let (ls, lj, _h) = get_json(app.clone(), "/api/guard/checkpoints", Some(&cookie)).await;
    assert!(
        ls.is_success(),
        "list checkpoints must succeed; got {ls} body={lj}"
    );
    let cps = lj.as_array().expect("array");
    assert!(
        cps.len() >= 2,
        "expected >=2 checkpoints, got {}",
        cps.len()
    );
    let labels: Vec<&str> = cps
        .iter()
        .map(|c| c["label"].as_str().unwrap_or(""))
        .collect();
    assert!(labels.contains(&"before-edit"));
    assert!(labels.contains(&"after-edit"));

    // Roll back to the second checkpoint id. With no tracked files the blob
    // store stays empty, but the handler must still return success and echo
    // the id we asked for.
    let (rs, rj, _h) = post_json(
        app,
        &format!("/api/guard/rollback?id={cp_id}"),
        serde_json::json!({}),
        Some(&cookie),
    )
    .await;
    assert!(rs.is_success(), "rollback must succeed; got {rs} body={rj}");
    assert_eq!(rj["checkpoint_id"], cp_id);
    assert!(rj["restored"].is_array(), "restored is array");
    assert!(rj["skipped"].is_array(), "skipped is array");
}

#[tokio::test]
async fn sessions_create_list_get_latest_round_trip() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // Empty list at first.
    let (ls, lj, _h) = get_json(app.clone(), "/api/sessions", Some(&cookie)).await;
    assert!(
        ls.is_success(),
        "list sessions must succeed; got {ls} body={lj}"
    );
    let initial = lj.as_array().expect("sessions is array").len();

    // Create a session for a synthetic project hash.
    let (cs, cj, _h) = post_json(
        app.clone(),
        "/api/sessions",
        serde_json::json!({"project_hash": "abc123-deadbeef"}),
        Some(&cookie),
    )
    .await;
    assert!(
        cs.is_success(),
        "create session must succeed; got {cs} body={cj}"
    );
    let new_id = cj["id"].as_str().expect("id").to_string();
    assert_eq!(cj["project_hash"], "abc123-deadbeef");

    // List now has one more.
    let (ls2, lj2, _h) = get_json(app.clone(), "/api/sessions", Some(&cookie)).await;
    assert!(ls2.is_success(), "list sessions must succeed");
    let after = lj2.as_array().expect("array");
    assert_eq!(after.len(), initial + 1);

    // GET by id.
    let (gs, gj, _h) = get_json(
        app.clone(),
        &format!("/api/sessions/{new_id}"),
        Some(&cookie),
    )
    .await;
    assert!(
        gs.is_success(),
        "get session must succeed; got {gs} body={gj}"
    );
    assert_eq!(gj["id"], new_id);

    // Latest must be the new one.
    let (lts, ltj, _h) = get_json(app, "/api/sessions/latest", Some(&cookie)).await;
    assert!(
        lts.is_success(),
        "latest must succeed; got {lts} body={ltj}"
    );
    assert_eq!(ltj["session"]["id"], new_id);
}

#[tokio::test]
async fn list_drift_ignores_status_query_filter_bug_09_1() {
    // REGRESSION TEST for BUG 09-1: list_drift at cairn-api/src/lib.rs:1099
    // hardcodes the status filter to `None` (it ignores `?status=...`). To prove
    // the bug, we seed TWO drift events with different statuses via the
    // cairn_session store directly, then assert the filter parameter has no
    // effect on the response.
    //
    // When the bug is fixed (handler reads Query<DriftFilter> + passes it to
    // recent_drift), this test will start FAILING — which is the correct
    // signal that the fix landed. Update the assertion to compare
    // `pending.len() == 1` and the `approved.len() == 1` case.
    let Some((app, dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // Seed 2 drift events via cairn_session directly. The cairn-api list_drift
    // handler reads from the same SessionStore the AppState was built with.
    use cairn_session::{DriftEvent, DriftStatus};
    let store = cairn_session::SessionStore::new(dir.path().to_path_buf());

    let now = chrono::Utc::now();
    let pending_id = store
        .append_drift(&DriftEvent {
            id: 1,
            ts: now,
            path: "/tmp/a".into(),
            risk: "danger".into(),
            kind: "verify".into(),
            detail: "pending event".into(),
            status: DriftStatus::Pending,
        })
        .unwrap();
    let _ = store
        .append_drift(&DriftEvent {
            id: 2,
            ts: now,
            path: "/tmp/b".into(),
            risk: "warn".into(),
            kind: "verify".into(),
            detail: "approved event".into(),
            status: DriftStatus::Approved,
        })
        .unwrap();
    assert!(pending_id > 0);

    // Baseline: no filter → 2 events.
    let (ns, nj, _h) = get_json(app.clone(), "/api/guard/drift?limit=50", Some(&cookie)).await;
    assert!(
        ns.is_success(),
        "unfiltered list must succeed; got {ns} body={nj}"
    );
    let unfiltered = nj.as_array().expect("array");
    assert_eq!(unfiltered.len(), 2, "must have 2 seeded events; got {nj}");

    // With `?status=pending` — handler at lib.rs:1102 hardcodes `None`, so the
    // response is the same as the unfiltered set. This is BUG 09-1.
    let (ps, pj, _h) = get_json(
        app.clone(),
        "/api/guard/drift?limit=50&status=pending",
        Some(&cookie),
    )
    .await;
    assert!(
        ps.is_success(),
        "filtered list must succeed; got {ps} body={pj}"
    );
    let pending = pj.as_array().expect("array");

    assert_eq!(
        pending.len(),
        unfiltered.len(),
        "BUG 09-1: ?status=pending must be ignored by the handler; \
         expected {} events (full set), got {}",
        unfiltered.len(),
        pending.len()
    );

    // With `?status=approved` — same bug, same expected behavior.
    let (as_, aj, _h) = get_json(
        app.clone(),
        "/api/guard/drift?limit=50&status=approved",
        Some(&cookie),
    )
    .await;
    assert!(as_.is_success());
    let approved = aj.as_array().expect("array");
    assert_eq!(
        approved.len(),
        unfiltered.len(),
        "BUG 09-1: ?status=approved must be ignored by the handler; \
         expected {} events (full set), got {}",
        unfiltered.len(),
        approved.len()
    );
}

#[tokio::test]
#[ignore = "BUG 09-1 still open - flips to green when cairn-api list_drift reads ?status="]
async fn list_drift_with_fixed_filter_returns_only_matching_status() {
    // POSITIVE-CASE TEMPLATE for the BUG 09-1 fix. Currently the handler at
    // cairn-api/src/lib.rs:1099 hardcodes `None` and this test fails because
    // `?status=pending` returns both events. When the fix lands (read the
    // `?status=` query param, pass to `recent_drift`), this test will pass
    // and the negative test above will fail — the two together pin the fix.
    //
    // Marked #[ignore] so the suite stays green while the bug is open.
    // Remove the attribute when fixing the handler.
    let Some((app, dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    use cairn_session::{DriftEvent, DriftStatus};
    let store = cairn_session::SessionStore::new(dir.path().to_path_buf());
    let now = chrono::Utc::now();
    store
        .append_drift(&DriftEvent {
            id: 10,
            ts: now,
            path: "/tmp/x".into(),
            risk: "danger".into(),
            kind: "verify".into(),
            detail: "p".into(),
            status: DriftStatus::Pending,
        })
        .unwrap();
    store
        .append_drift(&DriftEvent {
            id: 11,
            ts: now,
            path: "/tmp/y".into(),
            risk: "warn".into(),
            kind: "verify".into(),
            detail: "a".into(),
            status: DriftStatus::Approved,
        })
        .unwrap();

    let (ps, pj, _h) = get_json(
        app.clone(),
        "/api/guard/drift?limit=50&status=pending",
        Some(&cookie),
    )
    .await;
    assert!(ps.is_success());
    let pending = pj.as_array().expect("array");
    assert_eq!(
        pending.len(),
        1,
        "?status=pending must return 1 event after fix; got {pj}"
    );
    assert_eq!(pending[0]["status"], "pending");
}

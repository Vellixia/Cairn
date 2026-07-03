//! 26 — cairn-api `/api/cron/*` HTTP routes (v0.8.0 Sprint 4), mounted in-process via
//! tower::oneshot. The manual-trigger endpoint calls `cron::run_job_now` directly (the same
//! function the scheduler's own ticks use) - no `JobScheduler` needs to be running for these
//! tests, only the HTTP layer + an in-memory store.
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

/// Setup admin + login -> return the session cookie value.
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
async fn list_jobs_shows_all_six_with_no_prior_run() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = request_json(app, "GET", "/api/cron/jobs", Some(&cookie)).await;
    assert!(status.is_success());
    let jobs = body.as_array().expect("array");
    assert_eq!(
        jobs.len(),
        6,
        "session-gc, memory-decay, access-log-prune, llm-intelligence, memory-demote, tune"
    );
    let names: Vec<&str> = jobs.iter().filter_map(|j| j["name"].as_str()).collect();
    assert!(names.contains(&"session-gc"));
    assert!(names.contains(&"memory-decay"));
    assert!(names.contains(&"access-log-prune"));
    assert!(names.contains(&"llm-intelligence"));
    assert!(names.contains(&"memory-demote"));
    assert!(names.contains(&"tune"));
    for j in jobs {
        assert!(j["last_run"].is_null(), "no job has run yet");
    }
}

#[tokio::test]
async fn health_shows_all_six_unstale_and_not_running_with_no_prior_run() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = request_json(app, "GET", "/api/cron/health", Some(&cookie)).await;
    assert!(status.is_success(), "got {status} body={body}");
    let jobs = body.as_array().expect("array");
    assert_eq!(jobs.len(), 6);
    for j in jobs {
        assert!(j["last_run_at"].is_null());
        assert!(j["last_status"].is_null());
        assert_eq!(j["running"], false);
        // A job that has never run is never "stale" - there's nothing wrong with a freshly
        // started server waiting for its first scheduled tick.
        assert_eq!(j["stale"], false);
    }
}

#[tokio::test]
async fn health_reflects_last_run_after_a_manual_trigger() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, _, _) = request_json(
        app.clone(),
        "POST",
        "/api/cron/run/session-gc",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());

    let (status, body, _) = request_json(app, "GET", "/api/cron/health", Some(&cookie)).await;
    assert!(status.is_success());
    let session_gc = body
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["name"] == "session-gc")
        .expect("session-gc listed");
    assert_eq!(session_gc["last_status"], "ok");
    assert!(!session_gc["last_run_at"].is_null());
    assert_eq!(session_gc["running"], false, "run_job_now returns synchronously");
}

#[tokio::test]
async fn cron_health_requires_authentication() {
    let Some((app, _dir)) = state() else { return };
    let (status, _, _) = request_json(app, "GET", "/api/cron/health", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn memory_demote_job_runs_and_reports_zero_on_an_empty_store() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = request_json(
        app,
        "POST",
        "/api/cron/run/memory-demote",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["job"], "memory-demote");
    assert_eq!(body["outcome"], "ok");
}

#[tokio::test]
async fn memory_decay_job_reports_hygiene_counts_alongside_decay() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // v0.8.0 Sprint 9: the dedup sweep and working-tier cap ride along on this same job -
    // an empty store exercises all three passes and should report zero for each.
    let (status, body, _) = request_json(
        app,
        "POST",
        "/api/cron/run/memory-decay",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["job"], "memory-decay");
    assert_eq!(body["outcome"], "ok");
    let detail = body["detail"].as_str().unwrap();
    assert!(detail.contains("decayed confidence on 0 memories"), "{detail}");
    assert!(detail.contains("deduped 0"), "{detail}");
    assert!(detail.contains("capped 0"), "{detail}");
}

#[tokio::test]
async fn tune_job_skips_when_too_few_queries_observed() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    // A fresh engine has recorded zero queries - `followup_rate()` would report `0.0` for
    // that, indistinguishable from "genuinely excellent recall" if read at face value. The
    // minimum-sample gate must catch this and skip rather than act on it.
    let (status, body, _) = request_json(app, "POST", "/api/cron/run/tune", Some(&cookie)).await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["job"], "tune");
    assert_eq!(body["outcome"], "ok");
    assert!(body["detail"].as_str().unwrap().contains("skipped"));
}

#[tokio::test]
async fn llm_intelligence_job_is_a_noop_when_llm_disabled() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = request_json(
        app,
        "POST",
        "/api/cron/run/llm-intelligence",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["job"], "llm-intelligence");
    assert_eq!(body["outcome"], "ok");
    assert!(body["detail"]
        .as_str()
        .unwrap()
        .contains("CAIRN_LLM_CONSOLIDATION disabled"));
}

#[tokio::test]
async fn manual_trigger_runs_the_job_and_records_history() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    let (status, body, _) = request_json(
        app.clone(),
        "POST",
        "/api/cron/run/session-gc",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success(), "got {status} body={body}");
    assert_eq!(body["job"], "session-gc");
    assert_eq!(body["outcome"], "ok");

    let (status, history, _) =
        request_json(app.clone(), "GET", "/api/cron/history", Some(&cookie)).await;
    assert!(status.is_success());
    let runs = history.as_array().expect("array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["job"], "session-gc");

    // GET /api/cron/jobs should now show the last_run for session-gc.
    let (_, jobs, _) = request_json(app, "GET", "/api/cron/jobs", Some(&cookie)).await;
    let session_gc = jobs
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["name"] == "session-gc")
        .expect("session-gc listed");
    assert!(!session_gc["last_run"].is_null());
}

#[tokio::test]
async fn history_filters_by_job() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;

    for job in ["session-gc", "memory-decay", "session-gc"] {
        let (status, _, _) = request_json(
            app.clone(),
            "POST",
            &format!("/api/cron/run/{job}"),
            Some(&cookie),
        )
        .await;
        assert!(status.is_success());
    }

    let (status, filtered, _) = request_json(
        app,
        "GET",
        "/api/cron/history?job=session-gc",
        Some(&cookie),
    )
    .await;
    assert!(status.is_success());
    let runs = filtered.as_array().expect("array");
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|r| r["job"] == "session-gc"));
}

#[tokio::test]
async fn triggering_an_unknown_job_returns_404() {
    let Some((app, _dir)) = state() else { return };
    let cookie = login_cookie(app.clone()).await;
    let (status, _, _) = request_json(
        app,
        "POST",
        "/api/cron/run/does-not-exist",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cron_routes_require_authentication() {
    let Some((app, _dir)) = state() else { return };
    let (status, _, _) = request_json(app, "GET", "/api/cron/jobs", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

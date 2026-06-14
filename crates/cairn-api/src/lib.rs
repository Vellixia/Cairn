//! The Cairn HTTP API and embedded control-plane UI.
//!
//! Exposes health/stats plus the context (read/expand/assemble), memory (remember/recall/wakeup),
//! and guard (verify) engines over REST, and serves the embedded Next.js control plane — with a
//! built-in fallback page when the UI hasn't been built.

mod ui;

use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use cairn_assemble::{Assembler, AssemblyReport};
use cairn_context::{ContextEngine, ReadMode, ReadResult};
use cairn_core::{Config, Memory, NewMemory};
use cairn_guard::{Guard, VerifyReport};
use cairn_memory::{MemoryEngine, ScoredMemory};
use cairn_profile::Profile;
use cairn_shell::{Compressed, ShellCompressor};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

/// Shared application state, cheaply cloneable (everything behind `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub ctx: Arc<ContextEngine>,
    pub mem: Arc<MemoryEngine>,
    pub guard: Arc<Guard>,
    pub asm: Arc<Assembler>,
    pub shell: Arc<ShellCompressor>,
    pub profile: Arc<Profile>,
}

impl AppState {
    pub fn new(cfg: &Config) -> cairn_core::Result<Self> {
        let store = Arc::new(Store::open(cfg)?);
        let ctx = Arc::new(ContextEngine::new(store.clone()));
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        let guard = Arc::new(Guard::new(store.clone()));
        let asm = Arc::new(Assembler::new(mem.clone()));
        let shell = Arc::new(ShellCompressor::new(store.clone()));
        let profile = Arc::new(Profile::new(mem.clone()));
        Ok(Self {
            store,
            ctx,
            mem,
            guard,
            asm,
            shell,
            profile,
        })
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/stats", get(stats))
        .route("/api/context/read", get(read))
        .route("/api/context/expand", get(expand))
        .route("/api/context/assemble", get(assemble))
        .route("/api/memory", post(remember))
        .route("/api/memory/recall", get(recall))
        .route("/api/memory/wakeup", get(wakeup))
        .route("/api/memory/consolidate", post(consolidate_memory))
        .route("/api/guard/verify", post(verify))
        .route("/api/shell/compress", post(shell_compress))
        .route("/api/profile", get(get_profile).post(post_prefer))
        .route("/api/sync/pull", get(sync_pull))
        .route("/api/sync/push", post(sync_push))
        .fallback(static_handler)
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Bind and serve until shutdown.
pub async fn serve(addr: SocketAddr, state: AppState) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await
}

/// The web UI (landing + control plane), embedded from the Next.js static export. In dev builds
/// rust-embed reads it from disk; in release it is baked into the binary. If the export is absent
/// (UI not built), requests fall back to the lightweight built-in page.
#[derive(RustEmbed)]
#[folder = "../../web/out"]
struct WebAssets;

async fn static_handler(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let key = if path.is_empty() {
        "index.html".to_string()
    } else if <WebAssets as RustEmbed>::get(path).is_some() {
        path.to_string()
    } else if <WebAssets as RustEmbed>::get(&format!("{path}.html")).is_some() {
        format!("{path}.html")
    } else {
        "index.html".to_string()
    };
    match <WebAssets as RustEmbed>::get(&key) {
        Some(file) => (
            [(axum::http::header::CONTENT_TYPE, content_type(&key))],
            file.data.into_owned(),
        )
            .into_response(),
        None => Html(ui::INDEX_HTML).into_response(),
    }
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") | Some("map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

// ---- handlers ----------------------------------------------------------------------------------

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "name": "cairn",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn stats(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "memories": s.store.count_memories()? })))
}

#[derive(Deserialize)]
struct ReadQuery {
    path: String,
    #[serde(default)]
    mode: Option<String>,
}

async fn read(
    State(s): State<AppState>,
    Query(q): Query<ReadQuery>,
) -> Result<Json<ReadResult>, ApiError> {
    let mode = ReadMode::parse(q.mode.as_deref());
    Ok(Json(s.ctx.read(Path::new(&q.path), mode)?))
}

#[derive(Deserialize)]
struct ExpandQuery {
    hash: String,
}

async fn expand(
    State(s): State<AppState>,
    Query(q): Query<ExpandQuery>,
) -> Result<Json<Value>, ApiError> {
    match s.ctx.expand(&q.hash)? {
        Some(content) => Ok(Json(json!({ "hash": q.hash, "content": content }))),
        None => Err(ApiError(StatusCode::NOT_FOUND, "unknown handle".into())),
    }
}

async fn remember(
    State(s): State<AppState>,
    Json(input): Json<NewMemory>,
) -> Result<Json<Memory>, ApiError> {
    Ok(Json(s.mem.remember(input)?))
}

#[derive(Deserialize)]
struct RecallQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

async fn recall(
    State(s): State<AppState>,
    Query(q): Query<RecallQuery>,
) -> Result<Json<Vec<ScoredMemory>>, ApiError> {
    Ok(Json(s.mem.recall(&q.q, q.limit.unwrap_or(10))?))
}

#[derive(Deserialize)]
struct WakeupQuery {
    #[serde(default)]
    limit: Option<usize>,
}

async fn wakeup(
    State(s): State<AppState>,
    Query(q): Query<WakeupQuery>,
) -> Result<Json<Vec<Memory>>, ApiError> {
    Ok(Json(s.mem.wakeup(q.limit.unwrap_or(12))?))
}

async fn consolidate_memory(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "promoted": s.mem.consolidate()? })))
}

#[derive(Deserialize)]
struct VerifyBody {
    path: String,
    content: String,
}

async fn verify(
    State(s): State<AppState>,
    Json(b): Json<VerifyBody>,
) -> Result<Json<VerifyReport>, ApiError> {
    Ok(Json(s.guard.verify_edit(Path::new(&b.path), &b.content)?))
}

#[derive(Deserialize)]
struct AssembleQuery {
    q: String,
    #[serde(default)]
    budget: Option<usize>,
}

async fn assemble(
    State(s): State<AppState>,
    Query(q): Query<AssembleQuery>,
) -> Result<Json<AssemblyReport>, ApiError> {
    Ok(Json(s.asm.assemble(&q.q, q.budget.unwrap_or(2000))?))
}

#[derive(Deserialize)]
struct ShellCompressBody {
    command: String,
    output: String,
}

async fn shell_compress(
    State(s): State<AppState>,
    Json(b): Json<ShellCompressBody>,
) -> Result<Json<Compressed>, ApiError> {
    Ok(Json(s.shell.compress(&b.command, &b.output)?))
}

async fn get_profile(State(s): State<AppState>) -> Result<Json<Vec<Memory>>, ApiError> {
    Ok(Json(s.profile.preferences()?))
}

#[derive(Deserialize)]
struct PreferBody {
    rule: String,
}

async fn post_prefer(
    State(s): State<AppState>,
    Json(b): Json<PreferBody>,
) -> Result<Json<Memory>, ApiError> {
    Ok(Json(s.profile.prefer(&b.rule)?))
}

// ---- sync + auth -------------------------------------------------------------------------------

#[derive(Deserialize)]
struct SyncPullQuery {
    #[serde(default)]
    since: Option<String>,
}

async fn sync_pull(
    State(s): State<AppState>,
    Query(q): Query<SyncPullQuery>,
) -> Result<Json<Value>, ApiError> {
    let since = q
        .since
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap());
    let memories = s.store.memories_since(since)?;
    Ok(Json(json!({
        "memories": memories,
        "now": Utc::now().to_rfc3339(),
    })))
}

#[derive(Deserialize)]
struct SyncPushBody {
    memories: Vec<Memory>,
}

async fn sync_push(
    State(s): State<AppState>,
    Json(b): Json<SyncPushBody>,
) -> Result<Json<Value>, ApiError> {
    let mut applied = 0usize;
    for m in &b.memories {
        if s.store.upsert_memory(m)? {
            applied += 1;
        }
    }
    Ok(Json(
        json!({ "applied": applied, "received": b.memories.len() }),
    ))
}

/// Bearer-token auth. The web UI and `/api/health` are always open; other `/api/*` routes require
/// a valid device token *once any token has been created* (so local-only setups stay friction-free).
async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let needs_auth = path.starts_with("/api/") && path != "/api/health";
    if needs_auth {
        match s.store.count_tokens() {
            Ok(0) => {}
            Ok(_) => {
                let ok = req
                    .headers()
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .map(|t| s.store.validate_token(t).unwrap_or(false))
                    .unwrap_or(false);
                if !ok {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "invalid or missing device token" })),
                    )
                        .into_response();
                }
            }
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "auth check failed" })),
                )
                    .into_response()
            }
        }
    }
    next.run(req).await
}

// ---- error plumbing ----------------------------------------------------------------------------

/// A simple API error that renders as JSON `{ "error": ... }`.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<cairn_core::Error> for ApiError {
    fn from(e: cairn_core::Error) -> Self {
        use cairn_core::Error::*;
        let code = match e {
            NotFound(_) => StatusCode::NOT_FOUND,
            Invalid(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(code, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_maps_common_extensions() {
        assert_eq!(content_type("dir/index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("_next/main.js"),
            "text/javascript; charset=utf-8"
        );
        assert!(content_type("noext").contains("octet-stream"));
    }

    #[tokio::test]
    async fn root_serves_ok() {
        let resp = static_handler("/".parse().unwrap()).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
}

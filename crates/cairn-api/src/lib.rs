//! The Cairn HTTP API and embedded control-plane UI.
//!
//! Exposes health/stats plus the context (read/expand/assemble), memory (remember/recall/wakeup),
//! and guard (verify, anchor, checkpoint/rollback) engines over REST, and serves the embedded
//! Next.js control plane — with a built-in fallback page when the UI hasn't been built.

mod auth;
mod rate_limit;
mod ui;

use crate::auth::{extract_bearer, TokenInfo, TokenSigner};
use crate::rate_limit::RateLimiter;
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
use cairn_core::{Config, Memory, NewMemory, TlsConfig};
use cairn_guard::{Checkpoint, Guard, RollbackReport, VerifyReport};
use cairn_memory::{MemoryEngine, ScoredMemory};
use cairn_profile::Profile;
use cairn_shell::{Compressed, ShellCompressor};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

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
    pub san: Arc<cairn_share::Sanitizer>,
    /// Config used to create this state, kept so the MCP-over-HTTP bridge can spawn an
    /// `cairn_mcp::McpServer` without reopening everything by hand.
    pub cfg: Config,
    pub tls: Option<TlsConfig>,
    pub insecure: bool,
    pub cors_origins: Vec<String>,
    pub rate_limiter: RateLimiter,
    pub pair_rate_limiter: RateLimiter,
    signer: Option<Arc<TokenSigner>>,
}

impl AppState {
    pub fn new(cfg: &Config) -> cairn_core::Result<Self> {
        let store = Arc::new(Store::open(cfg)?);
        let ctx = Arc::new(ContextEngine::new_with_root(
            store.clone(),
            cfg.workspace_root.clone(),
        ));
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        let guard = Arc::new(Guard::new(store.clone()));
        let asm = Arc::new(Assembler::new(mem.clone()));
        let shell = Arc::new(ShellCompressor::new(store.clone()));
        let profile = Arc::new(Profile::new(mem.clone()));
        let san = Arc::new(cairn_share::Sanitizer::new());
        let signer = cfg.secret_key.as_ref().map(|k| {
            Arc::new(
                TokenSigner::new(k.clone()).expect("CAIRN_SECRET_KEY must be non-empty for auth"),
            )
        });
        Ok(Self {
            store,
            ctx,
            mem,
            guard,
            asm,
            shell,
            profile,
            san,
            cfg: cfg.clone(),
            tls: cfg.tls.clone(),
            insecure: cfg.insecure,
            cors_origins: cfg.cors_origins.clone(),
            rate_limiter: RateLimiter::new(60, std::time::Duration::from_secs(60)),
            pair_rate_limiter: RateLimiter::new(5, std::time::Duration::from_secs(60)),
            signer,
        })
    }

    /// Issue a signed JWT for a newly created token id/name. Panics if no secret is configured.
    pub fn sign_token(
        &self,
        id: &str,
        name: &str,
        scope: cairn_core::TokenScope,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> String {
        self.signer
            .as_ref()
            .expect("CAIRN_SECRET_KEY is required to sign device tokens")
            .mint(id, name, scope, expires_at)
    }

    /// Decode a bearer JWT into its token id + scope. Returns None if no secret is configured or
    /// the token is invalid/missing.
    pub fn verify_bearer(&self, bearer: &str) -> Option<TokenInfo> {
        self.signer.as_ref()?.verify(bearer).ok()
    }

    /// Revoke the token identified by a bearer JWT. Returns Ok(false) if the JWT is invalid.
    pub fn revoke_bearer(&self, bearer: &str) -> cairn_core::Result<bool> {
        let Some(info) = self.verify_bearer(bearer) else {
            return Ok(false);
        };
        self.store.revoke_token(&info.id)
    }
}

pub fn router(state: AppState) -> Router {
    let cors = build_cors(&state.cors_origins);
    let pair_limiter = state.pair_rate_limiter.clone();
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
        .route("/api/guard/anchor", get(get_anchor).post(post_anchor))
        .route("/api/guard/checkpoint", post(create_checkpoint))
        .route("/api/guard/checkpoints", get(list_checkpoints))
        .route("/api/guard/rollback", post(rollback_checkpoint))
        .route("/api/shell/compress", post(shell_compress))
        .route("/api/profile", get(get_profile).post(post_prefer))
        .route("/api/share/sanitize", post(sanitize_text))
        .route("/api/share/export", get(share_export))
        .route("/api/share/import", post(share_import))
        .route("/api/pool/contribute", post(pool_contribute))
        .route("/api/pool", get(pool_list))
        .route("/api/tools/list", get(tools_list))
        .route("/api/tools/call", post(tools_call))
        .route("/api/pair/new", post(pair_new))
        .route(
            "/api/pair/claim",
            post(pair_claim).layer(axum::middleware::from_fn_with_state(
                pair_limiter,
                rate_limit::rate_limit_middleware,
            )),
        )
        .route("/api/sync/pull", get(sync_pull))
        .route("/api/sync/push", post(sync_push))
        .fallback(static_handler)
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.rate_limiter.clone(),
            rate_limit::rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(cors)
        .with_state(state)
}

/// Build a CORS layer from the configured origins. Empty origins means same-origin only (no
/// cross-origin requests allowed). Specific origins are allowlisted explicitly.
///
/// `["*"]` is rejected outright: the Cairn API is fully authenticated, and a wildcard origin
/// combined with credentialed requests is the dangerous combo that CORS specifically defends
/// against. Users who actually want full permissive behavior must opt in by listing every
/// trusted origin explicitly.
fn build_cors(origins: &[String]) -> CorsLayer {
    if origins.is_empty() {
        // Same-origin only: no cross-origin requests allowed. The browser enforces this by
        // default when no CORS headers are present, so we return a restrictive layer.
        return CorsLayer::new()
            .allow_methods(AllowMethods::list(vec![
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ]))
            .allow_headers(AllowHeaders::list(vec![
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
            ]));
    }
    if origins.iter().any(|o| o == "*") {
        // Hard fail. Use tracing::error so it shows up in any alerting that's watching for
        // ERROR-level events. We still return a restrictive layer so a misconfigured server
        // doesn't accidentally open itself up.
        tracing::error!(
            "CAIRN_CORS_ORIGINS contains '*' — wildcard origin rejected. The Cairn API is \
             authenticated; list explicit origins instead (e.g. CAIRN_CORS_ORIGINS=https://app.example.com). \
             Falling back to same-origin-only CORS."
        );
        return CorsLayer::new()
            .allow_methods(AllowMethods::list(vec![
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::OPTIONS,
            ]))
            .allow_headers(AllowHeaders::list(vec![
                axum::http::header::AUTHORIZATION,
                axum::http::header::CONTENT_TYPE,
            ]));
    }
    let allowed: Vec<axum::http::HeaderValue> =
        origins.iter().filter_map(|o| o.parse().ok()).collect();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed))
        .allow_methods(AllowMethods::list(vec![
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ]))
        .allow_headers(AllowHeaders::list(vec![
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]))
}

/// Bind and serve until shutdown.
///
/// TLS posture is driven by the bind address and the optional `tls` material on the
/// [`AppState::config_view`]:
///
/// - **Loopback bind, no TLS** → plain HTTP (dev-friendly default).
/// - **Non-loopback bind, no TLS** → refuses to start; the caller surfaces an error explaining
///   that network exposure requires `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`.
/// - **TLS material present** → serves HTTPS via `axum-server` (rustls).
pub async fn serve(addr: SocketAddr, mut state: AppState) -> std::io::Result<()> {
    if let Some(tls) = state.tls.take() {
        return serve_https(addr, state, tls.cert, tls.key).await;
    }
    // No TLS material. We need to look at the bind address: if it's a loopback address we allow
    // plain HTTP for local dev, otherwise we refuse unless `CAIRN_INSECURE=1` was explicitly set.
    if !is_loopback_addr(addr) && !state.insecure {
        return Err(std::io::Error::other(format!(
            "refusing to serve HTTP on non-loopback address {addr}: \
             Cairn's API is authenticated and must not travel in cleartext over a network. \
             Set CAIRN_TLS_CERT and CAIRN_TLS_KEY to a PEM cert+key pair (e.g. via \
             `mkcert` or a reverse proxy), bind to 127.0.0.1/localhost, or set \
             CAIRN_INSECURE=1 if this is a trusted local/private network."
        )));
    }

    if state.insecure && !is_loopback_addr(addr) {
        tracing::warn!(
            "CAIRN_INSECURE=1: serving plain HTTP on {addr}. Do not use this on a public network."
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

async fn serve_https(
    addr: SocketAddr,
    state: AppState,
    cert: PathBuf,
    key: PathBuf,
) -> std::io::Result<()> {
    use axum_server::tls_rustls::RustlsConfig;
    let rustls = RustlsConfig::from_pem_file(&cert, &key)
        .await
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "failed to load TLS material from {} / {}: {e}",
                    cert.display(),
                    key.display()
                ),
            )
        })?;
    let app = router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum_server::bind_rustls(addr, rustls).serve(app).await
}

/// True if `addr` resolves to a loopback interface (v4 or v6).
fn is_loopback_addr(addr: SocketAddr) -> bool {
    addr.ip().is_loopback()
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
    Ok(Json(json!({
        "memories": s.store.count_memories()?,
        "checkpoints": s.guard.list_checkpoints()?.len(),
        "preferences": s.profile.preferences()?.len(),
        "anchor": s.guard.anchor()?,
        "reliability": s.guard.reliability()?,
    })))
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
    Json(mut input): Json<NewMemory>,
) -> Result<Json<Memory>, ApiError> {
    // Strip injected preference delimiter blocks so memory content cannot smuggle itself back
    // into the preference pipeline as a model directive.
    input.content = cairn_profile::strip_preference_blocks(&input.content);
    input.suspicious =
        Some(input.suspicious.unwrap_or(false) || cairn_profile::is_suspicious(&input.content));
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
    let report = s.guard.verify_edit(Path::new(&b.path), &b.content)?;
    let _ = s.guard.note_verify(&report);
    Ok(Json(report))
}

async fn get_anchor(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "anchor": s.guard.anchor()? })))
}

#[derive(Deserialize)]
struct AnchorBody {
    goal: String,
}

async fn post_anchor(
    State(s): State<AppState>,
    Json(b): Json<AnchorBody>,
) -> Result<Json<Value>, ApiError> {
    let meta = s.guard.set_anchor(&b.goal)?;
    Ok(Json(
        json!({ "anchor": meta.goal, "suspicious": meta.suspicious }),
    ))
}

#[derive(Deserialize)]
struct CheckpointQuery {
    #[serde(default)]
    label: Option<String>,
}

async fn create_checkpoint(
    State(s): State<AppState>,
    Query(q): Query<CheckpointQuery>,
) -> Result<Json<Checkpoint>, ApiError> {
    let label = q.label.unwrap_or_else(|| "checkpoint".to_string());
    Ok(Json(s.guard.checkpoint(&label)?))
}

async fn list_checkpoints(State(s): State<AppState>) -> Result<Json<Vec<Checkpoint>>, ApiError> {
    Ok(Json(s.guard.list_checkpoints()?))
}

#[derive(Deserialize)]
struct RollbackQuery {
    id: String,
}

async fn rollback_checkpoint(
    State(s): State<AppState>,
    Query(q): Query<RollbackQuery>,
) -> Result<Json<RollbackReport>, ApiError> {
    Ok(Json(s.guard.rollback(&q.id)?))
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

#[derive(Deserialize)]
struct SanitizeBody {
    text: String,
}

async fn sanitize_text(
    State(s): State<AppState>,
    Json(b): Json<SanitizeBody>,
) -> Json<cairn_share::Sanitized> {
    Json(s.san.sanitize(&b.text))
}

/// Export a sanitized, shareable bundle: secrets/PII redacted, and memories that still classify as
/// private are withheld entirely.
async fn share_export(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    let mems = s.store.all_memories()?;
    let (bundle, stats) = s.san.bundle(&mems);
    Ok(Json(json!({
        "schema": bundle.schema,
        "version": bundle.version,
        "total": stats.total,
        "shared": stats.shared,
        "needs_review": stats.needs_review,
        "withheld": stats.withheld,
        "memories": bundle.memories,
    })))
}

/// Ingest a sanitized share bundle as `shared`-tagged memories (deduplicated against existing).
async fn share_import(
    State(s): State<AppState>,
    Json(bundle): Json<cairn_share::ShareBundle>,
) -> Result<Json<Value>, ApiError> {
    let news = bundle.into_new_memories();
    let total = news.len();
    for nm in news {
        s.mem.remember(nm)?;
    }
    Ok(Json(json!({ "ingested": total })))
}

/// Accept a contribution into this server's shared pool. The server re-sanitizes every memory
/// itself (defense-in-depth — a client's redaction is never trusted) and rejects anything that
/// still contains a hard secret; the rest is stored under `pool` provenance, deduplicated.
async fn pool_contribute(
    State(s): State<AppState>,
    Json(bundle): Json<cairn_share::ShareBundle>,
) -> Result<Json<Value>, ApiError> {
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    for sm in bundle.memories {
        let scan = s.san.sanitize(&sm.content);
        if scan.sensitivity == cairn_share::Sensitivity::Private {
            rejected += 1;
            continue;
        }
        let mut nm = NewMemory::new(scan.text);
        nm.kind = Some(sm.kind);
        nm.concepts = sm.concepts.iter().map(|c| s.san.sanitize(c).text).collect();
        nm.session_id = Some("pool".to_string());
        s.mem.remember(nm)?;
        accepted += 1;
    }
    Ok(Json(json!({ "accepted": accepted, "rejected": rejected })))
}

/// Serve this server's shared pool as a sanitized bundle others can pull.
async fn pool_list(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    let memories: Vec<_> = s
        .store
        .all_memories()?
        .into_iter()
        .filter(|m| m.session_id.as_deref() == Some("pool"))
        .map(|m| s.san.sanitize_memory(&m))
        .collect();
    Ok(Json(json!({
        "schema": cairn_share::ShareBundle::SCHEMA,
        "version": 1,
        "count": memories.len(),
        "memories": memories,
    })))
}

/// Generate a short, unambiguous pairing code (~40 bits of entropy): 8 chars, no 0/O/1/I/L.
pub fn pairing_code() -> String {
    const ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let mut acc = 0u64;
    for &b in &bytes[..5] {
        acc = (acc << 8) | b as u64;
    }
    (0..8)
        .map(|i| ALPHA[((acc >> (35 - i * 5)) & 0x1f) as usize] as char)
        .collect()
}

#[derive(Deserialize)]
struct PairNewBody {
    #[serde(default)]
    name: Option<String>,
}

/// Mint a device token and a short-lived pairing code for it (authed). A new device claims the code
/// to receive the token, so long secrets never have to be copied around.
async fn pair_new(
    State(s): State<AppState>,
    Json(b): Json<PairNewBody>,
) -> Result<Json<Value>, ApiError> {
    let name = b
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "device".to_string());
    let token = s.store.create_token(&name)?;
    let code = pairing_code();
    let expires = (Utc::now() + chrono::Duration::minutes(10))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    s.store.create_pairing(&code, &token.id, &name, &expires)?;
    let bearer = s.sign_token(&token.id, &name, cairn_core::TokenScope::Write, None);
    Ok(Json(
        json!({ "code": code, "name": name, "expires_at": expires, "token": bearer }),
    ))
}

#[derive(Deserialize)]
struct PairClaimBody {
    code: String,
}

/// Claim a pairing code (open endpoint): returns the device token if the code is valid, unexpired,
/// and unclaimed. Single-use.
async fn pair_claim(
    State(s): State<AppState>,
    Json(b): Json<PairClaimBody>,
) -> Result<Json<Value>, ApiError> {
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    match s
        .store
        .claim_pairing(b.code.trim().to_uppercase().as_str(), &now)?
    {
        Some((token_id, name)) => {
            let bearer = s.sign_token(&token_id, &name, cairn_core::TokenScope::Write, None);
            Ok(Json(json!({ "token": bearer, "name": name })))
        }
        None => {
            tracing::warn!(code = %b.code, "failed pairing claim attempt (invalid or expired code)");
            Err(ApiError(
                StatusCode::NOT_FOUND,
                "invalid or expired pairing code".into(),
            ))
        }
    }
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

async fn tools_list(State(_s): State<AppState>) -> Json<Value> {
    Json(json!({ "tools": cairn_mcp::tool_defs() }))
}

#[derive(Deserialize)]
struct ToolCallBody {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn tools_call(
    State(s): State<AppState>,
    Json(b): Json<ToolCallBody>,
) -> Result<Json<Value>, ApiError> {
    let mcp = cairn_mcp::McpServer::new(&s.cfg)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match mcp.dispatch(&b.name, &b.arguments) {
        Ok(text) => Ok(Json(
            json!({ "content": [{ "type": "text", "text": text }] }),
        )),
        Err(msg) => Ok(Json(
            json!({ "content": [{ "type": "text", "text": format!("error: {msg}") }], "isError": true }),
        )),
    }
}

/// Bearer-token auth. The web UI and `/api/health` are always open; other `/api/*` routes require
/// a valid device token *once any token has been created*. When no tokens exist, only loopback
/// (local) requests are allowed — remote API access without any device token is never safe.
async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // `/api/pair/claim` is open: a brand-new device has no token yet — the short-lived,
    // single-use pairing code is its credential.
    let needs_auth =
        path.starts_with("/api/") && path != "/api/health" && path != "/api/pair/claim";
    if needs_auth {
        match s.store.count_tokens() {
            Ok(0) => {
                // No tokens configured — require the request to be local (loopback).
                // Remote API access without any device token is never safe.
                let is_local = req
                    .extensions()
                    .get::<axum::extract::ConnectInfo<SocketAddr>>()
                    .map(|ci| ci.0.ip().is_loopback())
                    .unwrap_or(false);
                if !is_local {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({ "error": "no device tokens configured; create one with `cairn token create` or access from localhost" })),
                    )
                        .into_response();
                }
            }
            Ok(_) => {
                let ok = req
                    .headers()
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(extract_bearer)
                    .and_then(|bearer| s.verify_bearer(bearer))
                    .map(|info| {
                        let method = req.method().as_str();
                        let path = req.uri().path();
                        info.scope.allows(method, path)
                            && s.store.validate_token_id(&info.id).unwrap_or(false)
                    })
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
#[derive(Debug)]
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

    /// `None` when `CAIRN_HELIX_URL` is unset or HelixDB is unreachable (tests skip gracefully).
    /// The temp dir is a scratch workspace for the test's files (separate from the store).
    fn test_state() -> Option<(AppState, tempfile::TempDir)> {
        let cfg = cairn_store::Store::test_config()?;
        let dir = tempfile::tempdir().ok()?;
        Some((AppState::new(&cfg).ok()?, dir))
    }

    #[tokio::test]
    async fn guard_checkpoint_rollback_endpoints_restore_a_tracked_file() {
        let Some((state, dir)) = test_state() else {
            return;
        };
        let file = dir.path().join("work.txt");
        std::fs::write(&file, "original\n").unwrap();
        // Track the file by reading it through the context engine (records a baseline version).
        state.ctx.read(&file, ReadMode::Full).unwrap();

        // Create a checkpoint — it should capture the tracked file.
        let cp = create_checkpoint(
            State(state.clone()),
            Query(CheckpointQuery {
                label: Some("api".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(cp.label, "api");
        assert!(cp.files >= 1, "checkpoint should track the file");

        // It shows up in the list and in stats.
        let list = list_checkpoints(State(state.clone())).await.unwrap().0;
        assert!(list.iter().any(|c| c.id == cp.id));
        let st = stats(State(state.clone())).await.unwrap().0;
        assert!(st["checkpoints"].as_u64().unwrap() >= 1);

        // Corrupt the file, then roll back to the checkpoint.
        std::fs::write(&file, "DAMAGED\n").unwrap();
        let report = rollback_checkpoint(
            State(state.clone()),
            Query(RollbackQuery { id: cp.id.clone() }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(report.restored.len(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original\n");

        // Unknown checkpoint id surfaces as a 404.
        let err = rollback_checkpoint(
            State(state.clone()),
            Query(RollbackQuery { id: "nope".into() }),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn anchor_endpoints_round_trip() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let set = post_anchor(
            State(state.clone()),
            Json(AnchorBody {
                goal: "ship the API".into(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(set["anchor"], "ship the API");
        let got = get_anchor(State(state.clone())).await.unwrap().0;
        assert_eq!(got["anchor"], "ship the API");
    }

    #[tokio::test]
    async fn share_endpoints_sanitize_text_and_withhold_private_memories() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(NewMemory::new("prefer ripgrep over grep"))
            .unwrap();
        // Assembled at runtime so the repo stores no verbatim credential (push protection).
        let leak = format!("api_key = sk_{}_{}", "live", "abcdefghijklmnop12345678");
        state.mem.remember(NewMemory::new(leak.clone())).unwrap();

        // The sanitize endpoint redacts and classifies.
        let s = sanitize_text(State(state.clone()), Json(SanitizeBody { text: leak }))
            .await
            .0;
        assert_eq!(s.sensitivity, cairn_share::Sensitivity::Private);
        assert!(!s.text.contains("sk_live"));

        // The export endpoint withholds the private memory and keeps the clean one.
        let bundle = share_export(State(state.clone())).await.unwrap().0;
        assert_eq!(bundle["schema"], "cairn-share-bundle");
        assert_eq!(bundle["total"], 2);
        assert_eq!(bundle["withheld"], 1);
        assert_eq!(bundle["shared"], 1);
        let serialized = serde_json::to_string(&bundle).unwrap();
        assert!(!serialized.contains("abcdefghijklmnop12345678"));
    }

    #[tokio::test]
    async fn share_export_then_import_round_trips_into_a_fresh_instance() {
        let Some((src, _d1)) = test_state() else {
            return;
        };
        src.mem
            .remember(NewMemory::new("prefer ripgrep over grep"))
            .unwrap();

        // Export, then parse the bundle back (extra summary fields are ignored).
        let exported = share_export(State(src.clone())).await.unwrap().0;
        let bundle: cairn_share::ShareBundle = serde_json::from_value(exported).unwrap();

        // Import into a brand-new instance.
        let Some((dst, _d2)) = test_state() else {
            return;
        };
        let res = share_import(State(dst.clone()), Json(bundle))
            .await
            .unwrap()
            .0;
        assert_eq!(res["ingested"], 1);

        // The shared memory is now recallable there, tagged with `shared` provenance.
        let hits = dst.mem.recall("ripgrep", 5).unwrap();
        assert!(hits.iter().any(|h| h.memory.content.contains("ripgrep")));
        assert_eq!(hits[0].memory.session_id.as_deref(), Some("shared"));
    }

    #[tokio::test]
    async fn stats_surfaces_reliability_after_a_verify() {
        let Some((state, dir)) = test_state() else {
            return;
        };
        let f = dir.path().join("f.txt");
        let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&f, &original).unwrap();

        // A gutting edit recorded through the verify endpoint shows up as a danger.
        let _ = verify(
            State(state.clone()),
            Json(VerifyBody {
                path: f.to_string_lossy().into_owned(),
                content: "x\n".into(),
            }),
        )
        .await
        .unwrap();

        let st = stats(State(state.clone())).await.unwrap().0;
        assert_eq!(st["reliability"]["danger"], 1);
        assert!(st["reliability"]["samples"].as_u64().unwrap() >= 1);
        assert_eq!(st["reliability"]["score"], 0);
    }

    #[tokio::test]
    async fn pool_contribute_re_sanitizes_and_pull_serves_clean() {
        let Some((state, _dir)) = test_state() else {
            return;
        };

        fn shareable(content: &str) -> cairn_share::ShareableMemory {
            cairn_share::ShareableMemory {
                kind: cairn_core::MemoryKind::Note,
                content: content.into(),
                concepts: vec![],
                sensitivity: cairn_share::Sensitivity::Shareable,
                redactions: 0,
            }
        }

        // A buggy/malicious client sends one clean memory and one with a raw, unredacted secret it
        // falsely marked shareable. The server re-sanitizes and must reject the latter.
        let leaked = format!("token = sk_{}_{}", "live", "abcdefghijklmnop12345678");
        let bundle = cairn_share::ShareBundle {
            schema: cairn_share::ShareBundle::SCHEMA.into(),
            version: 1,
            memories: vec![shareable("use BM25 for recall ranking"), shareable(&leaked)],
        };

        let res = pool_contribute(State(state.clone()), Json(bundle))
            .await
            .unwrap()
            .0;
        assert_eq!(res["accepted"], 1);
        assert_eq!(res["rejected"], 1);

        let pool = pool_list(State(state.clone())).await.unwrap().0;
        assert_eq!(pool["count"], 1);
        let serialized = serde_json::to_string(&pool).unwrap();
        assert!(serialized.contains("BM25"));
        assert!(!serialized.contains("abcdefghijklmnop12345678"));
    }

    #[tokio::test]
    async fn pair_new_then_claim_yields_a_valid_token_once() {
        let Some((state, _dir)) = test_state() else {
            return;
        };

        // The host mints a pairing code.
        let created = pair_new(
            State(state.clone()),
            Json(PairNewBody {
                name: Some("laptop".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        let code = created["code"].as_str().unwrap().to_string();
        assert_eq!(created["name"], "laptop");
        assert_eq!(code.len(), 8);

        // The new device claims it → a real, valid device token.
        let claimed = pair_claim(
            State(state.clone()),
            Json(PairClaimBody { code: code.clone() }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(claimed["name"], "laptop");
        let token = claimed["token"].as_str().unwrap();
        let info = state
            .verify_bearer(token)
            .expect("claimed token is a valid JWT");
        assert!(state.store.validate_token_id(&info.id).unwrap());

        // Single-use: a second claim is a 404.
        let err = pair_claim(State(state.clone()), Json(PairClaimBody { code }))
            .await
            .err()
            .unwrap();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tools_list_and_call_expose_mcp_surface_over_http() {
        let Some((state, _dir)) = test_state() else {
            return;
        };

        let list = tools_list(State(state.clone())).await.0;
        let tools = list["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "remember"));
        assert!(tools.iter().any(|t| t["name"] == "recall"));

        let called = tools_call(
            State(state.clone()),
            Json(ToolCallBody {
                name: "remember".into(),
                arguments: json!({"content": "cairn exposes mcp tools over http", "kind": "decision"}),
            }),
        )
        .await
        .unwrap()
        .0;
        let text = called["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("remembered"));

        let recalled = tools_call(
            State(state.clone()),
            Json(ToolCallBody {
                name: "recall".into(),
                arguments: json!({"query": "mcp http", "limit": 5}),
            }),
        )
        .await
        .unwrap()
        .0;
        let text = recalled["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("mcp"), "recall text was: {text}");
    }

    #[test]
    fn build_cors_rejects_wildcard_origin() {
        // A bare "*" must not produce a permissive layer. We assert that the returned layer's
        // allow_origin is NOT `AllowOrigin::any()` (the permissive wildcard marker).
        let layer = build_cors(&["*".to_string()]);
        // AllowOrigin doesn't expose PartialEq, so we format-and-stringify instead: a permissive
        // layer renders as `*` in its Debug output; a restricted layer renders the origin list.
        let dbg = format!("{:?}", layer);
        assert!(
            !dbg.contains("Any") && !dbg.contains('*'),
            "wildcard CORS must not produce a permissive layer; got: {dbg}"
        );
    }

    #[test]
    fn build_cors_rejects_wildcard_in_list() {
        // Even if "*" is one entry among many, reject and fall back to restrictive. A user
        // passing ["https://app.example.com", "*"] gets the same protection as ["*"] alone.
        let layer = build_cors(&["https://app.example.com".to_string(), "*".to_string()]);
        let dbg = format!("{:?}", layer);
        assert!(
            !dbg.contains("Any"),
            "mixed list containing '*' must not produce a permissive layer; got: {dbg}"
        );
    }

    #[test]
    fn build_cors_accepts_explicit_origin_list() {
        let layer = build_cors(&["https://app.example.com".to_string()]);
        let dbg = format!("{:?}", layer);
        assert!(
            dbg.contains("https://app.example.com"),
            "explicit origin should appear in layer debug; got: {dbg}"
        );
    }

    #[test]
    fn build_cors_empty_yields_restrictive() {
        let layer = build_cors(&[]);
        let dbg = format!("{:?}", layer);
        assert!(
            !dbg.contains("Any"),
            "empty origin list must not produce a permissive layer; got: {dbg}"
        );
    }
}

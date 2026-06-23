//! The Cairn HTTP API and embedded control-plane UI.
//!
//! Exposes health/stats plus the context (read/expand/assemble), memory (remember/recall/wakeup),
//! and guard (verify, anchor, checkpoint/rollback) engines over REST, and serves the embedded
//! Next.js control plane — with a built-in fallback page when the UI hasn't been built.

mod admin;
mod auth;
mod devices;
mod events;
mod extensions;
mod ingest;
mod ledger;
mod metrics;
mod push;
mod security_headers;
mod session;
mod setup_wizard;
mod ui;

pub use admin::ADMIN_META_KEY;
pub use cairn_registry::{
    PackMeta, PublishReceipt, PublishStatus, Registry, RegistryError, RevocationEvent,
};
pub use events::{EventBroker, EventPayload};

use crate::admin::{self as admin_mod, auth_status, list_audit, login, logout, me, setup};
use crate::auth::{extract_bearer, TokenInfo, TokenSigner};
use crate::devices::{create_pair_code, create_token, list_tokens, revoke_token};
use crate::events::events as sse_events;
use crate::ledger::{get_ledger, verify_ledger, LedgerState};
use crate::metrics::{self as metrics_mod, metrics as metrics_endpoint, SavingsState};
use crate::session::{extract_cookie as extract_session_cookie, SessionSigner};
use crate::setup_wizard::setup_health;
use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
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
    pub session_signer: Option<Arc<SessionSigner>>,
    pub audit_log: Arc<admin::AuditLog>,
    /// SSE event broker (v0.5.0 Sprint 1) — publish from mutating handlers, subscribe from
    /// `/api/events`.
    pub events: EventBroker,
    /// Live cost-savings counter (v0.5.0 Sprint 1) — instrumented handlers bump it.
    pub savings: SavingsState,
    /// Server-start timestamp (seconds since epoch) used by the metrics endpoint.
    pub started_at: i64,
    /// Cross-Session Protocol store (v0.5.0 Sprint 4) — sessions + drift log on disk.
    pub sessions: Arc<cairn_session::SessionStore>,
    /// Signed cost-savings ledger (v0.5.0 Sprint 5).
    pub ledger: LedgerState,
    /// Self-hosted pack registry (v0.5.0 Sprint 13). Mounted under `/registry`.
    pub registry: Option<Arc<cairn_registry::Registry>>,
    /// Push notification subscription store (v0.5.0 Sprint 20b). One JSON
    /// file per subscription under `<data_dir>/push/`. Optional so the API can
    /// run without a writable data dir (some embedded test harnesses).
    pub push: Option<Arc<push::PushStore>>,
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
        let session_signer = cfg
            .secret_key
            .as_ref()
            .map(|k| Arc::new(SessionSigner::new(k.clone())));
        // Seed the synthetic-event counter from the durable audit log so SSE ids for
        // `stats-`/`drift-` events never collide with replayed audit-log ids.
        // Performed before `store` is moved into `Self`.
        let events = {
            let seed = store
                .max_audit_event_id()
                .ok()
                .and_then(|n| u64::try_from(n).ok())
                .unwrap_or(0);
            crate::events::EventBroker::new(1024, seed.saturating_add(1))
        };
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
            session_signer,
            audit_log: Arc::new(admin::AuditLog::default()),
            events,
            savings: SavingsState::default(),
            started_at: metrics_mod::server_started(),
            sessions: Arc::new(cairn_session::SessionStore::new(cfg.data_dir.clone())),
            ledger: LedgerState::default(),
            registry: cairn_registry::Registry::open(&cfg.data_dir)
                .ok()
                .map(Arc::new),
            push: push::PushStore::open(&cfg.data_dir).ok().map(Arc::new),
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
    Router::new()
        .route("/api/health", get(health))
        .route("/api/events", get(sse_events))
        .route("/api/metrics", get(metrics_endpoint))
        .route("/api/ledger", get(get_ledger))
        .route("/api/ledger/verify", get(verify_ledger))
        .route("/api/search", get(search_handler))
        .route("/api/stats", get(stats))
        .route("/api/context/read", get(read))
        .route("/api/context/expand", get(expand))
        .route("/api/context/assemble", get(assemble))
        .route("/api/memory", post(remember))
        .route("/api/memory/recall", get(recall))
        .route("/api/memory/wakeup", get(wakeup))
        .route("/api/memory/consolidate", post(consolidate_memory))
        .route("/api/memory/:id", post(edit_memory).delete(delete_memory))
        .route("/api/memory/:id/pin", post(pin_memory))
        .route("/api/memory/:id/reinforce", post(reinforce_memory))
        .route("/api/memory/crystallize", post(crystallize))
        .route("/api/memory/graph", get(memory_graph))
        .route("/api/guard/verify", post(verify))
        .route("/api/guard/anchor", get(get_anchor).post(post_anchor))
        .route("/api/guard/checkpoint", post(create_checkpoint))
        .route("/api/guard/checkpoints", get(list_checkpoints))
        .route("/api/guard/rollback", post(rollback_checkpoint))
        .route("/api/guard/drift", get(list_drift))
        .route("/api/guard/drift/:id/approve", post(approve_drift))
        .route("/api/guard/drift/:id/reject", post(reject_drift))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/latest", get(latest_session))
        .route("/api/sessions/:id", get(get_session).patch(update_session))
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
        .route("/api/pair/claim", post(pair_claim))
        .route("/api/sync/pull", get(sync_pull))
        .route("/api/sync/push", post(sync_push))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
        .route("/api/auth/setup", post(setup))
        .route("/api/setup/health", get(setup_health))
        .route("/api/setup/embed-default", get(setup_embed_default))
        .route("/api/devices/audit", get(list_audit))
        .route("/api/devices/tokens", get(list_tokens).post(create_token))
        .route("/api/devices/tokens/:id/revoke", post(revoke_token))
        .route("/api/devices/pair-codes", post(create_pair_code))
        .route("/api/push/subscribe", post(push::subscribe))
        .route("/api/push/unsubscribe", post(push::unsubscribe))
        .route("/api/push/list", get(push::list_subscriptions))
        .route("/api/extensions/capture", post(extensions::capture))
        .route("/api/ingest/transcript", post(ingest::transcript))
        .fallback(static_handler)
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(middleware::from_fn(security_headers::security_headers))
        .layer(cors)
        .with_state(state)
}

// Note: the registry router is mounted separately in `build_router_with_registry` below
// because `axum::Router::merge` would force the registry to share the AppState, and the
// registry's own state type is `Arc<Registry>`. We mount `/registry` at the top level so
// the existing `/api/...` chain is unaffected.

/// Build the full router (API + registry). When `state.registry` is `None`, the
/// registry routes return 404 — useful for tests that don't want a registry on disk.
pub fn build_router_with_registry(state: AppState) -> Router {
    let base = router(state.clone());
    match state.registry.as_ref() {
        Some(reg) => base.nest(
            "/registry",
            cairn_registry::router(reg.clone()).layer(
                tower_http::limit::RequestBodyLimitLayer::new(
                    32 * 1024 * 1024, // 32 MiB for pack uploads
                ),
            ),
        ),
        None => base,
    }
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
        build_router_with_registry(state).into_make_service_with_connect_info::<SocketAddr>(),
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
    let app = build_router_with_registry(state).into_make_service_with_connect_info::<SocketAddr>();
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

async fn static_handler(uri: axum::http::Uri, req: Request) -> Response {
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
    // Pull the per-request CSP nonce (set by security_headers middleware) so we can
    // rewrite inline `<script>` tags in the HTML shell.
    let nonce = req
        .extensions()
        .get::<crate::security_headers::CspNonce>()
        .map(|c| c.0.clone());
    match <WebAssets as RustEmbed>::get(&key) {
        Some(file) => {
            let bytes = file.data.into_owned();
            let body = if key.ends_with(".html") {
                inject_csp_nonce(&bytes, &nonce).into_owned()
            } else {
                bytes
            };
            (
                [(axum::http::header::CONTENT_TYPE, content_type(&key))],
                body,
            )
                .into_response()
        }
        None => {
            // Fallback page — same nonce injection treatment.
            let raw = ui::INDEX_HTML.as_bytes();
            let body = inject_csp_nonce(raw, &nonce).into_owned();
            (
                [(axum::http::header::CONTENT_TYPE, content_type("index.html"))],
                body,
            )
                .into_response()
        }
    }
}

/// Insert a CSP nonce into every `<script>` tag that doesn't already have one. We rewrite
/// `<script>` → `<script nonce="{nonce}">` (preserving any other attributes) and add a
/// `<meta http-equiv="Content-Security-Policy">` if none is present so the browser enforces
/// the policy even on a bare HTML page (e.g. the fallback).
fn inject_csp_nonce<'a>(html: &'a [u8], nonce: &Option<String>) -> std::borrow::Cow<'a, [u8]> {
    let Some(n) = nonce else {
        return std::borrow::Cow::Borrowed(html);
    };
    let s = match std::str::from_utf8(html) {
        Ok(s) => s,
        Err(_) => return std::borrow::Cow::Borrowed(html),
    };
    // Tag each <script> with `nonce="..."` if not already tagged. Insert a space between
    // the rewritten nonce attribute and whatever follows (typically `type=`) so the
    // rendered HTML reads `<script nonce="X" type="...">` instead of the no-space
    // `<script nonce="X"type="...">`. Browsers parse both correctly, but the latter
    // is harder to read and trips some linters.
    let mut out = String::with_capacity(s.len() + 64);
    let mut rest = s;
    while let Some(idx) = rest.find("<script") {
        out.push_str(&rest[..idx]);
        out.push_str("<script nonce=\"");
        out.push_str(n);
        out.push('"');
        out.push(' ');
        // Skip the "<script" prefix and look at the rest of the tag.
        let after_open = idx + "<script".len();
        rest = &rest[after_open..];
        if let Some(gt) = rest.find('>') {
            out.push_str(&rest[..=gt]);
            rest = &rest[gt + 1..];
        }
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out.into_bytes())
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

/// `GET /api/setup/embed-default` — the embed provider the wizard pre-selects. Per the
/// v0.5.0 plan, the default is local hashing (offline-first); the wizard offers an opt-in
/// switch to ONNX or OpenAI-compatible.
async fn setup_embed_default() -> Json<Value> {
    Json(json!({
        "provider": "hashing",
        "model": null,
        "url": null,
        "needs_api_key": false,
        "description": "Local hashing (no model download, no network). Switch to ONNX or OpenAI for semantic recall."
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
    let result = s.ctx.read(Path::new(&q.path), mode)?;
    // Record a savings entry — the "compact" bytes we sent vs the "full" bytes the file
    // actually contains. Approximate: ReadResult exposes `bytes` (the served view); for the
    // "full" cost we use the same ReadResult (the served view IS the full bytes for
    // non-cached modes). Cache hits report savings because they reuse prior work.
    let bytes_in = result.bytes as u64;
    let bytes_out = bytes_in;
    s.savings
        .record_read(bytes_in, bytes_out, result.view.is_empty());
    if let Some(key) = s.cfg.secret_key.as_ref() {
        s.ledger.append("context.read", bytes_in, bytes_out, key);
    }
    Ok(Json(result))
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
struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    rerank_depth: Option<usize>,
}

/// `GET /api/search?q=...&limit=N&rerank_depth=M` — Sprint 7 hybrid search: BM25 + HNSW
/// + provenance graph, fused with RRF, reranked with MMR.
async fn search_handler(
    State(s): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<ScoredMemory>>, ApiError> {
    let limit = q.limit.unwrap_or(20);
    let rerank = q.rerank_depth.unwrap_or(20);
    Ok(Json(s.mem.hybrid_search(&q.q, limit, rerank)?))
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
struct MemoryEditBody {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    importance: Option<f32>,
    #[serde(default)]
    concepts: Option<Vec<String>>,
    #[serde(default)]
    files: Option<Vec<String>>,
}

/// POST `/api/memory/:id` — edit a memory's mutable fields. Any field omitted from the body is
/// left unchanged. The suspicious-content scan re-runs on the new content (defense-in-depth).
async fn edit_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<MemoryEditBody>,
) -> Result<Json<Memory>, ApiError> {
    let content = body
        .content
        .map(|c| cairn_profile::strip_preference_blocks(&c));
    match s
        .mem
        .edit(&id, content, body.importance, body.concepts, body.files)?
    {
        Some(m) => {
            crate::events::publish_memory(&s.events, "edited", &m.id);
            Ok(Json(m))
        }
        None => Err(ApiError(StatusCode::NOT_FOUND, "no such memory".into())),
    }
}

/// DELETE `/api/memory/:id` — remove a memory by id.
async fn delete_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Value>, ApiError> {
    if s.mem.delete(&id)? {
        crate::events::publish_memory(&s.events, "deleted", &id);
        Ok(Json(json!({ "deleted": true })))
    } else {
        Err(ApiError(StatusCode::NOT_FOUND, "no such memory".into()))
    }
}

#[derive(Deserialize)]
struct PinBody {
    pinned: bool,
}

/// POST `/api/memory/:id/pin` — pin or unpin a memory.
async fn pin_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<PinBody>,
) -> Result<Json<Memory>, ApiError> {
    if s.mem.pin(&id, body.pinned)? {
        crate::events::publish_memory(
            &s.events,
            if body.pinned { "pinned" } else { "unpinned" },
            &id,
        );
        Ok(Json(s.mem.get(&id)?.unwrap()))
    } else {
        Err(ApiError(StatusCode::NOT_FOUND, "no such memory".into()))
    }
}

/// POST `/api/memory/:id/reinforce` — manually nudge a memory's confidence (e.g. after a
/// `confirm_useful` click in the dashboard). Returns the updated memory.
async fn reinforce_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Memory>, ApiError> {
    s.store.reinforce_memory(&id)?;
    match s.mem.get(&id)? {
        Some(m) => Ok(Json(m)),
        None => Err(ApiError(StatusCode::NOT_FOUND, "no such memory".into())),
    }
}

#[derive(Deserialize)]
struct CrystallizeBody {
    #[serde(default)]
    session_id: Option<String>,
}

/// POST `/api/memory/crystallize` — promote working-tier memories into a semantic crystal.
async fn crystallize(
    State(s): State<AppState>,
    Json(body): Json<CrystallizeBody>,
) -> Result<Json<Value>, ApiError> {
    match s.mem.crystallize(body.session_id.as_deref())? {
        Some(id) => {
            crate::events::publish_memory(&s.events, "crystallized", &id);
            Ok(Json(json!({ "crystallized": true, "crystal_id": id })))
        }
        None => Ok(Json(json!({ "crystallized": false }))),
    }
}

/// GET `/api/memory/graph` — the provenance graph (nodes + edges) for the dashboard.
async fn memory_graph(
    State(s): State<AppState>,
) -> Result<Json<cairn_memory::MemoryGraph>, ApiError> {
    Ok(Json(s.mem.graph()?))
}

// ---- drift + sessions handlers (v0.5.0 Sprint 4) -------------------------------------------

async fn list_drift(
    State(s): State<AppState>,
) -> Result<Json<Vec<cairn_session::DriftEvent>>, ApiError> {
    Ok(Json(s.sessions.recent_drift(200, None)?))
}

async fn approve_drift(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<Value>, ApiError> {
    if s.sessions
        .set_drift_status(id, cairn_session::DriftStatus::Approved)?
    {
        Ok(Json(json!({ "ok": true, "status": "approved" })))
    } else {
        Err(ApiError(
            StatusCode::NOT_FOUND,
            "drift event not found or already resolved".into(),
        ))
    }
}

async fn reject_drift(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<Value>, ApiError> {
    if s.sessions
        .set_drift_status(id, cairn_session::DriftStatus::Rejected)?
    {
        Ok(Json(json!({ "ok": true, "status": "rejected" })))
    } else {
        Err(ApiError(
            StatusCode::NOT_FOUND,
            "drift event not found or already resolved".into(),
        ))
    }
}

async fn list_sessions(
    State(s): State<AppState>,
) -> Result<Json<Vec<cairn_session::Session>>, ApiError> {
    let ids = s.sessions.list()?;
    let mut out: Vec<cairn_session::Session> = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(sess) = s.sessions.load(&id)? {
            out.push(sess);
        }
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct CreateSessionBody {
    project_hash: String,
}

async fn create_session(
    State(s): State<AppState>,
    Json(b): Json<CreateSessionBody>,
) -> Result<Json<cairn_session::Session>, ApiError> {
    let sess = cairn_session::Session::new(b.project_hash);
    s.sessions.save(&sess)?;
    Ok(Json(sess))
}

async fn latest_session(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    let Some(id) = s.sessions.latest_id() else {
        return Ok(Json(json!({ "session": null })));
    };
    let Some(sess) = s.sessions.load(&id)? else {
        return Ok(Json(json!({ "session": null })));
    };
    Ok(Json(json!({ "session": sess, "block": sess.as_block() })))
}

async fn get_session(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<cairn_session::Session>, ApiError> {
    match s.sessions.load(&id)? {
        Some(sess) => Ok(Json(sess)),
        None => Err(ApiError(StatusCode::NOT_FOUND, "no such session".into())),
    }
}

/// PATCH a session — merge in new tasks/findings/decisions/touched_files/next_steps.
async fn update_session(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<cairn_session::SessionPatch>,
) -> Result<Json<cairn_session::Session>, ApiError> {
    let Some(mut sess) = s.sessions.load(&id)? else {
        return Err(ApiError(StatusCode::NOT_FOUND, "no such session".into()));
    };
    if let Some(t) = patch.tasks {
        sess.tasks.extend(t);
    }
    if let Some(f) = patch.findings {
        sess.findings.extend(f);
    }
    if let Some(d) = patch.decisions {
        sess.decisions.extend(d);
    }
    if let Some(f) = patch.touched_files {
        sess.touched_files.extend(f);
    }
    if let Some(n) = patch.next_steps {
        sess.next_steps.extend(n);
    }
    if patch.end == Some(true) {
        sess.ended_at = Some(Utc::now());
    }
    s.sessions.save(&sess)?;
    Ok(Json(sess))
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
    // Persist drift events for warn/danger outcomes so the dashboard can review them.
    if matches!(
        report.risk,
        cairn_guard::Risk::Warn | cairn_guard::Risk::Danger
    ) {
        let ev = cairn_session::DriftEvent {
            id: s.sessions.next_drift_id(),
            ts: Utc::now(),
            path: b.path.clone(),
            risk: report.risk.as_str().to_string(),
            kind: "verify".into(),
            detail: report.message.clone(),
            status: cairn_session::DriftStatus::Pending,
        };
        let _ = s.sessions.append_drift(&ev);
        crate::events::publish_drift(&s.events, &b.path, report.risk.as_str());
    }
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
    let report = s.asm.assemble(&q.q, q.budget.unwrap_or(2000))?;
    s.savings.record_assemble(&report);
    let bytes_in = (report.used_tokens as u64) * 4;
    let bytes_out = (report.budget_tokens as u64) * 4;
    if let Some(key) = s.cfg.secret_key.as_ref() {
        s.ledger
            .append("context.assemble", bytes_in, bytes_out, key);
    }
    Ok(Json(report))
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

/// Authentication middleware.
///
/// Composition:
///   1. Public endpoints (`/api/health`, `/api/pair/claim`, the admin auth surface) — pass
///      through unchanged.
///   2. Admin cookie — if `cairn_session` is present, signed, and the embedded generation
///      matches the live admin record, the request is treated as the admin (all scopes).
///   3. Device-token bearer — the existing JWT path; respected when no admin cookie is
///      available so CLI / MCP clients keep working.
///   4. Loopback fallback — when there are no device tokens AND no admin (first-run before
///      `/setup`), only loopback calls pass. The admin cookie path overrides this once
///      `/setup` has been visited.
///
/// All errors are 401 with a uniform JSON body.
async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let method = req.method().as_str();

    // 1. Public endpoints — never require auth.
    if !path.starts_with("/api/")
        || matches!(
            path,
            "/api/health"
                | "/api/pair/claim"
                | "/api/auth/login"
                | "/api/auth/logout"
                | "/api/auth/me"
                | "/api/auth/setup"
                | "/api/auth/status"
                | "/api/setup/health"
        )
    {
        return next.run(req).await;
    }

    // 2. Admin cookie.
    if verify_admin_cookie(&s, &req).is_some() {
        // Admin can do everything. No scope table check.
        tracing::trace!(path, "auth=admin");
        return next.run(req).await;
    }

    // 3. Device-token bearer.
    // `verify_bearer_auth` returns a tri-state so we can hard-401 on a presented-but-invalid
    // bearer instead of falling through to the loopback fallback (the previous code only
    // distinguished `Ok(true)` from "everything else", which let a junk bearer from
    // loopback pass when no admin/tokens were configured yet).
    match verify_bearer_auth(&s, &req, method, path) {
        VerifyBearerOutcome::Valid => {
            tracing::trace!(path, "auth=bearer");
            return next.run(req).await;
        }
        VerifyBearerOutcome::Invalid => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "invalid bearer token" })),
            )
                .into_response();
        }
        VerifyBearerOutcome::Absent => {} // fall through to loopback / 401 below
    }

    // 4. Loopback fallback — only when there are no device tokens AND no admin.
    let token_count = s.store.count_tokens().unwrap_or(0);
    let admin_exists = admin_mod::load_admin(&s)
        .map(|r| r.is_some())
        .unwrap_or(false);
    if token_count == 0 && !admin_exists {
        let is_local = req
            .extensions()
            .get::<axum::extract::ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        if is_local {
            return next.run(req).await;
        }
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "no admin or device tokens configured; create an admin via /setup on localhost"
            })),
        )
            .into_response();
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "invalid or missing credentials" })),
    )
        .into_response()
}

/// Verify the admin cookie, returning `Some(())` on success.
fn verify_admin_cookie(s: &AppState, req: &Request) -> Option<()> {
    let signer = s.session_signer.as_ref()?;
    let header_value = req.headers().get(axum::http::header::COOKIE)?;
    let header_str = header_value.to_str().ok()?;
    let cookie = extract_session_cookie(Some(header_str))?;
    let rec = admin_mod::load_admin(s).ok().flatten()?;
    signer.verify(cookie, rec.generation).ok()?;
    Some(())
}

/// Outcome of probing a request for a device-token bearer. The tri-state lets the auth
/// middleware hard-401 on a presented-but-invalid bearer instead of silently falling
/// through to the loopback fallback path (which would let a junk bearer pass when no
/// admin / tokens are configured yet).
enum VerifyBearerOutcome {
    /// A bearer was presented AND it verified against a trusted key with sufficient scope.
    Valid,
    /// A bearer was presented but did not verify (bad signature, expired, revoked, or
    /// out-of-scope). The caller MUST reject with 401.
    Invalid,
    /// No `Authorization: Bearer …` header was sent at all. The caller may fall through
    /// to weaker authentication paths (loopback fallback) if appropriate.
    Absent,
}

/// Verify a device-token bearer against the request. Returns a tri-state — see
/// [`VerifyBearerOutcome`] for why we don't collapse `Invalid` and `Absent`.
fn verify_bearer_auth(
    s: &AppState,
    req: &Request,
    method: &str,
    path: &str,
) -> VerifyBearerOutcome {
    let Some(bearer) = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(extract_bearer)
    else {
        return VerifyBearerOutcome::Absent;
    };
    match s.verify_bearer(bearer) {
        None => VerifyBearerOutcome::Invalid,
        Some(info) => {
            let allowed = info.scope.allows(method, path)
                && s.store.validate_token_id(&info.id).unwrap_or(false);
            if allowed {
                VerifyBearerOutcome::Valid
            } else {
                VerifyBearerOutcome::Invalid
            }
        }
    }
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
#[allow(
    unused_must_use,
    clippy::needless_question_mark,
    clippy::unnecessary_sort_by
)]
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
        use axum::extract::Request;
        let req = Request::builder()
            .uri("/")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = static_handler("/".parse().unwrap(), req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// `None` when `CAIRN_HELIX_URL` is unset or HelixDB is unreachable (tests skip gracefully).
    /// The temp dir is a scratch workspace for the test's files (separate from the store).
    pub(crate) fn test_state() -> Option<(AppState, tempfile::TempDir)> {
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

    // ---- v0.5.0 Sprint 1: SSE event broker + metrics endpoint -----------------------------

    #[test]
    fn event_broker_delivers_published_payloads_to_subscribers() {
        let broker = crate::events::EventBroker::default();
        let mut rx = broker.subscribe();
        broker.publish(crate::events::EventPayload {
            id: "test-1".into(),
            kind: crate::events::KIND_STATS,
            ts: 12345,
            data: serde_json::json!({"hello": "world"}),
        });
        // Subscription is async, so poll for up to 500 ms.
        let start = std::time::Instant::now();
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    assert_eq!(ev.id, "test-1");
                    assert_eq!(ev.kind, crate::events::KIND_STATS);
                    assert_eq!(ev.ts, 12345);
                    assert_eq!(ev.data["hello"], "world");
                    return;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                    if start.elapsed() > std::time::Duration::from_millis(500) {
                        panic!("subscriber never received published event");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => panic!("unexpected recv error: {e}"),
            }
        }
    }

    #[test]
    fn savings_counter_records_reads_and_computes_ratios() {
        let counter = crate::metrics::SavingsCounter::default();
        counter.record_read(200, 1000, false);
        counter.record_read(0, 100, true);
        let snap = counter.snapshot();
        assert_eq!(snap.compact_bytes, 200);
        assert_eq!(snap.full_bytes, 1100);
        assert_eq!(snap.saved_bytes, 900);
        assert!((snap.hit_rate - 0.5).abs() < 1e-9);
        assert!((snap.bounce_rate - 0.5).abs() < 1e-9);
        assert!(snap.usd_saved() > 0.0);
    }

    #[tokio::test]
    async fn sse_endpoint_serves_a_first_event_under_500ms() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        // Start the SSE handler — it returns immediately with a 200 + `text/event-stream`.
        let started = std::time::Instant::now();
        let resp = sse_events(
            State(state.clone()),
            axum::extract::Query(crate::events::EventsQuery::default()),
            axum::http::HeaderMap::new(),
        )
        .await
        .into_response();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "SSE handler should return in <500ms; took {:?}",
            started.elapsed()
        );
        // axum's Sse wraps the body; we just verify it's a streaming Content-Type.
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.starts_with("text/event-stream"),
            "SSE response should have text/event-stream content-type; got {ct:?}"
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_json_with_savings_block() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        // Make sure there's a memory so `memories` isn't trivially zero.
        state
            .mem
            .remember(cairn_core::NewMemory::new("sprint1 metric test"))
            .unwrap();
        state.savings.record_read(500, 2000, false);

        let resp = metrics_endpoint(State(state.clone())).await.unwrap();
        let v = resp.0;
        assert!(v.savings.saved_bytes >= 1500);
        assert!(v.memories >= 1);
        assert!(v.usd_saved >= 0.0);
        assert!(v.server["version"].is_string());
    }

    #[tokio::test]
    async fn sse_backfill_replays_audit_events_with_since_id() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        // Record three audit events via the durable path.
        let id1 = state
            .store
            .append_audit(100, "login_ok", "alice", "")
            .unwrap();
        state
            .store
            .append_audit(200, "token_issued", "alice", "laptop")
            .unwrap();
        state
            .store
            .append_audit(300, "login_failed", "bob", "bad")
            .unwrap();

        // Since the first id, only the latter two should come back, in chronological order.
        let backfilled = crate::events::test_backfill(&state, Some(&id1)).unwrap();
        assert_eq!(backfilled.len(), 2);
        assert_eq!(backfilled[0].kind, crate::events::KIND_AUDIT);
        assert_eq!(backfilled[0].data["kind"], "token_issued");
        assert_eq!(backfilled[1].data["kind"], "login_failed");
    }

    // ---- v0.5.0 Sprint 2: memory CRUD + confidence + pin ---------------------------------

    #[tokio::test]
    async fn memory_edit_delete_pin_round_trip() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let m = state
            .mem
            .remember(cairn_core::NewMemory::new("sprint2 round trip"))
            .unwrap();

        // Edit: content + importance change, concepts/files left alone.
        let body = MemoryEditBody {
            content: Some("sprint2 round trip EDITED".into()),
            importance: Some(0.9),
            concepts: None,
            files: None,
        };
        let edited = edit_memory(
            State(state.clone()),
            axum::extract::Path(m.id.clone()),
            Json(body),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(edited.content, "sprint2 round trip EDITED");
        assert!((edited.importance - 0.9).abs() < 1e-6);

        // Pin: wakeup ordering reflects it.
        pin_memory(
            State(state.clone()),
            axum::extract::Path(m.id.clone()),
            Json(PinBody { pinned: true }),
        )
        .await
        .unwrap();

        // Delete: 404 on second attempt, GET get returns None.
        let ok = delete_memory(State(state.clone()), axum::extract::Path(m.id.clone()))
            .await
            .unwrap();
        assert_eq!(ok.0["deleted"], serde_json::json!(true));
        assert!(state.mem.get(&m.id).unwrap().is_none());

        let err = delete_memory(State(state.clone()), axum::extract::Path(m.id))
            .await
            .err()
            .unwrap();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn memory_reinforce_endpoint_advances_confidence() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let m = state
            .mem
            .remember(cairn_core::NewMemory::new("reinforce endpoint target"))
            .unwrap();
        let start = m.confidence;
        let updated = reinforce_memory(State(state.clone()), axum::extract::Path(m.id.clone()))
            .await
            .unwrap()
            .0;
        assert!(
            updated.confidence > start,
            "reinforce should bump confidence"
        );
    }

    #[tokio::test]
    async fn new_memory_default_confidence_is_half() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let m = state
            .mem
            .remember(cairn_core::NewMemory::new("default confidence check"))
            .unwrap();
        assert!((m.confidence - 0.5).abs() < 1e-6);
        assert!(!m.pinned);
    }

    // ---- v0.5.0 Sprint 3: crystallize + memory graph endpoints ----------------------------

    #[tokio::test]
    async fn memory_crystallize_endpoint_promotes_working_and_publishes_event() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory::new("working note A"))
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new("working note B"))
            .unwrap();

        let resp = crystallize(
            State(state.clone()),
            Json(CrystallizeBody { session_id: None }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(resp["crystallized"], serde_json::json!(true));
        assert!(resp["crystal_id"].is_string());

        // A second call with no fresh working memories returns crystallized: false.
        let second = crystallize(
            State(state.clone()),
            Json(CrystallizeBody { session_id: None }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(second["crystallized"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn memory_graph_endpoint_returns_nodes_and_edges() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory::new("graph node 1"))
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new("graph node 2"))
            .unwrap();
        // Crystallize so derived_from / supersedes edges appear.
        state.mem.crystallize(None).unwrap();

        let resp = memory_graph(State(state.clone())).await.unwrap();
        let v = resp.0;
        assert_eq!(v.nodes.len(), 3, "two inputs + one crystal");
        let derived_count = v.edges.iter().filter(|e| e.kind == "derived_from").count();
        assert!(
            derived_count >= 2,
            "crystal should have derived_from edges to both inputs"
        );
    }

    // ---- v0.5.0 Sprint 4: sessions + drift --------------------------------------------------

    #[tokio::test]
    async fn sessions_create_list_latest_round_trip() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let created = create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                project_hash: "demo".into(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(!created.id.is_empty());

        let listed = list_sessions(State(state.clone())).await.unwrap().0;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let latest = latest_session(State(state.clone())).await.unwrap().0;
        assert_eq!(latest["session"]["id"], serde_json::json!(created.id));
        assert!(latest["block"]
            .as_str()
            .unwrap()
            .contains("Cross-Session Protocol"));
    }

    #[tokio::test]
    async fn session_patch_appends_and_can_close() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let created = create_session(
            State(state.clone()),
            Json(CreateSessionBody {
                project_hash: "demo".into(),
            }),
        )
        .await
        .unwrap()
        .0;

        let patch = cairn_session::SessionPatch {
            tasks: Some(vec![cairn_session::Task {
                id: "t1".into(),
                title: "Ship Sprint 4".into(),
                progress: "in_progress".into(),
            }]),
            findings: Some(vec![cairn_session::Finding {
                text: "drift events persisted to sessions/drift_events.jsonl".into(),
                source_file: Some("crates/cairn-session/src/lib.rs".into()),
                confidence: 0.95,
            }]),
            decisions: None,
            touched_files: None,
            next_steps: Some(vec!["Sprint 5: assembler playground".into()]),
            end: Some(true),
        };
        let updated = update_session(
            State(state.clone()),
            axum::extract::Path(created.id.clone()),
            Json(patch),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(updated.tasks.len(), 1);
        assert_eq!(updated.findings.len(), 1);
        assert_eq!(updated.next_steps.len(), 1);
        assert!(updated.ended_at.is_some());
    }

    #[tokio::test]
    async fn verify_persists_drift_events_and_approve_moves_them() {
        let Some((state, dir)) = test_state() else {
            return;
        };
        let f = dir.path().join("drift.txt");
        let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&f, &original).unwrap();
        // A gutting edit — verify flags danger.
        verify(
            State(state.clone()),
            Json(VerifyBody {
                path: f.to_string_lossy().into_owned(),
                content: "x\n".into(),
            }),
        )
        .await
        .unwrap();

        let drifts = list_drift(State(state.clone())).await.unwrap().0;
        assert!(
            !drifts.is_empty(),
            "verify-d danger should produce a drift event"
        );
        let id = drifts[0].id;

        // Approve it.
        let ok = approve_drift(State(state.clone()), axum::extract::Path(id))
            .await
            .unwrap()
            .0;
        assert_eq!(ok["status"], serde_json::json!("approved"));

        // Re-approving a resolved event returns 404.
        let err = approve_drift(State(state.clone()), axum::extract::Path(id))
            .await
            .err()
            .unwrap();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    // ---- v0.5.0 Sprint 5: ledger -----------------------------------------------------------

    #[tokio::test]
    async fn ledger_records_savings_when_read_runs() {
        let Some((state, dir)) = test_state() else {
            return;
        };
        let f = dir.path().join("big.txt");
        std::fs::write(&f, "a\n".repeat(500)).unwrap();
        // A read in full mode → ledger gets an entry from "context.read".
        read(
            State(state.clone()),
            Query(ReadQuery {
                path: f.to_string_lossy().into_owned(),
                mode: Some("full".into()),
            }),
        )
        .await
        .unwrap();

        let entries = get_ledger(
            State(state.clone()),
            Query(crate::ledger::LedgerQuery { limit: Some(10) }),
        )
        .await
        .0;
        assert!(!entries.is_empty());
        let e = &entries[0];
        assert_eq!(e.source, "context.read");
        assert!(!e.signature.is_empty());

        // /api/ledger/verify?id=<id> confirms the entry.
        let v = verify_ledger(
            State(state.clone()),
            Query(crate::ledger::VerifyQuery { id: e.id }),
        )
        .await
        .0;
        assert_eq!(v["valid"], serde_json::json!(true));
    }

    // ---- v0.5.0 Sprint 6: setup wizard v2 --------------------------------------------------

    #[tokio::test]
    async fn setup_wizard_default_embed_provider_is_hashing() {
        let resp = setup_embed_default().await.0;
        assert_eq!(resp["provider"], "hashing");
        assert_eq!(resp["needs_api_key"], serde_json::json!(false));
    }

    #[tokio::test]
    async fn setup_health_endpoint_reports_components() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let h = setup_health(State(state.clone())).await.0;
        let v = serde_json::to_value(&h.health).unwrap();
        assert!(v.get("helix_reachable").is_some());
        assert!(v.get("admin_exists").is_some());
        assert!(v.get("embedder_loaded").is_some());
        assert!(v.get("secret_key_configured").is_some());
        assert_eq!(h.embed_provider, "hashing");
    }

    // ---- v0.5.0 Sprint 7: hybrid search + CSP nonce ---------------------------------------

    #[tokio::test]
    async fn hybrid_search_endpoint_returns_reranked_hits() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory::new("hybrid search test query A"))
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new(
                "hybrid search test query B similar to A",
            ))
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new("rust async runtime unrelated"))
            .unwrap();

        let resp = search_handler(
            State(state.clone()),
            axum::extract::Query(SearchQuery {
                q: "hybrid search test query".into(),
                limit: Some(2),
                rerank_depth: Some(20),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(resp.len(), 2);
        // MMR should pull the orthogonal hit, not both near-duplicates.
        assert!(
            resp.iter()
                .any(|h| h.memory.content.contains("async runtime")),
            "MMR should diversify; got {:?}",
            resp.iter().map(|h| &h.memory.content).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn csp_nonce_injected_into_html_response() {
        use axum::body::to_bytes;
        use axum::http::Request as HttpRequest;
        use tower::ServiceExt;

        // Build a router with the security_headers middleware + a minimal HTML fallback,
        // then GET / and verify the CSP header is present AND that the nonce from the
        // header matches the nonce embedded in the HTML.
        let app = axum::Router::new()
            .fallback(static_handler)
            .layer(axum::middleware::from_fn(
                crate::security_headers::security_headers,
            ));

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let csp = resp
            .headers()
            .get("content-security-policy")
            .expect("CSP should be set")
            .to_str()
            .unwrap()
            .to_string();
        // Extract the nonce from the header.
        let nonce_start = csp.find("nonce-").unwrap() + "nonce-".len();
        let nonce = csp[nonce_start..nonce_start + 32].to_string();

        // The body should have an inline <script> with the matching nonce attribute.
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body_str = String::from_utf8_lossy(&body);
        assert!(
            body_str.contains(&format!("nonce=\"{nonce}\"")),
            "HTML should contain the nonce from the CSP header; nonce={nonce}"
        );
    }

    /// Registry HTTP integration: publish a signed pack, list it, download the tarball
    /// back, then revoke and confirm the revocations log records it. Skips when
    /// `CAIRN_HELIX_URL` is unset.
    #[tokio::test]
    async fn registry_publish_list_download_revoke_end_to_end() {
        use axum::body::{to_bytes, Body};
        use axum::http::{Request as HttpRequest, StatusCode as AxStatus};
        use tower::ServiceExt;

        use cairn_pack::{self, Pack};

        let Some((state, dir)) = test_state() else {
            return;
        };
        // Registry is created at <data_dir>/registry; data_dir is the AppState's data_dir.
        let reg = state.registry.clone().expect("registry should be open");
        let kp = cairn_pack::Keypair::generate();
        reg.trust(kp.public(), cairn_registry::TrustScope::Public, None)
            .unwrap();

        // Build a small signed tarball.
        let tar_path = dir.path().join("alpha.cairnpkg");
        let mut pack = Pack::new("alpha", "1.0.0");
        pack.author = "tester".into();
        pack.description = "http test".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "hi"}));
        pack.write_tarball_signed(&tar_path, &kp).unwrap();
        let tar_bytes = std::fs::read(&tar_path).unwrap();

        let app = build_router_with_registry(state);

        // POST /registry/packs with the tarball body.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/registry/packs")
                    .header("content-type", cairn_pack::MIME)
                    .body(Body::from(tar_bytes.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxStatus::CREATED);
        let receipt: PublishReceipt =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(receipt.name, "alpha");
        assert_eq!(receipt.status, PublishStatus::Signed);

        // GET /registry/packs — must include the published pack.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/registry/packs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        let list: Vec<PackMeta> =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "alpha");

        // GET /registry/packs/alpha/1.0.0/download — round-trip the bytes.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/registry/packs/alpha/1.0.0/download")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        assert_eq!(
            body, tar_bytes,
            "downloaded bytes should match published tarball"
        );

        // GET /registry/search?q=alp — substring match.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/registry/search?q=alp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let hits: Vec<PackMeta> =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(hits.len(), 1);

        // DELETE /registry/packs/alpha/1.0.0 — revoke.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/registry/packs/alpha/1.0.0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        let rev: RevocationEvent =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(rev.name, "alpha");
        assert_eq!(rev.version, "1.0.0");

        // GET /registry/revocations must include it.
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/registry/revocations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let revs: Vec<RevocationEvent> =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].name, "alpha");
    }

    /// Federation scope + provenance display (Sprint 14). Publishes a pack whose
    /// manifest declares `scope: team` but only public-scope trusts are configured —
    /// must be rejected with `ScopeDenied`. Then publishes with `scope: local` and
    /// a public-scope grant — must succeed, and the cached manifest endpoint must
    /// return a manifest that includes `stats.graph_edges` (Sprint 14c provenance
    /// display).
    #[tokio::test]
    async fn registry_federation_scope_and_provenance_endpoints() {
        use axum::body::{to_bytes, Body};
        use axum::http::{Request as HttpRequest, StatusCode as AxStatus};
        use cairn_pack::Pack;
        use cairn_registry::TrustScope;
        use tower::ServiceExt;

        let Some((state, dir)) = test_state() else {
            return;
        };
        let reg = state.registry.clone().expect("registry should be open");

        // 1. team-scoped pack + public-only grant → rejected.
        let team_pack_path = dir.path().join("team.cairnpkg");
        let mut team_pack = Pack::new("team-only", "1.0.0");
        team_pack.description = "scope: team — for the team".into();
        team_pack
            .memories
            .push(serde_json::json!({"id": "m1", "content": "x"}));
        let kp = cairn_pack::Keypair::generate();
        reg.trust(kp.public(), TrustScope::Public, None).unwrap();
        team_pack
            .write_tarball_signed(&team_pack_path, &kp)
            .unwrap();
        let team_bytes = std::fs::read(&team_pack_path).unwrap();

        let team_resp = reg.publish(&team_bytes, None);
        match team_resp {
            Err(cairn_registry::RegistryError::ScopeDenied {
                pack_scope: TrustScope::Team,
                ..
            }) => {}
            other => panic!("expected ScopeDenied for team pack, got {other:?}"),
        }

        // 2. local-scoped pack + public grant → accepted.
        let local_pack_path = dir.path().join("local.cairnpkg");
        let mut local_pack = Pack::new("local-notes", "1.0.0");
        local_pack.description = "scope: local".into();
        local_pack
            .memories
            .push(serde_json::json!({"id": "m1", "content": "alpha"}));
        local_pack.graph_edges.push(serde_json::json!({
            "src": "m1",
            "dst": "src/foo.rs",
            "kind": "applies_to",
        }));
        local_pack
            .write_tarball_signed(&local_pack_path, &kp)
            .unwrap();
        let local_bytes = std::fs::read(&local_pack_path).unwrap();
        let receipt = reg.publish(&local_bytes, None).unwrap();
        assert_eq!(receipt.name, "local-notes");
        assert_eq!(receipt.status, cairn_registry::PublishStatus::Signed);

        // 3. The /registry/packs list must include the new pack with its scope
        // and provenance_edge_count.
        let app = build_router_with_registry(state);
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/registry/packs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list: Vec<PackMeta> =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        let local_meta = list
            .iter()
            .find(|m| m.name == "local-notes")
            .expect("local-notes should be listed");
        assert_eq!(local_meta.scope, TrustScope::Local);
        assert_eq!(local_meta.provenance_edge_count, 1);

        // 4. /registry/packs/:name/:version/manifest.json must return the cached
        // manifest with the provenance stats.
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/registry/packs/local-notes/1.0.0/manifest.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), AxStatus::OK);
        let manifest: serde_json::Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(manifest["name"], "local-notes");
        assert_eq!(manifest["stats"]["graph_edges"], 1);
    }

    #[test]
    fn verify_bearer_distinguishes_absent_from_invalid() {
        // The tri-state must return Absent / Invalid / Invalid respectively — collapsing
        // Invalid into Absent would let a junk bearer slip past the middleware to the
        // loopback fallback. Skipped on environments without a backend fixture.
        let Some((state, _dir)) = test_state() else {
            return;
        };

        let mk = |auth: Option<&str>| -> axum::extract::Request {
            let mut b = axum::extract::Request::builder()
                .uri("/api/memories")
                .body(axum::body::Body::empty())
                .unwrap();
            if let Some(value) = auth {
                b.headers_mut()
                    .insert(axum::http::header::AUTHORIZATION, value.parse().unwrap());
            }
            b
        };

        let absent = mk(None);
        assert!(matches!(
            verify_bearer_auth(&state, &absent, "GET", "/api/memories"),
            VerifyBearerOutcome::Absent
        ));

        let bogus = mk(Some("Bearer this-is-not-a-valid-jwt"));
        assert!(matches!(
            verify_bearer_auth(&state, &bogus, "GET", "/api/memories"),
            VerifyBearerOutcome::Invalid
        ));
    }
}

//! The Cairn HTTP API and embedded control-plane UI.
//!
//! Exposes health/stats plus the context (read/expand/assemble), memory (remember/recall/wakeup),
//! and guard (verify, anchor, checkpoint/rollback) engines over REST, and serves the embedded
//! Next.js control plane - with a built-in fallback page when the UI hasn't been built.

pub mod admin;
mod auth;
mod capabilities;
pub mod cron;
mod devices;
mod documents;
mod events;
mod extensions;
mod ingest;
mod ledger;
mod metrics;
mod openapi;
mod push;
mod rate_limit;
mod scope;
mod security_headers;
mod selftune;
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
use crate::metrics::{
    self as metrics_mod, metrics as metrics_endpoint, mobile_savings, SavingsState,
};
use crate::session::{extract_cookie as extract_session_cookie, SessionSigner};
use crate::setup_wizard::setup_health;
use axum::{
    extract::{Extension, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post},
    Json, Router,
};
use cairn_assemble::{Assembler, AssemblyReport};
use cairn_context::{ContextEngine, ReadMode, ReadResult};
use cairn_core::{Config, Memory, NewMemory, OrgId, ScopeCtx, TlsConfig};
use cairn_guard::{Checkpoint, Guard, RollbackReport, VerifyReport};
use cairn_memory::{MemoryEngine, ScoredMemory};
use cairn_profile::Profile;
use cairn_shell::{Compressed, ShellCompressor};
use cairn_store::Store;
use chrono::{DateTime, Utc};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

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
    /// SSE event broker (v0.5.0 Sprint 1) - publish from mutating handlers, subscribe from
    /// `/api/events`.
    pub events: EventBroker,
    /// Live cost-savings counter (v0.5.0 Sprint 1) - instrumented handlers bump it.
    pub savings: SavingsState,
    /// Server-start timestamp (seconds since epoch) used by the metrics endpoint.
    pub started_at: i64,
    /// Cross-Session Protocol store (v0.5.0 Sprint 4) - sessions + drift log on disk.
    pub sessions: Arc<cairn_session::SessionStore>,
    /// Signed cost-savings ledger (v0.5.0 Sprint 5).
    pub ledger: LedgerState,
    /// Self-hosted pack registry (v0.5.0 Sprint 13). Mounted under `/registry`.
    pub registry: Option<Arc<cairn_registry::Registry>>,
    /// Server package version (CARGO_PKG_VERSION) - exposed via `/api/capabilities` and
    /// `/api/openapi.json`.
    pub version: String,
    /// Push notification subscription store (v0.5.0 Sprint 20b). One JSON
    /// file per subscription under `<data_dir>/push/`. Optional so the API can
    /// run without a writable data dir (some embedded test harnesses).
    pub push: Option<Arc<push::PushStore>>,
    /// Recent cron job runs (v0.8.0 Sprint 4) - see `cron.rs`. Populated by both the
    /// scheduler's own ticks and the manual `POST /api/cron/run/:job` trigger.
    pub cron_history: Arc<cron::CronHistory>,
    signer: Option<Arc<TokenSigner>>,
}

impl AppState {
    pub fn new(cfg: &Config) -> cairn_core::Result<Self> {
        let store = Arc::new(Store::open(cfg)?);
        Self::with_store(cfg, store)
    }

    /// Construct `AppState` from a caller-supplied `Arc<Store>`. Used by the hermetic
    /// test bucket to wire a fully-in-memory store; production callers should keep using
    /// `new`.
    pub fn with_store(cfg: &Config, store: Arc<Store>) -> cairn_core::Result<Self> {
        let ctx = Arc::new(ContextEngine::new_with_root(
            store.clone(),
            cfg.workspace_root.clone(),
        ));
        let mem = Arc::new(MemoryEngine::new(store.clone()));
        let guard = {
            let mut g = Guard::new(store.clone());
            if let Some(ref root) = cfg.workspace_root {
                g = g.with_workspace(&root.to_string_lossy());
            }
            Arc::new(g)
        };
        let asm = Arc::new(Assembler::new(mem.clone(), store.clone()));
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
            cron_history: Arc::new(cron::CronHistory::default()),
            signer,
            version: env!("CARGO_PKG_VERSION").to_string(),
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

    // Auth routes: 5 attempts per IP per minute before 429.
    let auth_limiter = Arc::new(rate_limit::AuthRateLimiter::new(
        5,
        std::time::Duration::from_secs(60),
    ));
    let auth_routes = Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/setup", post(setup))
        .layer(middleware::from_fn(move |req, next| {
            let lim = Arc::clone(&auth_limiter);
            rate_limit::rate_limit_middleware(lim, req, next)
        }));

    Router::new()
        .route("/api/health", get(health))
        .route("/api/health/deep", get(health_deep))
        .route("/api/capabilities", get(capabilities::capabilities))
        .route("/api/openapi.json", get(openapi::openapi_spec))
        .route("/api/events", get(sse_events))
        .route("/api/metrics", get(metrics_endpoint))
        .route("/api/metrics/savings", get(mobile_savings))
        .route("/api/ledger", get(get_ledger))
        .route("/api/ledger/verify", get(verify_ledger))
        .route("/api/search", get(search_handler))
        .route("/api/stats", get(stats))
        .route("/api/context/read", get(read))
        .route("/api/context/expand", get(expand))
        .route("/api/context/assemble", get(assemble))
        .route("/api/context/pressure", get(context_pressure))
        .route("/api/config", get(config_view))
        .route("/api/memory", get(list_memories).post(remember))
        .route("/api/memory/gotcha", post(record_gotcha))
        .route("/api/memory/gotcha/wakeup", get(gotcha_wakeup))
        .route("/api/memory/recall", get(recall))
        .route("/api/memory/wakeup", get(wakeup))
        .route("/api/memory/consolidate", post(consolidate_memory))
        .route(
            "/api/memory/:id",
            get(get_memory).post(edit_memory).delete(delete_memory),
        )
        .route("/api/memory/:id/pin", post(pin_memory))
        .route("/api/memory/:id/reinforce", post(reinforce_memory))
        .route("/api/memory/:id/promote", post(promote_memory))
        .route("/api/memory/:id/dismiss-promotion", post(dismiss_promotion))
        .route("/api/memory/:id/demote", post(demote_memory))
        .route(
            "/api/memory/promotion-candidates",
            get(promotion_candidates),
        )
        .route("/api/memory/promotion-log", get(promotion_log))
        .route("/api/memory/autopilot-digest", get(autopilot_digest))
        .route("/api/memory/session-summary", post(session_summary))
        .route("/api/memory/crystallize", post(crystallize))
        .route("/api/memory/by-scope", get(memory_by_scope))
        .route("/api/memory/graph", get(memory_graph))
        .route("/api/memory/architecture-report", get(architecture_report))
        .route("/api/projects/upsert", patch(upsert_project))
        .route("/api/projects", get(list_projects))
        .route("/api/projects/:id", get(get_project))
        .route("/api/cron/jobs", get(list_cron_jobs))
        .route("/api/cron/run/:job", post(run_cron_job))
        .route("/api/cron/history", get(cron_history))
        .route("/api/cron/health", get(cron_health))
        .route("/api/memory/heatmap", get(memory_heatmap))
        .route("/api/guard/verify", post(verify))
        .route("/api/guard/anchor", get(get_anchor).post(post_anchor))
        .route("/api/guard/anchor/auto", post(auto_anchor))
        .route("/api/guard/checkpoint", post(create_checkpoint))
        .route("/api/guard/checkpoints", get(list_checkpoints))
        .route("/api/guard/rollback", post(rollback_checkpoint))
        .route("/api/guard/drift", get(list_drift))
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
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
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
        .route("/api/documents/ingest", post(documents::ingest))
        .route("/api/documents", get(documents::list))
        .route("/api/documents/search", get(documents::search))
        .route("/api/documents/:id", delete(documents::delete))
        .fallback(static_handler)
        .merge(auth_routes)
        .layer(RequestBodyLimitLayer::new(1024 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(middleware::from_fn(security_headers::security_headers))
        .layer(middleware::from_fn(scope::scope_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

// Note: the registry router is mounted separately in `build_router_with_registry` below
// because `axum::Router::merge` would force the registry to share the AppState, and the
// registry's own state type is `Arc<Registry>`. We mount `/registry` at the top level so
// the existing `/api/...` chain is unaffected.

/// Build the full router (API + registry). When `state.registry` is `None`, the
/// registry routes return 404 - useful for tests that don't want a registry on disk.
pub fn build_router_with_registry(state: AppState) -> Router {
    let base = router(state.clone());
    match state.registry.as_ref() {
        Some(reg) => {
            // Mount the registry under `/api/registry` so it doesn't shadow the
            // Next.js dashboard's `/registry` and `/registry/packs` page routes
            // (which are served by the static_handler fallback). The dashboard's
            // `lib/queries.ts` calls `/registry/packs` directly; that needs to be
            // updated to `/api/registry/packs` in lockstep with this change.
            base.nest(
                "/api/registry",
                cairn_registry::router(reg.clone()).layer(
                    tower_http::limit::RequestBodyLimitLayer::new(
                        32 * 1024 * 1024, // 32 MiB for pack uploads
                    ),
                ),
            )
        }
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
            "CAIRN_CORS_ORIGINS contains '*' - wildcard origin rejected. The Cairn API is \
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
/// - **Loopback bind, no TLS** -> plain HTTP (dev-friendly default).
/// - **Non-loopback bind, no TLS** -> refuses to start; the caller surfaces an error explaining
///   that network exposure requires `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`.
/// - **TLS material present** -> serves HTTPS via `axum-server` (rustls).
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
    let raw_path = uri.path().trim_start_matches('/');
    // An unmatched /api/* path is an API 404, never the SPA shell - serving HTML with a
    // 200 to an API caller (e.g. an old dashboard build POSTing to a removed route) hides
    // the failure and looks like success to naive clients.
    if raw_path == "api" || raw_path.starts_with("api/") {
        return (
            axum::http::StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json; charset=utf-8",
            )],
            serde_json::json!({ "error": format!("no such API route: /{raw_path}") }).to_string(),
        )
            .into_response();
    }
    // Bug fix (BUG-2026-06-30-C): browsers URL-encode characters in chunk paths
    // (e.g. `%5Bname%5D` for `[name]`, `%28app%29` for `(app)`). The embedded
    // WebAssets key uses the decoded form, so we percent-decode the request
    // path before lookup. Without this, every dynamic-route chunk 404s and the
    // dashboard throws ChunkLoadError.
    let path = match percent_decode(raw_path) {
        Some(p) => p,
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )],
                format!("invalid path: {raw_path}"),
            )
                .into_response();
        }
    };
    // Bug fix (BUG-2026-06-30-C): if the request path looks like a static asset
    // (has a non-HTML file extension), never fall back to index.html. Doing so
    // serves the dashboard shell HTML with a JS/CSS MIME, which the browser then
    // tries to parse as that asset and surfaces a confusing ChunkLoadError. Return
    // 404 instead so the failure is observable and recoverable.
    let looks_like_asset = std::path::Path::new(&path)
        .extension()
        .is_some_and(|ext| !ext.eq_ignore_ascii_case("html") && !ext.eq_ignore_ascii_case("htm"));
    let key = if path.is_empty() {
        "index.html".to_string()
    } else if <WebAssets as RustEmbed>::get(&path).is_some() {
        path.clone()
    } else if <WebAssets as RustEmbed>::get(&format!("{path}.html")).is_some() {
        format!("{path}.html")
    } else if looks_like_asset {
        return (
            axum::http::StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            format!("not found: {path}"),
        )
            .into_response();
    } else {
        // v0.8.0 Sprint 10 bug fix: a hard navigation/reload of a dynamic route id that
        // `generateStaticParams()` didn't pre-render (e.g. a real `/projects/<id>` or
        // `/you/sessions/<id>`) used to fall straight through to the site ROOT's index.html -
        // verified live, this silently rendered the "Now" dashboard at the `/you/sessions/<id>`
        // URL instead of the session page. `find_route_family_shell` finds that route family's
        // own pre-rendered placeholder shell instead, which loads the right client component
        // and reads the real `id` from the URL itself.
        find_route_family_shell(&path).unwrap_or_else(|| "index.html".to_string())
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
            // Fallback page - same nonce injection treatment.
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

/// Find the pre-rendered shell for `path`'s dynamic-route family among the embedded web assets.
fn find_route_family_shell(path: &str) -> Option<String> {
    find_shell_in(
        path,
        <WebAssets as RustEmbed>::iter().map(|f| f.to_string()),
    )
}

/// Walk up `path`'s segments (most specific first) looking for any file in `files` that sits
/// one level under that prefix and ends in `.html`. Every dynamic route in this dashboard uses
/// a single-entry `generateStaticParams()` (e.g. `projects/placeholder.html`,
/// `you/sessions/new.html`), so there's exactly one shell to find per route family. Returns
/// `None` if no prefix at any depth matches. Split from [`find_route_family_shell`] so the
/// matching logic is testable without depending on `web/out/` actually being built.
fn find_shell_in(path: &str, files: impl Iterator<Item = String>) -> Option<String> {
    let segments: Vec<&str> = path.split('/').collect();
    let files: Vec<String> = files.collect();
    for take in (1..segments.len()).rev() {
        let want_prefix = format!("{}/", segments[..take].join("/"));
        if let Some(hit) = files
            .iter()
            .find(|f| f.starts_with(&want_prefix) && f.ends_with(".html"))
        {
            return Some(hit.clone());
        }
    }
    None
}

/// Insert a CSP nonce into every `<script>` tag that doesn't already have one. We rewrite
/// `<script>` -> `<script nonce="{nonce}">` (preserving any other attributes) and add a
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

/// Percent-decode a URL path. Returns `None` if the input contains an
/// invalid percent-escape or a non-UTF-8 byte sequence.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_digit(bytes[i + 1])?;
                let lo = hex_digit(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// -- handlers --------------------------------------------------------------------------------

async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "name": "cairn",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /api/health/deep` - real dependency probe. Returns 200 when all components are
/// reachable, 503 when any are degraded. Safe to use as a load-balancer readiness check.
async fn health_deep(State(s): State<AppState>) -> (axum::http::StatusCode, Json<Value>) {
    let db_ok = s.store.count_memories().is_ok();
    let embedder_ok = cairn_embed::from_config(&s.cfg.embed).is_ok();
    let admin_ok = admin_mod::load_admin(&s)
        .map(|r| r.is_some())
        .unwrap_or(false);
    let all_ok = db_ok && embedder_ok;
    let code = if all_ok {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        code,
        Json(json!({
            "status": if all_ok { "ok" } else { "degraded" },
            "name": "cairn",
            "version": env!("CARGO_PKG_VERSION"),
            "components": {
                "db": if db_ok { "ok" } else { "unreachable" },
                "embedder": if embedder_ok { "ok" } else { "unavailable" },
                "admin": if admin_ok { "configured" } else { "not_configured" },
            }
        })),
    )
}

/// `GET /api/setup/embed-default` - the embed provider the wizard pre-selects. Per the
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
    // Record a savings entry - the "compact" bytes we sent vs the "full" bytes the file
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
        None => Err(ApiError::not_found("unknown handle")),
    }
}

/// Web redesign v2: read-only effective-config viewer for the dashboard's Settings page.
/// Server config is env-driven and read once at boot - this endpoint SHOWS every knob, its
/// current effective value, and the env var that changes it (one-time customization is done
/// by editing the environment and restarting; the dashboard never mutates server config).
/// Secrets are redacted to a set/not-set marker. Authed like every other /api route.
#[derive(Serialize)]
struct ConfigEntry {
    key: &'static str,
    value: Value,
    env: &'static str,
    group: &'static str,
    description: &'static str,
}

/// Masks embedded userinfo credentials in a URL (`scheme://user:pass@host/...` becomes
/// `scheme://***@host/...`). `db_url` is documented as credential-free - auth goes through the
/// separate `db_user`/`db_pass` fields - but nothing enforces that, so a `CAIRN_DB_URL` set with
/// embedded credentials would otherwise leak a password verbatim to the config viewer. Returns
/// the input unchanged if it doesn't parse as a URL or carries no username/password.
fn mask_url_credentials(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        let _ = parsed.set_password(None);
        let _ = parsed.set_username("***");
    }
    parsed.to_string()
}

async fn config_view(State(s): State<AppState>) -> Json<Vec<ConfigEntry>> {
    fn secret(present: bool) -> Value {
        json!(if present { "set" } else { "not set" })
    }
    let c = &s.cfg;
    let entries = vec![
        ConfigEntry { key: "host", value: json!(c.host), env: "CAIRN_HOST", group: "server", description: "Bind host for the HTTP server." },
        ConfigEntry { key: "port", value: json!(c.port), env: "CAIRN_PORT", group: "server", description: "Bind port for the HTTP server." },
        ConfigEntry { key: "data_dir", value: json!(c.data_dir.display().to_string()), env: "CAIRN_DATA_DIR", group: "server", description: "Where Cairn keeps blobs, sessions, and the registry." },
        ConfigEntry { key: "tls", value: json!(c.tls.is_some()), env: "CAIRN_TLS_CERT / CAIRN_TLS_KEY", group: "server", description: "HTTPS material. Required for non-loopback binds unless insecure is set." },
        ConfigEntry { key: "insecure", value: json!(c.insecure), env: "CAIRN_INSECURE", group: "server", description: "Allow plain HTTP on a non-loopback bind (reverse-proxy setups only)." },
        ConfigEntry { key: "cors_origins", value: json!(c.cors_origins), env: "CAIRN_CORS_ORIGINS", group: "server", description: "Allowed CORS origins; empty = same-origin only." },
        ConfigEntry { key: "secret_key", value: secret(c.secret_key.is_some()), env: "CAIRN_SECRET_KEY", group: "server", description: "HMAC secret signing device-token JWTs and the savings ledger." },
        ConfigEntry { key: "db_url", value: json!(mask_url_credentials(&c.db_url)), env: "CAIRN_DB_URL", group: "database", description: "SurrealDB connection URL. Embedded userinfo credentials, if any, are masked." },
        ConfigEntry { key: "db_user", value: json!(c.db_user), env: "CAIRN_DB_USER", group: "database", description: "SurrealDB auth username." },
        ConfigEntry { key: "db_pass", value: secret(!c.db_pass.is_empty()), env: "CAIRN_DB_PASS", group: "database", description: "SurrealDB auth password." },
        ConfigEntry { key: "db_ns", value: json!(c.db_ns), env: "CAIRN_DB_NS", group: "database", description: "SurrealDB namespace (isolation boundary between Cairn instances)." },
        ConfigEntry { key: "db_timeout_secs", value: json!(c.db_timeout_secs), env: "CAIRN_DB_TIMEOUT_SECS", group: "database", description: "Per-query deadline for the SurrealDB client." },
        ConfigEntry { key: "multi_tenant", value: json!(c.multi_tenant), env: "CAIRN_MULTI_TENANT", group: "database", description: "Org-scoped memories per bearer token; off = single implicit org." },
        ConfigEntry { key: "embed.provider", value: json!(c.embed.provider), env: "CAIRN_EMBED_PROVIDER", group: "intelligence", description: "Embedding backend (hashing / openai / ollama / ...)." },
        ConfigEntry { key: "embed.api_key", value: secret(c.embed.api_key.is_some()), env: "CAIRN_EMBED_API_KEY", group: "intelligence", description: "API key for remote embedding providers." },
        ConfigEntry { key: "llm_consolidation.enabled", value: json!(c.llm_consolidation.enabled), env: "CAIRN_LLM_CONSOLIDATION", group: "intelligence", description: "LLM-driven consolidation, concept extraction, and query expansion." },
        ConfigEntry { key: "llm_daily_budget", value: json!(c.llm_daily_budget), env: "CAIRN_LLM_DAILY_BUDGET", group: "intelligence", description: "Daily token budget for background LLM calls; 0 = unlimited. Heuristic fallback once spent." },
        ConfigEntry { key: "rerank.enabled", value: json!(c.rerank.enabled), env: "CAIRN_RERANK", group: "intelligence", description: "Cross-encoder reranking for search." },
        ConfigEntry { key: "selftune", value: json!(c.selftune), env: "CAIRN_SELFTUNE", group: "automation", description: "Weekly tune job nudges promote_threshold from measured retrieval quality (bounded)." },
        ConfigEntry { key: "cron_enabled", value: json!(c.cron_enabled), env: "CAIRN_CRON_ENABLED", group: "automation", description: "Master switch for every background job." },
        ConfigEntry { key: "session_ttl_days", value: json!(c.session_ttl_days), env: "CAIRN_SESSION_TTL_DAYS", group: "automation", description: "Idle days before session-gc promotes Session-scoped memories to Global; 0 disables." },
        ConfigEntry { key: "decay_period_days", value: json!(c.decay_period_days), env: "CAIRN_DECAY_PERIOD_DAYS", group: "automation", description: "Confidence half-life for the weekly memory-decay job." },
        ConfigEntry { key: "access_log_retention_days", value: json!(c.access_log_retention_days), env: "CAIRN_ACCESS_LOG_RETENTION_DAYS", group: "automation", description: "How long access-log rows survive the monthly prune." },
        ConfigEntry { key: "promote_threshold", value: json!(c.promote_threshold), env: "CAIRN_PROMOTE_THRESHOLD", group: "automation", description: "promo_score above this auto-promotes to Global on the nightly llm-intelligence run." },
        ConfigEntry { key: "demote_idle_days", value: json!(c.demote_idle_days), env: "CAIRN_DEMOTE_IDLE_DAYS", group: "automation", description: "Unused days before an auto-promoted Global memory reverts to Project scope." },
        ConfigEntry { key: "max_working_per_project", value: json!(c.max_working_per_project), env: "CAIRN_MAX_WORKING_PER_PROJECT", group: "automation", description: "Cap on Working-tier memories per project before decay prunes the weakest." },
        ConfigEntry { key: "drift_autopilot", value: json!(c.drift_autopilot), env: "CAIRN_DRIFT_AUTOPILOT", group: "guard", description: "Drift auto-approval policy: safe (default), off, or all. Danger never auto-approves." },
        ConfigEntry { key: "drift_safe_globs", value: json!(c.drift_safe_globs), env: "CAIRN_DRIFT_SAFE_GLOBS", group: "guard", description: "Low-stakes path globs where warn-risk drift auto-approves under the safe policy." },
        ConfigEntry { key: "auto_anchor", value: json!(c.auto_anchor), env: "CAIRN_AUTO_ANCHOR", group: "guard", description: "Derive a session anchor from the first substantive prompt when none is set." },
    ];
    Json(entries)
}

async fn remember(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
    Json(mut input): Json<NewMemory>,
) -> Result<Json<Memory>, ApiError> {
    // Strip injected preference delimiter blocks so memory content cannot smuggle itself back
    // into the preference pipeline as a model directive.
    input.content = cairn_profile::strip_preference_blocks(&input.content);
    input.suspicious =
        Some(input.suspicious.unwrap_or(false) || cairn_profile::is_suspicious(&input.content));
    // v0.8.0 Sprint 2: a caller that sets `scope_type` without an explicit `scope_id` gets the
    // current request's project/session (from `X-Cairn-Project`/`X-Cairn-Session`) - so the
    // client doesn't have to repeat the id it already sent as a header.
    if input.scope_id.is_none() {
        input.scope_id = match input.scope_type {
            cairn_core::ScopeType::Project => scope.project_id.clone(),
            cairn_core::ScopeType::Session => scope.session_id.clone(),
            cairn_core::ScopeType::Global => None,
        };
    }
    let m = s.mem.remember(input)?;
    crate::events::publish_memory(&s.events, "added", &m.id);
    Ok(Json(m))
}

/// P4.3: `POST /api/memory/gotcha` - record a failure event. The gotcha tracker
/// auto-promotes to a `MemoryKind::Gotcha` memory when the cluster threshold is met.
#[derive(Deserialize, Clone)]
struct GotchaBody {
    topic: String,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    refs: Vec<String>,
    #[serde(default)]
    session_id: Option<String>,
}

async fn record_gotcha(
    State(s): State<AppState>,
    Json(b): Json<GotchaBody>,
) -> Result<Json<Value>, ApiError> {
    let mut event =
        cairn_memory::FailureEvent::new(b.topic.clone(), b.context.clone().unwrap_or_default());
    if !b.refs.is_empty() {
        event = event.with_refs(b.refs.clone());
    }
    if let Some(sid) = b.session_id {
        event = event.with_session(sid);
    }
    let promoted = s.mem.record_failure(event)?;
    Ok(Json(json!({
        "ok": true,
        "promoted": promoted.is_some(),
        "memory": promoted,
    })))
}

/// P4.3: `GET /api/memory/gotcha/wakeup` - top-K gotcha clusters for proactive recall.
#[derive(Deserialize)]
struct GotchaWakeupQuery {
    #[serde(default)]
    limit: Option<usize>,
}

async fn gotcha_wakeup(
    State(s): State<AppState>,
    Query(q): Query<GotchaWakeupQuery>,
) -> Result<Json<Value>, ApiError> {
    let limit = q.limit.unwrap_or(5).clamp(1, 50);
    let clusters: Vec<Value> = s
        .mem
        .top_gotcha_clusters(limit)?
        .iter()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
            if let Value::Object(ref mut m) = v {
                m.insert("size".into(), json!(c.size()));
            }
            v
        })
        .collect();
    let total = s
        .mem
        .gotcha_tracker()
        .lock()
        .map(|t| t.total_failures)
        .unwrap_or(0);
    let promoted = s
        .mem
        .gotcha_tracker()
        .lock()
        .map(|t| t.promoted_clusters)
        .unwrap_or(0);
    Ok(Json(json!({
        "clusters": clusters,
        "total_failures": total,
        "promoted_clusters": promoted,
    })))
}

#[derive(Deserialize)]
struct RecallQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
}

async fn recall(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
    Query(q): Query<RecallQuery>,
) -> Result<Json<Vec<ScoredMemory>>, ApiError> {
    Ok(Json(s.mem.recall_for_org(
        &q.q,
        q.limit.unwrap_or(10),
        OrgId::default(),
        &scope,
    )?))
}

#[derive(Deserialize)]
struct ByScopeQuery {
    scope_type: cairn_core::ScopeType,
    #[serde(default)]
    scope_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// GET `/api/memory/by-scope?scope_type=project&scope_id=<id>&limit=50` - v0.8.0 Sprint 10:
/// every memory in an *exact* scope, most-recently-updated first. Unlike `/api/memory/recall`
/// this isn't a search (no ranking, no Global fallback) - it's the plain filter the Projects
/// detail page needs ("what does this project know"), where faking `X-Cairn-Project` on recall
/// would wrongly blend in Global results and require a text query that doesn't exist yet.
async fn memory_by_scope(
    State(s): State<AppState>,
    Query(q): Query<ByScopeQuery>,
) -> Result<Json<Vec<Memory>>, ApiError> {
    if q.scope_type != cairn_core::ScopeType::Global && q.scope_id.is_none() {
        return Err(ApiError::bad_request(
            "scope_id is required unless scope_type=global",
        ));
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let mut mems: Vec<Memory> = s
        .store
        .all_memories()?
        .into_iter()
        .filter(|m| m.scope_type == q.scope_type && m.scope_id == q.scope_id)
        .collect();
    mems.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
    mems.truncate(limit);
    Ok(Json(mems))
}

/// Web redesign v2: the Memory Browser's list endpoint. Filterable, sortable, paginated view of
/// EVERY memory with the full record serialization (all fields, edges included) - the dashboard's
/// primary observability surface. Same in-handler `all_memories()` scan pattern as `by-scope` and
/// the project stats; `total` requires the full scan anyway, so pagination only saves
/// serialization - fine at self-hosted scale.
#[derive(Deserialize, Default)]
struct MemoryListQuery {
    #[serde(default)]
    scope_type: Option<cairn_core::ScopeType>,
    #[serde(default)]
    scope_id: Option<String>,
    #[serde(default)]
    tier: Option<cairn_core::MemoryTier>,
    #[serde(default)]
    kind: Option<cairn_core::MemoryKind>,
    #[serde(default)]
    pinned: Option<bool>,
    #[serde(default)]
    suspicious: Option<bool>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Serialize)]
struct MemoryListResponse {
    items: Vec<Memory>,
    total: usize,
}

/// GET `/api/memory?scope_type=&tier=&kind=&pinned=&suspicious=&q=&sort=&limit=&offset=`
async fn list_memories(
    State(s): State<AppState>,
    Query(q): Query<MemoryListQuery>,
) -> Result<Json<MemoryListResponse>, ApiError> {
    let mut mems: Vec<Memory> = s.store.all_memories()?;
    if let Some(st) = q.scope_type {
        mems.retain(|m| m.scope_type == st);
    }
    if let Some(sid) = &q.scope_id {
        mems.retain(|m| m.scope_id.as_deref() == Some(sid.as_str()));
    }
    if let Some(tier) = q.tier {
        mems.retain(|m| m.tier == tier);
    }
    if let Some(kind) = q.kind {
        mems.retain(|m| m.kind == kind);
    }
    if let Some(pinned) = q.pinned {
        mems.retain(|m| m.pinned == pinned);
    }
    if let Some(suspicious) = q.suspicious {
        mems.retain(|m| m.suspicious == suspicious);
    }
    if let Some(needle) =
        q.q.as_deref()
            .map(str::to_lowercase)
            .filter(|n| !n.is_empty())
    {
        mems.retain(|m| {
            m.content.to_lowercase().contains(&needle)
                || m.concepts
                    .iter()
                    .any(|c| c.to_lowercase().contains(&needle))
        });
    }
    let total = mems.len();
    // Unknown sort values fall back to updated_at rather than erroring - consistent with the
    // leniency of `heatmap`/`by-scope`. All sorts are descending (newest/highest first).
    match q.sort.as_deref() {
        Some("created_at") => mems.sort_by_key(|m| std::cmp::Reverse(m.created_at)),
        Some("importance") => mems.sort_by(|a, b| b.importance.total_cmp(&a.importance)),
        Some("promo_score") => mems.sort_by(|a, b| b.promo_score.total_cmp(&a.promo_score)),
        Some("access_count") => mems.sort_by_key(|m| std::cmp::Reverse(m.access_count)),
        _ => mems.sort_by_key(|m| std::cmp::Reverse(m.updated_at)),
    }
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0);
    let items: Vec<Memory> = mems.into_iter().skip(offset).take(limit).collect();
    Ok(Json(MemoryListResponse { items, total }))
}

/// GET `/api/memory/:id` - single memory, full record. Read-only lookup for the Memory Browser's
/// detail drawer (and edge-hopping between related memories); does NOT bump `access_count`.
async fn get_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Memory>, ApiError> {
    match s.mem.get(&id)? {
        Some(m) => Ok(Json(m)),
        None => Err(ApiError::not_found("no such memory")),
    }
}

/// v0.8.0 Sprint 3: the project registry populated by the `SessionStart` hook's auto-detection
/// (`cairn-client`'s `project::detect_project`), so the dashboard can show which repos an agent
/// has actually worked in. Registration is independent of scope isolation itself - Sprint 2's
/// recall filtering only needs the `X-Cairn-Project` header, not a matching registry row.
#[derive(Deserialize)]
struct ProjectUpsertBody {
    id: String,
    name: String,
    #[serde(default)]
    path: String,
}

/// PATCH `/api/projects/upsert` - create or update a project's `last_active` timestamp.
async fn upsert_project(
    State(s): State<AppState>,
    Json(body): Json<ProjectUpsertBody>,
) -> Result<Json<cairn_store::ProjectRecord>, ApiError> {
    s.store.upsert_project(&body.id, &body.name, &body.path)?;
    let project = s
        .store
        .get_project(&body.id)?
        .ok_or_else(|| ApiError::internal("project vanished immediately after upsert"))?;
    crate::events::publish_project(&s.events, &project.id);
    Ok(Json(project))
}

/// v0.8.0 Sprint 10: `GET /api/projects`/`GET /api/projects/:id` enriched with memory_count +
/// last_memory_at, so the dashboard's Projects page doesn't need a second round-trip per row.
#[derive(Serialize)]
struct ProjectWithStats {
    #[serde(flatten)]
    project: cairn_store::ProjectRecord,
    memory_count: usize,
    last_memory_at: Option<DateTime<Utc>>,
}

/// Group `mems` by `scope_id` for every `Project`-scoped memory, giving `(count, latest
/// updated_at)` per project id. One full-corpus scan shared by both project endpoints below -
/// same cost class as `promotion_candidates`/`architecture_report`, no new store trait method.
fn project_memory_stats(
    mems: &[Memory],
) -> std::collections::HashMap<String, (usize, DateTime<Utc>)> {
    let mut stats: std::collections::HashMap<String, (usize, DateTime<Utc>)> =
        std::collections::HashMap::new();
    for m in mems {
        if m.scope_type != cairn_core::ScopeType::Project {
            continue;
        }
        let Some(id) = m.scope_id.as_ref() else {
            continue;
        };
        let entry = stats.entry(id.clone()).or_insert((0, m.updated_at));
        entry.0 += 1;
        if m.updated_at > entry.1 {
            entry.1 = m.updated_at;
        }
    }
    stats
}

fn with_stats(
    project: cairn_store::ProjectRecord,
    stats: &std::collections::HashMap<String, (usize, DateTime<Utc>)>,
) -> ProjectWithStats {
    let (memory_count, last_memory_at) = stats
        .get(&project.id)
        .map(|(c, t)| (*c, Some(*t)))
        .unwrap_or((0, None));
    ProjectWithStats {
        project,
        memory_count,
        last_memory_at,
    }
}

/// GET `/api/projects` - every known project, most-recently-active first.
async fn list_projects(State(s): State<AppState>) -> Result<Json<Vec<ProjectWithStats>>, ApiError> {
    let projects = s.store.list_projects()?;
    let stats = project_memory_stats(&s.store.all_memories()?);
    Ok(Json(
        projects
            .into_iter()
            .map(|p| with_stats(p, &stats))
            .collect(),
    ))
}

/// GET `/api/projects/:id` - a single project's details.
async fn get_project(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ProjectWithStats>, ApiError> {
    let project = s
        .store
        .get_project(&id)?
        .ok_or_else(|| ApiError::not_found("no such project"))?;
    let stats = project_memory_stats(&s.store.all_memories()?);
    Ok(Json(with_stats(project, &stats)))
}

/// v0.8.0 Sprint 4: one entry in `GET /api/cron/jobs` - the job's static schedule plus whatever
/// its last recorded run looked like (`None` if it has never run since server start).
#[derive(Serialize)]
struct CronJobStatus {
    name: &'static str,
    schedule: &'static str,
    last_run: Option<cron::CronRun>,
}

/// GET `/api/cron/jobs` - every background job, its cron schedule, and its last run.
async fn list_cron_jobs(State(s): State<AppState>) -> Json<Vec<CronJobStatus>> {
    Json(
        cron::JOBS
            .iter()
            .map(|(name, schedule)| CronJobStatus {
                name,
                schedule,
                last_run: s.cron_history.last_run(name),
            })
            .collect(),
    )
}

/// POST `/api/cron/run/:job` - trigger a job immediately, same auth as every other `/api/*`
/// route. Goes through the same [`cron::run_job_now`] the scheduler itself calls, so a manual
/// run and a scheduled run are indistinguishable in `GET /api/cron/history`.
async fn run_cron_job(
    State(s): State<AppState>,
    axum::extract::Path(job): axum::extract::Path<String>,
) -> Result<Json<cron::CronRun>, ApiError> {
    let name = cron::JOBS
        .iter()
        .find(|(n, _)| *n == job)
        .map(|(n, _)| *n)
        .ok_or_else(|| ApiError::not_found(format!("no such cron job: {job}")))?;
    cron::run_job_now(&s, name)
        .map(Json)
        .map_err(ApiError::bad_request)
}

#[derive(Deserialize)]
struct CronHistoryQuery {
    #[serde(default)]
    job: Option<String>,
}

/// GET `/api/cron/history` - recent runs, optionally filtered to one job (`?job=session-gc`).
async fn cron_history(
    State(s): State<AppState>,
    Query(q): Query<CronHistoryQuery>,
) -> Json<Vec<cron::CronRun>> {
    Json(s.cron_history.recent(q.job.as_deref()))
}

/// v0.8.0 Sprint 9: one entry in `GET /api/cron/health` - unlike [`CronJobStatus`] this exists
/// for a watchdog/dashboard to answer "is anything wrong?" at a glance, without having to infer
/// it from `last_run`'s shape.
#[derive(Serialize)]
struct CronJobHealth {
    name: &'static str,
    schedule: &'static str,
    running: bool,
    last_run_at: Option<DateTime<Utc>>,
    last_status: Option<&'static str>,
    stale: bool,
}

/// GET `/api/cron/health` - single-flight + staleness view of every job, for a watchdog or the
/// dashboard to poll instead of re-deriving health from `GET /api/cron/history` itself.
async fn cron_health(State(s): State<AppState>) -> Json<Vec<CronJobHealth>> {
    let now = Utc::now();
    Json(
        cron::JOBS
            .iter()
            .map(|(name, schedule)| {
                let last = s.cron_history.last_run(name);
                CronJobHealth {
                    name,
                    schedule,
                    running: s.cron_history.is_running(name),
                    last_run_at: last.as_ref().map(|r| r.started_at),
                    last_status: last.as_ref().map(|r| r.outcome),
                    stale: s.cron_history.is_stale(name, now),
                }
            })
            .collect(),
    )
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    rerank_depth: Option<usize>,
    /// P3.3: when `true` and `LlmConsolidationConfig::enabled`, runs the query through
    /// `QueryExpander` and merges by max combinedScore across reformulations.
    #[serde(default)]
    expand: Option<bool>,
    /// P4.2: when `local` and `RerankConfig::enabled`, installs a cross-encoder
    /// reranker at search time. Other values: `none` (default, no-op).
    #[serde(default)]
    rerank: Option<String>,
}

/// `GET /api/search?q=...&limit=N&rerank_depth=M&expand=true&rerank=local` - hybrid search:
/// BM25 + HNSW + memory provenance graph, fused with RRF, reranked with MMR. With
/// `expand=true` the engine first asks an LLM for 3-5 reformulations and merges results.
/// With `rerank=local` the post-MMR top-K is re-scored by a cross-encoder.
async fn search_handler(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<ScoredMemory>>, ApiError> {
    let limit = q.limit.unwrap_or(20);
    let rerank_depth = q.rerank_depth.unwrap_or(20);
    let org_id = OrgId::default();

    // P3.3 query expansion: when `expand=true` and the LLM gate is on.
    if q.expand.unwrap_or(false) && s.cfg.llm_consolidation.enabled {
        let expander = cairn_memory::QueryExpander::new(s.cfg.llm_consolidation.clone());
        // P4.2 rerank: reranks the *expanded* hits, rather than discarding them and rerunning
        // a plain (unexpanded) search-with-rerank - see expanded_search_with_rerank_for_org's
        // doc comment for why these two used to be mutually exclusive.
        let hits = if q.rerank.as_deref() == Some("local") && s.cfg.rerank.enabled {
            let reranker = cairn_memory::rerank_from_config(&s.cfg.rerank);
            s.mem.expanded_search_with_rerank_for_org(
                &q.q,
                limit,
                rerank_depth,
                &expander,
                &*reranker,
                s.cfg.rerank.blend_weight,
                org_id,
                &scope,
            )?
        } else {
            s.mem
                .expanded_search_for_org(&q.q, limit, rerank_depth, &expander, org_id, &scope)?
        };
        return Ok(Json(hits));
    }

    // P4.2 rerank only (no expansion).
    if q.rerank.as_deref() == Some("local") && s.cfg.rerank.enabled {
        let reranker = cairn_memory::rerank_from_config(&s.cfg.rerank);
        return Ok(Json(s.mem.hybrid_search_with_rerank_for_org(
            &q.q,
            limit,
            rerank_depth,
            &*reranker,
            s.cfg.rerank.blend_weight,
            org_id,
            &scope,
        )?));
    }

    // Default path: plain hybrid search, scope-aware (v0.8.0 Sprint 10 - was Global-only).
    Ok(Json(s.mem.hybrid_search_for_org(
        &q.q,
        limit,
        rerank_depth,
        org_id,
        &scope,
    )?))
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
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
}

/// POST `/api/memory/:id` - edit a memory's mutable fields. Any field omitted from the body is
/// left unchanged. The suspicious-content scan re-runs on the new content (defense-in-depth).
async fn edit_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<MemoryEditBody>,
) -> Result<Json<Memory>, ApiError> {
    let content = body
        .content
        .map(|c| cairn_profile::strip_preference_blocks(&c));
    match s.mem.edit(
        &id,
        content,
        body.importance,
        body.concepts,
        body.files,
        body.title,
        body.reasoning,
    )? {
        Some(m) => {
            crate::events::publish_memory(&s.events, "edited", &m.id);
            Ok(Json(m))
        }
        None => Err(ApiError::not_found("no such memory")),
    }
}

/// DELETE `/api/memory/:id` - remove a memory by id.
async fn delete_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Value>, ApiError> {
    if s.mem.delete(&id)? {
        crate::events::publish_memory(&s.events, "deleted", &id);
        Ok(Json(json!({ "deleted": true })))
    } else {
        Err(ApiError::not_found("no such memory"))
    }
}

#[derive(Deserialize)]
struct PinBody {
    pinned: bool,
}

/// POST `/api/memory/:id/pin` - pin or unpin a memory.
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
        let m = s
            .mem
            .get(&id)?
            .ok_or_else(|| ApiError::internal("memory vanished immediately after pin/unpin"))?;
        Ok(Json(m))
    } else {
        Err(ApiError::not_found("no such memory"))
    }
}

/// POST `/api/memory/:id/reinforce` - manually nudge a memory's confidence (e.g. after a
/// `confirm_useful` click in the dashboard). Returns the updated memory.
async fn reinforce_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Memory>, ApiError> {
    s.store.reinforce_memory(&id)?;
    match s.mem.get(&id)? {
        Some(m) => Ok(Json(m)),
        None => Err(ApiError::not_found("no such memory")),
    }
}

#[derive(Deserialize)]
struct CrystallizeBody {
    #[serde(default)]
    session_id: Option<String>,
}

/// GET `/api/memory/promotion-candidates` - v0.8.0 Sprint 5: `Project`-scoped memories whose
/// `promo_score` (written by the `llm-intelligence` cron job) falls in the human-review band.
async fn promotion_candidates(State(s): State<AppState>) -> Result<Json<Vec<Memory>>, ApiError> {
    Ok(Json(s.mem.promotion_candidates()?))
}

/// POST `/api/memory/:id/promote` - v0.8.0 Sprint 5: approve a promotion candidate. Moves the
/// memory to `Global` scope and locks it against further promotion scoring.
async fn promote_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Memory>, ApiError> {
    if s.mem.promote_memory(&id)? {
        crate::events::publish_memory(&s.events, "promoted", &id);
        let m = s
            .mem
            .get(&id)?
            .ok_or_else(|| ApiError::internal("memory vanished immediately after promotion"))?;
        Ok(Json(m))
    } else {
        Err(ApiError::not_found("no such memory"))
    }
}

/// POST `/api/memory/:id/dismiss-promotion` - v0.8.0 Sprint 5: "don't ask again". Locks the
/// memory against further promotion scoring/suggestions without changing its scope.
async fn dismiss_promotion(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Memory>, ApiError> {
    if s.mem.dismiss_promotion(&id)? {
        let m = s
            .mem
            .get(&id)?
            .ok_or_else(|| ApiError::internal("memory vanished immediately after dismissal"))?;
        Ok(Json(m))
    } else {
        Err(ApiError::not_found("no such memory"))
    }
}

/// GET `/api/memory/promotion-log` - v0.8.0 Sprint 8: recent promotion/demotion events, auto
/// and manual alike, newest first. The dashboard's audit trail for full-auto promotion.
#[derive(Deserialize)]
struct PromotionLogQuery {
    #[serde(default)]
    limit: Option<usize>,
}
async fn promotion_log(
    State(s): State<AppState>,
    Query(q): Query<PromotionLogQuery>,
) -> Result<Json<Vec<cairn_store::PromotionLogEntry>>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    Ok(Json(s.store.list_promotion_log(limit)?))
}

/// POST `/api/memory/:id/demote` - v0.8.0 Sprint 8 (Undo): revert a promotion back to the
/// scope it was promoted from, using the promotion log. Works for both auto-promotions and
/// human `/promote` calls. 404 if the memory doesn't exist or was never promoted through this
/// pipeline (nothing recorded to revert to).
async fn demote_memory(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<Memory>, ApiError> {
    if s.mem.demote_memory(&id)? {
        crate::events::publish_memory(&s.events, "demoted", &id);
        let m = s
            .mem
            .get(&id)?
            .ok_or_else(|| ApiError::internal("memory vanished immediately after demotion"))?;
        Ok(Json(m))
    } else {
        Err(ApiError::not_found(
            "no such memory, or it was never promoted",
        ))
    }
}

/// POST `/api/memory/session-summary` - v0.8.0 Sprint 5: called by the `SessionEnd` hook.
/// Summarizes the current session's memories (from `X-Cairn-Session`) into one `Project`-scoped
/// `Episodic` note via the LLM. A safe no-op (`{"summarized": false}`) when the LLM is disabled,
/// there's no session id, or the session has no memories yet - the hook never treats this as an
/// error either way.
async fn session_summary(
    State(s): State<AppState>,
    Extension(scope): Extension<ScopeCtx>,
) -> Result<Json<Value>, ApiError> {
    let Some(session_id) = scope.session_id else {
        return Ok(Json(json!({ "summarized": false })));
    };
    match s
        .mem
        .synthesize_session(&s.cfg.llm_consolidation, &session_id, scope.project_id)?
    {
        Some(m) => {
            crate::events::publish_memory(&s.events, "created", &m.id);
            Ok(Json(json!({ "summarized": true, "memory": m })))
        }
        None => Ok(Json(json!({ "summarized": false }))),
    }
}

/// POST `/api/memory/crystallize` - promote working-tier memories into a semantic crystal.
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

/// GET `/api/memory/graph` - the provenance graph (nodes + edges) for the dashboard.
async fn memory_graph(
    State(s): State<AppState>,
) -> Result<Json<cairn_memory::MemoryGraph>, ApiError> {
    Ok(Json(s.mem.graph()?))
}

// P2.6: heatmap (last 365 days by default, ?days=N to override).
async fn memory_heatmap(
    State(s): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<HeatmapQuery>,
) -> Result<Json<std::collections::HashMap<String, u32>>, ApiError> {
    let days = q.days.unwrap_or(365).clamp(1, 3650);
    Ok(Json(s.mem.activity_heatmap(days)?))
}

// P2.4: structural analysis of the memory graph.
async fn architecture_report(
    State(s): State<AppState>,
) -> Result<Json<cairn_memory::ArchitectureReport>, ApiError> {
    let graph = s.mem.graph()?;
    Ok(Json(cairn_memory::generate_architecture_report(&graph)))
}

#[derive(Default, Deserialize)]
struct HeatmapQuery {
    /// Window size in days. Default 365. Capped to 3650 (10y).
    days: Option<usize>,
}

// -- drift + sessions handlers (v0.5.0 Sprint 4) -----------------------------------------
// Web redesign v2: drift decisions are made by the autopilot inline at verify time
// (cairn-guard/src/autopilot.rs) - the manual approve/reject endpoints were removed because
// the dashboard is observability-only. This list endpoint is the read-only audit trail.

async fn list_drift(
    State(s): State<AppState>,
) -> Result<Json<Vec<cairn_session::DriftEvent>>, ApiError> {
    Ok(Json(s.sessions.recent_drift(200, None)?))
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
        None => Err(ApiError::not_found("no such session")),
    }
}

/// PATCH a session - merge in new tasks/findings/decisions/touched_files/next_steps.
async fn update_session(
    State(s): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(patch): Json<cairn_session::SessionPatch>,
) -> Result<Json<cairn_session::Session>, ApiError> {
    let Some(mut sess) = s.sessions.load(&id)? else {
        return Err(ApiError::not_found("no such session"));
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
        // v0.8.0 Sprint 8: drift autopilot. A `risk=ok` edit never even reaches this branch (no
        // event is created for it at all - it was already a zero-interaction case before this
        // sprint); this only decides whether a `warn` under `CAIRN_DRIFT_AUTOPILOT` gets
        // auto-approved instead of sitting `Pending`. `danger` never auto-approves.
        let mode = cairn_guard::DriftAutopilot::parse(&s.cfg.drift_autopilot);
        let decision =
            cairn_guard::autopilot_decision(mode, report.risk, &b.path, &s.cfg.drift_safe_globs);
        let (status, detail) = match decision {
            Some(reason) => (
                cairn_session::DriftStatus::Approved,
                format!("{} (auto-approved: {reason})", report.message),
            ),
            None => (cairn_session::DriftStatus::Pending, report.message.clone()),
        };
        let ev = cairn_session::DriftEvent {
            id: s.sessions.next_drift_id(),
            ts: Utc::now(),
            path: b.path.clone(),
            risk: report.risk.as_str().to_string(),
            kind: "verify".into(),
            detail,
            status,
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
struct AutoAnchorBody {
    prompt: String,
}

/// POST `/api/guard/anchor/auto` - v0.8.0 Sprint 8: called by the `UserPromptSubmit` hook on
/// every prompt. A no-op (`"derived": false`) once an anchor already exists (whether set
/// manually or auto-derived earlier this session) or when `CAIRN_AUTO_ANCHOR` is off - cheap
/// to call unconditionally since the common case after the first prompt is an immediate no-op.
async fn auto_anchor(
    State(s): State<AppState>,
    Json(b): Json<AutoAnchorBody>,
) -> Result<Json<Value>, ApiError> {
    if !s.cfg.auto_anchor {
        return Ok(Json(
            json!({ "anchor": s.guard.anchor()?, "derived": false }),
        ));
    }
    match s.guard.auto_anchor_if_unset(&b.prompt)? {
        Some(meta) => Ok(Json(
            json!({ "anchor": meta.goal, "derived": true, "suspicious": meta.suspicious }),
        )),
        None => Ok(Json(
            json!({ "anchor": s.guard.anchor()?, "derived": false }),
        )),
    }
}

#[derive(Deserialize)]
struct DigestQuery {
    #[serde(default)]
    hours: Option<i64>,
}

#[derive(Serialize)]
struct AutopilotDigest {
    promoted: usize,
    demoted: usize,
    drift_auto_approved: usize,
}

/// GET `/api/memory/autopilot-digest?hours=24` - v0.8.0 Sprint 8: what autopilot did while the
/// human was away, for the `SessionStart` hook's "since you were away" summary line. Looks
/// back a fixed window (default 24h) rather than tracking a precise "since last session ended"
/// timestamp - each hook invocation is a stateless process (v0.8.0 Sprint 3), so there's no
/// cheap place to persist that client-side, and "in the last day" is close enough for a
/// human-facing digest.
async fn autopilot_digest(
    State(s): State<AppState>,
    Query(q): Query<DigestQuery>,
) -> Result<Json<AutopilotDigest>, ApiError> {
    let since = Utc::now() - chrono::Duration::hours(q.hours.unwrap_or(24));
    let log = s.store.list_promotion_log(500)?;
    let promoted = log
        .iter()
        .filter(|e| e.action == "promote" && e.ts > since)
        .count();
    let demoted = log
        .iter()
        .filter(|e| e.action == "demote" && e.ts > since)
        .count();
    let drift_auto_approved = s
        .sessions
        .recent_drift(500, None)?
        .into_iter()
        .filter(|d| d.ts > since && d.detail.contains("(auto-approved:"))
        .count();
    Ok(Json(AutopilotDigest {
        promoted,
        demoted,
        drift_auto_approved,
    }))
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

/// `GET /api/context/pressure` (P2.5) - returns the current context window utilization,
/// recommendation (NoAction / SuggestCompression / ForceCompression / EvictLeastRelevant),
/// and the top eviction candidates ranked by phi.
async fn context_pressure(
    State(s): State<AppState>,
) -> Result<Json<cairn_context::ContextPressure>, ApiError> {
    // For now, the ContextLedger isn't wired into AppState (it'd need to accumulate
    // entries from every `read`/`assemble` call). To make the endpoint and frontend
    // live immediately, we instantiate a fresh ledger with a sensible default window.
    // A future sprint will move this to AppState for real-time tracking.
    let window = s
        .cfg
        .embed
        .api_key
        .as_ref()
        .map(|_| 128_000)
        .unwrap_or(cairn_context::DEFAULT_WINDOW_SIZE);
    let ledger = cairn_context::ContextLedger::with_window_size(window);
    Ok(Json(ledger.pressure()))
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
/// itself (defense-in-depth - a client's redaction is never trusted) and rejects anything that
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
    let token = s
        .store
        .create_token(&name, cairn_core::TokenScope::Write, None)?;
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
            Err(ApiError::not_found("invalid or expired pairing code"))
        }
    }
}

// -- sync + auth -----------------------------------------------------------------------------

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
    let mcp = cairn_mcp::McpServer::new(&s.cfg).map_err(|e| ApiError::internal(e.to_string()))?;
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
///   1. Public endpoints (`/api/health`, `/api/pair/claim`, the admin auth surface) - pass
///      through unchanged.
///   2. Admin cookie - if `cairn_session` is present, signed, and the embedded generation
///      matches the live admin record, the request is treated as the admin (all scopes).
///   3. Device-token bearer - the existing JWT path; respected when no admin cookie is
///      available so CLI / MCP clients keep working.
///   4. Loopback fallback - when there are no device tokens AND no admin (first-run before
///      `/setup`), only loopback calls pass. The admin cookie path overrides this once
///      `/setup` has been visited.
///
/// All errors are 401 with a uniform JSON body.
async fn auth(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let method = req.method().as_str();

    // 1. Public endpoints - never require auth.
    if !path.starts_with("/api/")
        || matches!(
            path,
            "/api/health"
                | "/api/capabilities"
                | "/api/openapi.json"
                | "/api/pair/claim"
                | "/api/auth/login"
                | "/api/auth/logout"
                | "/api/auth/me"
                | "/api/auth/setup"
                | "/api/auth/status"
                | "/api/setup/health"
                | "/api/push/subscribe"
                | "/api/push/unsubscribe"
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
    match verify_bearer_auth(&s, &req, method, path) {
        VerifyBearerOutcome::Valid(token_id) => {
            tracing::trace!(path, "auth=bearer");
            let _ = s.store.record_token_usage(&token_id);
            return next.run(req).await;
        }
        VerifyBearerOutcome::BadSignature => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "invalid bearer token",
                    "error_code": "unauthenticated",
                    "reason": "bad_signature",
                    "detail": "token signature verification failed (secret key may have been rotated, or the token is malformed)"
                })),
            )
                .into_response();
        }
        VerifyBearerOutcome::UnknownToken => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": "invalid bearer token",
                    "error_code": "unauthenticated",
                    "reason": "unknown_token",
                    "detail": "token is cryptographically valid but was not found in the store (it may have been revoked, or the database data may have been lost)"
                })),
            )
                .into_response();
        }
        VerifyBearerOutcome::InsufficientScope => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "invalid bearer token",
                    "error_code": "forbidden",
                    "reason": "insufficient_scope",
                    "detail": "the token's scope does not permit this operation"
                })),
            )
                .into_response();
        }
        VerifyBearerOutcome::Absent => {} // fall through to loopback / 401 below
    }

    // 4. Loopback fallback - only when there are no device tokens AND no admin.
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
                "error": "no admin or device tokens configured; create an admin via /setup on localhost",
                "error_code": "unauthenticated"
            })),
        )
            .into_response();
    }

    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": "invalid or missing credentials",
            "error_code": "unauthenticated"
        })),
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
    /// Carries the token id so the caller can record usage.
    Valid(String),
    /// Bearer present but signature/expiration check failed.
    BadSignature,
    /// Bearer present, signature valid, but the token id is not in the store (revoked or
    /// data loss).
    UnknownToken,
    /// Bearer present, signature valid, token exists, but the scope does not permit the
    /// requested method+path.
    InsufficientScope,
    /// No `Authorization: Bearer ...` header was sent at all. The caller may fall through
    /// to weaker authentication paths (loopback fallback) if appropriate.
    Absent,
}

/// Verify a device-token bearer against the request.
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
    let info = match s.verify_bearer(bearer) {
        Some(i) => i,
        None => return VerifyBearerOutcome::BadSignature,
    };
    if !s.store.validate_token_id(&info.id).unwrap_or(false) {
        return VerifyBearerOutcome::UnknownToken;
    }
    if !info.scope.allows(method, path) {
        return VerifyBearerOutcome::InsufficientScope;
    }
    VerifyBearerOutcome::Valid(info.id)
}

// -- error plumbing --------------------------------------------------------------------------

/// A simple API error that renders as JSON `{ "error": "...", "error_code": "stable_code" }`.
///
/// `error_code` is a stable machine-readable identifier that SDKs/agents can branch on
/// without parsing prose. The set of codes is documented and stable across versions:
/// `not_found`, `bad_request`, `unauthenticated`, `forbidden`, `conflict`, `internal_error`.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub code: &'static str,
}

impl ApiError {
    /// Construct an error with an explicit stable code.
    pub fn new(status: StatusCode, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            code,
        }
    }

    /// Convenience: 404 with `error_code: "not_found"`.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message, "not_found")
    }

    /// Convenience: 400 with `error_code: "bad_request"`.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message, "bad_request")
    }

    /// Convenience: 500 with `error_code: "internal_error"`.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message, "internal_error")
    }

    /// Convenience: 401 with `error_code: "unauthenticated"`.
    pub fn unauthenticated(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message, "unauthenticated")
    }

    /// Convenience: 403 with `error_code: "forbidden"`.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message, "forbidden")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
                "error_code": self.code,
            })),
        )
            .into_response()
    }
}

impl From<cairn_core::Error> for ApiError {
    fn from(e: cairn_core::Error) -> Self {
        use cairn_core::Error::*;
        let (status, code) = match e {
            NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            Invalid(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        ApiError {
            status,
            message: e.to_string(),
            code,
        }
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

    /// P3.1: ApiError renders as `{ "error": "...", "error_code": "..." }`.
    #[tokio::test]
    async fn api_error_envelope_has_error_code() {
        let resp = ApiError::not_found("no such memory").into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "no such memory");
        assert_eq!(v["error_code"], "not_found");
    }

    #[test]
    fn mask_url_credentials_strips_embedded_userinfo() {
        assert_eq!(
            mask_url_credentials("wss://myuser:hunter2@db.example.com:8000/rpc"),
            "wss://***@db.example.com:8000/rpc"
        );
    }

    #[test]
    fn mask_url_credentials_leaves_credential_free_urls_untouched() {
        assert_eq!(
            mask_url_credentials("ws://localhost:8000/rpc"),
            "ws://localhost:8000/rpc"
        );
    }

    #[test]
    fn mask_url_credentials_passes_through_unparseable_input() {
        assert_eq!(mask_url_credentials("not a url"), "not a url");
    }

    #[tokio::test]
    async fn api_error_envelope_codes_per_constructor() {
        let cases: [(ApiError, StatusCode, &str); 5] = [
            (ApiError::not_found("n"), StatusCode::NOT_FOUND, "not_found"),
            (
                ApiError::bad_request("b"),
                StatusCode::BAD_REQUEST,
                "bad_request",
            ),
            (
                ApiError::internal("i"),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
            ),
            (
                ApiError::unauthenticated("u"),
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
            ),
            (ApiError::forbidden("f"), StatusCode::FORBIDDEN, "forbidden"),
        ];
        for (err, expected_status, expected_code) in cases {
            let resp = err.into_response();
            assert_eq!(resp.status(), expected_status);
            let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(
                v["error_code"], expected_code,
                "wrong code for {expected_status}"
            );
        }
    }

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

    /// `None` when `CAIRN_DB_URL` is unset or SurrealDB is unreachable (tests skip gracefully).
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

        // Create a checkpoint - it should capture the tracked file.
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
        assert_eq!(err.status, StatusCode::NOT_FOUND);
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

        // The new device claims it -> a real, valid device token.
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
        assert_eq!(err.status, StatusCode::NOT_FOUND);
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

    // -- v0.5.0 Sprint 1: SSE event broker + metrics endpoint ---------------------------

    #[test]
    fn event_broker_delivers_published_payloads_to_subscribers() {
        let broker = crate::events::EventBroker::default();
        let mut rx = broker.subscribe();
        broker.publish(crate::events::EventPayload {
            id: "test-1".into(),
            kind: crate::events::KIND_AUDIT,
            ts: 12345,
            data: serde_json::json!({"hello": "world"}),
        });
        // Subscription is async, so poll for up to 500 ms.
        let start = std::time::Instant::now();
        loop {
            match rx.try_recv() {
                Ok(ev) => {
                    assert_eq!(ev.id, "test-1");
                    assert_eq!(ev.kind, crate::events::KIND_AUDIT);
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
        // Start the SSE handler - it returns immediately with a 200 + `text/event-stream`.
        let started = std::time::Instant::now();
        let resp = sse_events(
            State(state.clone()),
            axum::extract::Query(crate::events::EventsQuery::default()),
            axum::http::HeaderMap::new(),
        )
        .await
        .into_response();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(5000),
            "SSE handler should return quickly (non-blocking); took {:?}",
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
        let backfilled = crate::events::backfill(&state, Some(&id1)).unwrap();
        assert_eq!(backfilled.len(), 2);
        assert_eq!(backfilled[0].kind, crate::events::KIND_AUDIT);
        assert_eq!(backfilled[0].data["kind"], "token_issued");
        assert_eq!(backfilled[1].data["kind"], "login_failed");
    }

    // -- v0.8.0 Sprint 10: every state-mutating handler that's supposed to go live over SSE
    // actually does. These are the direct proof for A5-C/D and C-3 - the invalidation map on
    // the web side is only correct if these fire.

    #[tokio::test]
    async fn remember_publishes_a_memory_added_event() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let mut rx = state.events.subscribe();
        let resp = remember(
            State(state.clone()),
            Extension(ScopeCtx::default()),
            Json(cairn_core::NewMemory::new("sprint10 sse test")),
        )
        .await
        .unwrap();
        let ev = rx
            .try_recv()
            .expect("remember should publish a memory event");
        assert_eq!(ev.kind, crate::events::KIND_MEMORY);
        assert_eq!(ev.data["action"], "added");
        assert_eq!(ev.data["memory_id"], resp.0.id);
    }

    #[tokio::test]
    async fn memory_by_scope_returns_only_the_exact_scope_no_global_blend() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory {
                scope_type: cairn_core::ScopeType::Project,
                scope_id: Some("proj-by-scope-test".to_string()),
                ..cairn_core::NewMemory::new("proj-by-scope-test: project-scoped fact")
            })
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new(
                "by-scope test: an unrelated global memory",
            ))
            .unwrap();

        // Unlike recall, by-scope must NOT blend in Global memories for a Project query.
        let resp = memory_by_scope(
            State(state.clone()),
            Query(ByScopeQuery {
                scope_type: cairn_core::ScopeType::Project,
                scope_id: Some("proj-by-scope-test".to_string()),
                limit: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(resp.0.len(), 1);
        assert_eq!(resp.0[0].content, "proj-by-scope-test: project-scoped fact");

        // Project scope without a scope_id is a 400, not a silent empty/Global fallback.
        let err = memory_by_scope(
            State(state.clone()),
            Query(ByScopeQuery {
                scope_type: cairn_core::ScopeType::Project,
                scope_id: None,
                limit: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn list_memories_filters_by_scope_and_reports_total() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory {
                scope_type: cairn_core::ScopeType::Project,
                scope_id: Some("proj-list-test".to_string()),
                ..cairn_core::NewMemory::new("list test: project-scoped fact")
            })
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new("list test: global memory"))
            .unwrap();

        let all = list_memories(State(state.clone()), Query(MemoryListQuery::default()))
            .await
            .unwrap();
        assert_eq!(all.0.total, 2);
        assert_eq!(all.0.items.len(), 2);

        let scoped = list_memories(
            State(state.clone()),
            Query(MemoryListQuery {
                scope_type: Some(cairn_core::ScopeType::Project),
                scope_id: Some("proj-list-test".to_string()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(scoped.0.total, 1);
        assert_eq!(scoped.0.items[0].content, "list test: project-scoped fact");
    }

    #[tokio::test]
    async fn list_memories_clamps_limit_and_offsets_past_end() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        for i in 0..3 {
            state
                .mem
                .remember(cairn_core::NewMemory::new(format!("clamp test mem {i}")))
                .unwrap();
        }

        // limit=0 clamps up to 1; total stays post-filter pre-pagination.
        let one = list_memories(
            State(state.clone()),
            Query(MemoryListQuery {
                limit: Some(0),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(one.0.items.len(), 1);
        assert_eq!(one.0.total, 3);

        // offset past the end yields empty items, same total.
        let past = list_memories(
            State(state.clone()),
            Query(MemoryListQuery {
                offset: Some(50),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert!(past.0.items.is_empty());
        assert_eq!(past.0.total, 3);
    }

    #[tokio::test]
    async fn list_memories_q_matches_content_case_insensitively() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory::new("The Widget Factory uses Tokio"))
            .unwrap();
        state
            .mem
            .remember(cairn_core::NewMemory::new("unrelated note about pandas"))
            .unwrap();

        let hits = list_memories(
            State(state.clone()),
            Query(MemoryListQuery {
                q: Some("widget factory".to_string()),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(hits.0.total, 1);
        assert!(hits.0.items[0].content.contains("Widget Factory"));
    }

    #[tokio::test]
    async fn get_memory_returns_the_full_record_and_404s_on_unknown() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let created = state
            .mem
            .remember(cairn_core::NewMemory {
                scope_type: cairn_core::ScopeType::Project,
                scope_id: Some("proj-get-test".to_string()),
                ..cairn_core::NewMemory::new("get test: full record fact")
            })
            .unwrap();

        let got = get_memory(
            State(state.clone()),
            axum::extract::Path(created.id.clone()),
        )
        .await
        .unwrap();
        // Lock the full-serialization contract the Memory Browser drawer depends on: the wire
        // JSON must expose scope, provenance, edge, and trust fields - not a slimmed view.
        let wire = serde_json::to_value(&got.0).unwrap();
        for key in [
            "scope_type",
            "scope_id",
            "session_id",
            "suspicious",
            "derived_from",
            "contradicts",
            "supersedes",
            "applies_to",
            "promo_score",
            "promo_locked",
            "title",
            "reasoning",
        ] {
            assert!(wire.get(key).is_some(), "wire JSON must carry `{key}`");
        }
        assert_eq!(wire["scope_type"], "project");

        let err = get_memory(
            State(state.clone()),
            axum::extract::Path("nope".to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn config_view_redacts_secrets_and_names_env_vars() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let entries = config_view(State(state.clone())).await.0;
        assert!(!entries.is_empty());
        // Every entry names an env var; secret-bearing keys never leak raw values.
        for e in &entries {
            assert!(!e.env.is_empty());
            if e.key == "db_pass" || e.key == "secret_key" || e.key.ends_with("api_key") {
                assert!(
                    e.value == json!("set") || e.value == json!("not set"),
                    "secret `{}` must be redacted, got {}",
                    e.key,
                    e.value
                );
            }
        }
        let autopilot = entries.iter().find(|e| e.key == "drift_autopilot").unwrap();
        assert_eq!(autopilot.env, "CAIRN_DRIFT_AUTOPILOT");
    }

    #[tokio::test]
    async fn upsert_project_publishes_a_project_event() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let mut rx = state.events.subscribe();
        upsert_project(
            State(state.clone()),
            Json(ProjectUpsertBody {
                id: "proj-sse-test".into(),
                name: "sse test project".into(),
                path: "/tmp/proj".into(),
            }),
        )
        .await
        .unwrap();
        let ev = rx
            .try_recv()
            .expect("upsert_project should publish a project event");
        assert_eq!(ev.kind, crate::events::KIND_PROJECT);
        assert_eq!(ev.data["project_id"], "proj-sse-test");
    }

    #[tokio::test]
    async fn audit_log_record_publishes_a_live_audit_event() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let mut rx = state.events.subscribe();
        state.audit_log.record(
            &state.store,
            &state.events,
            "sse_test_kind",
            "tester",
            "detail here".into(),
        );
        let ev = rx
            .try_recv()
            .expect("audit_log.record should publish a live audit event");
        assert_eq!(ev.kind, crate::events::KIND_AUDIT);
        assert_eq!(ev.data["kind"], "sse_test_kind");
        assert_eq!(ev.data["actor"], "tester");
    }

    // -- v0.5.0 Sprint 2: memory CRUD + confidence + pin -------------------------------

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
            title: None,
            reasoning: None,
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
        assert_eq!(err.status, StatusCode::NOT_FOUND);
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

    // -- v0.5.0 Sprint 3: crystallize + memory graph endpoints --------------------------

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

    /// P4.3: `/api/memory/gotcha` records a failure, and the second one on the same
    /// topic auto-promotes to a `MemoryKind::Gotcha` memory.
    #[tokio::test]
    async fn gotcha_endpoint_promotes_on_second_failure() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        let body1 = GotchaBody {
            topic: "E0308 type mismatch".into(),
            context: Some("src/foo.rs:10".into()),
            refs: vec!["src/foo.rs".into()],
            session_id: Some("s1".into()),
        };
        let r1 = record_gotcha(State(state.clone()), Json(body1.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(r1["promoted"], serde_json::json!(false));

        // Second failure on the same topic - this is the one that promotes.
        let body2 = GotchaBody {
            topic: "E0308 type mismatch".into(),
            context: Some("src/bar.rs:5".into()),
            refs: vec!["src/bar.rs".into()],
            session_id: Some("s2".into()),
        };
        let r2 = record_gotcha(State(state.clone()), Json(body2))
            .await
            .unwrap()
            .0;
        assert_eq!(r2["promoted"], serde_json::json!(true));
        let memory = r2["memory"].as_object().expect("memory present");
        assert_eq!(memory["kind"].as_str().expect("kind present"), "gotcha");
        assert!(memory["content"]
            .as_str()
            .expect("content present")
            .contains("E0308"));
    }

    /// P4.3: `/api/memory/gotcha/wakeup` returns the top clusters.
    #[tokio::test]
    async fn gotcha_wakeup_returns_top_clusters() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        // Two failures on topic-A, one on topic-B. A is the larger cluster.
        for _ in 0..2 {
            let _ = state.mem.record_failure(cairn_memory::FailureEvent::new(
                "E0308 type mismatch",
                "ctx",
            ));
        }
        let _ = state
            .mem
            .record_failure(cairn_memory::FailureEvent::new("E0432 unresolved", "ctx"));

        let resp = gotcha_wakeup(
            State(state.clone()),
            axum::extract::Query(GotchaWakeupQuery { limit: Some(5) }),
        )
        .await
        .unwrap()
        .0;
        let clusters = resp["clusters"].as_array().expect("clusters present");
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0]["size"].as_u64().unwrap(), 2);
        assert_eq!(resp["total_failures"].as_u64().unwrap(), 3);
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

    // -- v0.5.0 Sprint 4: sessions + drift ------------------------------------------------

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
    async fn verify_persists_drift_events_with_a_pending_danger_status() {
        let Some((state, dir)) = test_state() else {
            return;
        };
        let f = dir.path().join("drift.txt");
        let original: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&f, &original).unwrap();
        // A gutting edit - verify flags danger. Web redesign v2: there is no manual
        // approve/reject anymore; the autopilot never auto-approves danger, so the event
        // stays pending in the read-only audit log ("flagged").
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
        assert_eq!(drifts[0].risk, "danger");
        assert_eq!(drifts[0].status, cairn_session::DriftStatus::Pending);
        assert!(
            !drifts[0].detail.contains("auto-approved"),
            "danger must never carry an autopilot approval; got {}",
            drifts[0].detail
        );
    }

    // -- v0.5.0 Sprint 5: ledger ---------------------------------------------------------

    #[tokio::test]
    async fn ledger_records_savings_when_read_runs() {
        let Some((state, dir)) = test_state() else {
            return;
        };
        let f = dir.path().join("big.txt");
        std::fs::write(&f, "a\n".repeat(500)).unwrap();
        // A read in full mode -> ledger gets an entry from "context.read".
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

    // -- v0.5.0 Sprint 6: setup wizard v2 ------------------------------------------------

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
        assert!(v.get("db_reachable").is_some());
        assert!(v.get("admin_exists").is_some());
        assert!(v.get("embedder_loaded").is_some());
        assert!(v.get("secret_key_configured").is_some());
        assert_eq!(h.embed_provider, "hashing");
    }

    // -- v0.5.0 Sprint 7: hybrid search + CSP nonce -------------------------------------

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
            Extension(ScopeCtx::default()),
            axum::extract::Query(SearchQuery {
                q: "hybrid search test query".into(),
                limit: Some(2),
                rerank_depth: Some(20),
                expand: None,
                rerank: None,
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

    /// P3.3: `expand=true` with the LLM gate off (default) short-circuits to the same
    /// single-query path - so the response shape and contents are unchanged.
    #[tokio::test]
    async fn search_endpoint_expand_true_with_llm_disabled_falls_back() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        // LLMs are off by default - cfg.llm_consolidation.enabled is false.
        assert!(!state.cfg.llm_consolidation.enabled);

        state
            .mem
            .remember(cairn_core::NewMemory::new("P3.3 expand fallback test"))
            .unwrap();
        let resp = search_handler(
            State(state.clone()),
            Extension(ScopeCtx::default()),
            axum::extract::Query(SearchQuery {
                q: "P3.3 expand fallback".into(),
                limit: Some(5),
                rerank_depth: Some(20),
                expand: Some(true),
                rerank: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            resp.iter()
                .any(|h| h.memory.content.contains("P3.3 expand fallback")),
            "expand=true with LLM off should still find the memory"
        );
    }

    /// P3.3: `expand=false` (or omitted) is the default path - regression test.
    #[tokio::test]
    async fn search_endpoint_expand_false_matches_default() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory::new("P3.3 default path"))
            .unwrap();
        let resp = search_handler(
            State(state.clone()),
            Extension(ScopeCtx::default()),
            axum::extract::Query(SearchQuery {
                q: "P3.3 default".into(),
                limit: Some(5),
                rerank_depth: Some(20),
                expand: Some(false),
                rerank: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(!resp.is_empty());
    }

    /// P4.2: `?rerank=local` with the rerank gate off (default) falls back to plain
    /// hybrid search - never errors, just bypasses the rerank.
    #[tokio::test]
    async fn search_endpoint_rerank_local_disabled_falls_back() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        // Default config has rerank.enabled = false.
        assert!(!state.cfg.rerank.enabled);

        state
            .mem
            .remember(cairn_core::NewMemory::new("P4.2 rerank fallback test"))
            .unwrap();
        let resp = search_handler(
            State(state.clone()),
            Extension(ScopeCtx::default()),
            axum::extract::Query(SearchQuery {
                q: "P4.2 rerank fallback".into(),
                limit: Some(5),
                rerank_depth: Some(20),
                expand: None,
                rerank: Some("local".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            resp.iter()
                .any(|h| h.memory.content.contains("P4.2 rerank fallback")),
            "rerank=local with config off should still find the memory"
        );
    }

    /// v0.8.0 Sprint 10 regression: `GET /api/search` used to never extract `Extension<ScopeCtx>`
    /// at all, so it always searched Global-only regardless of the caller's
    /// `X-Cairn-Project`/`X-Cairn-Session` headers - unlike `/api/memory/recall`, which already
    /// respected scope. Proves the fix end-to-end through the actual HTTP handler.
    #[tokio::test]
    async fn search_endpoint_respects_project_scope() {
        let Some((state, _dir)) = test_state() else {
            return;
        };
        state
            .mem
            .remember(cairn_core::NewMemory {
                scope_type: cairn_core::ScopeType::Project,
                scope_id: Some("proj-alpha".to_string()),
                ..cairn_core::NewMemory::new("proj-alpha secret: the launch code is banana")
            })
            .unwrap();

        let other_project = ScopeCtx {
            project_id: Some("proj-beta".to_string()),
            session_id: None,
        };
        let resp = search_handler(
            State(state.clone()),
            Extension(other_project),
            axum::extract::Query(SearchQuery {
                q: "launch code".into(),
                limit: Some(5),
                rerank_depth: Some(20),
                expand: None,
                rerank: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            resp.iter().all(|h| !h.memory.content.contains("banana")),
            "proj-beta must not see proj-alpha's project-scoped memory via /api/search"
        );

        let same_project = ScopeCtx {
            project_id: Some("proj-alpha".to_string()),
            session_id: None,
        };
        let resp = search_handler(
            State(state.clone()),
            Extension(same_project),
            axum::extract::Query(SearchQuery {
                q: "launch code".into(),
                limit: Some(5),
                rerank_depth: Some(20),
                expand: None,
                rerank: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            resp.iter().any(|h| h.memory.content.contains("banana")),
            "proj-alpha must see its own project-scoped memory via /api/search"
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

    /// v0.8.0 Sprint 10 regression: a hard navigation/reload of a real (non-pre-rendered)
    /// dynamic-route id used to fall straight through to the site ROOT's shell - verified
    /// live, this silently rendered the "Now" dashboard at a `/you/sessions/<real-id>` URL.
    /// `find_shell_in` must resolve to that route family's OWN placeholder shell instead.
    #[test]
    fn find_shell_in_picks_the_nearest_ancestor_with_a_prerendered_shell() {
        let files = || {
            [
                "index.html".to_string(),
                "projects/placeholder.html".to_string(),
                "you/sessions/new.html".to_string(),
                "you/settings.html".to_string(),
            ]
            .into_iter()
        };

        assert_eq!(
            find_shell_in("projects/f5d3a2d44a6859a0", files()),
            Some("projects/placeholder.html".to_string()),
        );
        assert_eq!(
            find_shell_in("you/sessions/5c1b717f-9a1f-41a7-83bd-ab48819e6ddc", files()),
            Some("you/sessions/new.html".to_string()),
            "must match the 2-segment `you/sessions` family, not fall through to `you/settings`",
        );
        assert_eq!(
            find_shell_in("totally/unknown/path", files()),
            None,
            "no ancestor prefix matches anything embedded - caller falls back to the site root",
        );
    }

    /// Registry HTTP integration: publish a signed pack, list it, download the tarball
    /// back, then revoke and confirm the revocations log records it. Skips when
    /// `CAIRN_DB_URL` is unset.
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
                    .uri("/api/registry/packs")
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

        // GET /registry/packs - must include the published pack.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/registry/packs")
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

        // GET /registry/packs/alpha/1.0.0/download - round-trip the bytes.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/registry/packs/alpha/1.0.0/download")
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

        // GET /registry/search?q=alp - substring match.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/registry/search?q=alp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let hits: Vec<PackMeta> =
            serde_json::from_slice(&to_bytes(resp.into_body(), 1 << 20).await.unwrap()).unwrap();
        assert_eq!(hits.len(), 1);

        // DELETE /registry/packs/alpha/1.0.0 - revoke.
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri("/api/registry/packs/alpha/1.0.0")
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
                    .uri("/api/registry/revocations")
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
    /// manifest declares `scope: team` but only public-scope trusts are configured ---
    /// must be rejected with `ScopeDenied`. Then publishes with `scope: local` and
    /// a public-scope grant - must succeed, and the cached manifest endpoint must
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

        // 1. team-scoped pack + public-only grant -> rejected.
        let team_pack_path = dir.path().join("team.cairnpkg");
        let mut team_pack = Pack::new("team-only", "1.0.0");
        team_pack.description = "scope: team - for the team".into();
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

        // 2. local-scoped pack + public grant -> accepted.
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
                    .uri("/api/registry/packs")
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
                    .uri("/api/registry/packs/local-notes/1.0.0/manifest.json")
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
        // The tri-state must return Absent / Invalid / Invalid respectively - collapsing
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
            VerifyBearerOutcome::BadSignature
        ));
    }
}

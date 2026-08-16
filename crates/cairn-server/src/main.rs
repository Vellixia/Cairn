//! `cairn-server` — the small central server behind shared project memory.
//!
//! It stores only the allowlist in FR-055. There is no observations table and
//! no route that would accept observation content.

mod api;
mod auth;
mod db;
mod error;
mod sync;
mod version;

use axum::http::{header, Method};
use axum::Router;
use clap::Parser;
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    /// Shared knowledge of the newest published release.
    pub releases: version::ReleaseCache,
    /// Whether the session cookie may only travel over HTTPS.
    pub secure_cookies: bool,
    /// The highest migration this database has applied (FR-415).
    pub schema_version: i64,
}

impl AppState {
    /// The `Secure` attribute, or nothing.
    ///
    /// Marking the cookie `Secure` on a plain-HTTP deployment would make the
    /// browser drop it and nobody could sign in, so this follows the origin the
    /// site is actually served from rather than being hardcoded.
    pub fn cookie_secure_attr(&self) -> &'static str {
        if self.secure_cookies {
            "; Secure"
        } else {
            ""
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "cairn-server", about = "Cairn shared memory server", version)]
struct Args {
    /// PostgreSQL connection string.
    #[arg(
        long,
        env = "DATABASE_URL",
        default_value = "postgres://cairn:cairn@localhost:5433/cairn"
    )]
    database_url: String,
    /// Address to listen on.
    #[arg(long, env = "CAIRN_SERVER_ADDR", default_value = "127.0.0.1:8080")]
    addr: String,
    /// Origin allowed to call the API from a browser (the web UI).
    ///
    /// Also decides whether the session cookie is marked `Secure`: an `https`
    /// origin means the cookie must never travel in clear.
    #[arg(long, env = "CAIRN_WEB_ORIGIN")]
    web_origin: Option<String>,
    /// Size of the PostgreSQL connection pool.
    ///
    /// The default suits a deployment where this server owns its database. The
    /// end-to-end suite runs many servers at once against one PostgreSQL and
    /// asks each for a small share, so they do not exhaust it between them.
    #[arg(long, env = "CAIRN_SERVER_MAX_CONNECTIONS", default_value_t = db::DEFAULT_MAX_CONNECTIONS)]
    max_connections: u32,
    /// Email of the account defined by the environment.
    ///
    /// Set it with `--admin-password` and the server creates that account on
    /// start, so a fresh deployment has someone able to sign in. Leave both
    /// unset and the server seeds nothing.
    #[arg(long, env = "CAIRN_ADMIN_EMAIL")]
    admin_email: Option<String>,
    /// Password for the environment-defined account. At least 8 characters.
    ///
    /// Re-applied on every start: this variable is the account's password, not
    /// merely its initial one.
    #[arg(long, env = "CAIRN_ADMIN_PASSWORD")]
    admin_password: Option<String>,
    /// Display name for the environment-defined account.
    #[arg(long, env = "CAIRN_ADMIN_DISPLAY_NAME", default_value = "Admin")]
    admin_display_name: String,
    /// Highest migration to apply, for a staged rollout.
    ///
    /// The binary ships first and the schema moves when the operator is ready.
    /// A held-back deployment advertises the capabilities of the schema it
    /// actually applied, so a peer queues only work this server can hold and
    /// retains the rest until the migration runs (FR-415).
    #[arg(long, env = "CAIRN_MAX_SCHEMA_VERSION", default_value_t = db::SCHEMA_VERSION)]
    max_schema_version: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing();

    // Migrations run on start, so a fresh deployment needs no separate step.
    let pool = db::connect(
        &args.database_url,
        args.max_connections,
        args.max_schema_version,
    )
    .await?;
    seed_admin(&pool, &args).await?;

    // What the database actually holds, not what this binary could apply. A
    // held-back deployment must advertise the smaller answer.
    let schema_version = db::applied_version(&pool).await?;
    if schema_version < db::SCHEMA_VERSION {
        tracing::warn!(
            schema_version,
            binary_supports = db::SCHEMA_VERSION,
            "running below this build's schema; peers will retain work this server cannot hold"
        );
    }

    let secure_cookies = args
        .web_origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("https://"));
    if !secure_cookies {
        tracing::warn!(
            "session cookies are not marked Secure; set CAIRN_WEB_ORIGIN to an https origin"
        );
    }
    let state = AppState {
        pool,
        secure_cookies,
        releases: version::ReleaseCache::new(),
        schema_version,
    };

    // A credentialed browser request cannot be paired with wildcard headers or
    // methods, so the web-origin branch names both explicitly.
    let cors = match &args.web_origin {
        Some(origin) => CorsLayer::new()
            .allow_origin(origin.parse::<axum::http::HeaderValue>()?)
            .allow_credentials(true)
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::COOKIE])
            .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS]),
        None => CorsLayer::new()
            .allow_origin(Any)
            .allow_headers(Any)
            .allow_methods(Any),
    };

    let app: Router = api::routes()
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&args.addr).await?;
    tracing::info!(addr = %args.addr, "cairn-server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Apply the environment-defined account, before the listener binds.
///
/// A deployment whose operator credentials are malformed has nobody able to
/// sign in, so it fails to start rather than serving traffic that turns anyone
/// away. Half a pair is a mistake worth the same treatment: it reads as
/// configured but seeds nothing.
async fn seed_admin(pool: &PgPool, args: &Args) -> anyhow::Result<()> {
    match (
        configured(args.admin_email.as_deref()),
        configured(args.admin_password.as_deref()),
    ) {
        (Some(email), Some(password)) => {
            let (id, outcome) =
                auth::ensure_admin(pool, email, args.admin_display_name.trim(), password).await?;
            tracing::info!(
                user = %id,
                email = %email.trim().to_lowercase(),
                outcome = outcome.as_str(),
                "admin account defined by the environment"
            );
            Ok(())
        }
        (None, None) => Ok(()),
        (Some(_), None) => {
            anyhow::bail!("CAIRN_ADMIN_EMAIL is set but CAIRN_ADMIN_PASSWORD is not")
        }
        (None, Some(_)) => {
            anyhow::bail!("CAIRN_ADMIN_PASSWORD is set but CAIRN_ADMIN_EMAIL is not")
        }
    }
}

/// An empty variable is how a `.env` file and a Compose `environment:` block
/// spell "unset" — the process still receives the name, just with nothing in
/// it. Treating that as configured would fail every default deployment.
fn configured(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("CAIRN_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn"));
    let _ = fmt().with_env_filter(filter).try_init();
}

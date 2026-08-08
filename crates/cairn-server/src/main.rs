//! `cairn-server` — the small central server behind shared project memory.
//!
//! It stores only the allowlist in FR-055. There is no observations table and
//! no route that would accept observation content.

mod api;
mod auth;
mod db;
mod error;
mod sync;

use axum::http::{header, Method};
use axum::Router;
use clap::Parser;
use sqlx::PgPool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

#[derive(Parser, Debug)]
#[command(name = "cairn-server", about = "Cairn shared memory server")]
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
    #[arg(long, env = "CAIRN_WEB_ORIGIN")]
    web_origin: Option<String>,
    /// Size of the PostgreSQL connection pool.
    ///
    /// The default suits a deployment where this server owns its database. The
    /// end-to-end suite runs many servers at once against one PostgreSQL and
    /// asks each for a small share, so they do not exhaust it between them.
    #[arg(long, env = "CAIRN_SERVER_MAX_CONNECTIONS", default_value_t = db::DEFAULT_MAX_CONNECTIONS)]
    max_connections: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing();

    // Migrations run on start, so a fresh deployment needs no separate step.
    let pool = db::connect(&args.database_url, args.max_connections).await?;
    let state = AppState { pool };

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

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_env("CAIRN_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn"));
    let _ = fmt().with_env_filter(filter).try_init();
}

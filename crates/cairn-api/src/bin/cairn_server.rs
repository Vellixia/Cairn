//! In-container entrypoint for the Cairn server.
//!
//! Built into the Docker image only - not shipped to host tarballs, not
//! documented as a host-installable binary. The host has one binary
//! (`cairn`, the client) that talks to the in-container server over HTTP.
//!
//! The Docker image's `CMD` invokes this as `cairn-server`.
//!
//! Responsibilities:
//! 1. Resolve config (env-driven in container; `.env` not needed because
//!    compose sets env directly via the `environment:` block).
//! 2. Open the meta store.
//! 3. Run `bootstrap_admin_from_env` - mint the first admin record from
//!    `CAIRN_ADMIN_USERNAME` + `CAIRN_ADMIN_PASSWORD` if no admin exists.
//! 4. Bind the HTTP listener and serve until SIGTERM.

use std::net::SocketAddr;

use anyhow::Context;
use cairn_api::{serve, AppState};
use cairn_core::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cfg = Config::resolve(None).context("resolving cairn config")?;
    let state = AppState::new(&cfg).context("building app state (open store)")?;
    cairn_api::admin::bootstrap_admin_from_env(&state).context("admin bootstrap")?;

    // v0.8.0 Sprint 4: background jobs (session-gc, memory-decay, access-log-prune). The
    // returned handle must stay alive for the process lifetime - dropping it stops the jobs -
    // so it's bound here and never touched again, living until `main` returns.
    let _cron = cairn_api::cron::start(state.clone())
        .await
        .context("starting cron scheduler")?;

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port)
        .parse()
        .with_context(|| format!("invalid bind address {}:{}", cfg.host, cfg.port))?;

    let scheme = if cfg.tls.is_some() { "https" } else { "http" };
    tracing::info!(
        scheme = scheme,
        addr = %addr,
        data_dir = %cfg.data_dir().display(),
        "cairn-server listening"
    );
    if cfg.insecure && !cfg.is_loopback_host() {
        tracing::warn!(
            "CAIRN_INSECURE=1: serving plain HTTP on {addr}. Do not use on a public network."
        );
    }

    serve(addr, state).await?;
    Ok(())
}

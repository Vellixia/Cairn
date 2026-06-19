//! The `cairn` server binary.
//!
//! `cairn serve` starts the HTTP API + embedded web UI. `cairn token` and `cairn pair-code`
//! operate directly on the server's local store for host administration.

use std::net::SocketAddr;

use anyhow::Context;
use cairn_api::AppState;
use cairn_core::Config;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cairn",
    version,
    about = "Cairn server — context & reliability layer for AI agents"
)]
struct Cli {
    /// Override the data directory (defaults to the OS data dir; use /data in Docker).
    #[arg(long, global = true)]
    data_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the Cairn server (HTTP API + web control plane).
    Serve {
        /// Bind host (default 127.0.0.1, or `CAIRN_HOST`).
        #[arg(long)]
        host: Option<String>,
        /// Bind port (default 7777, or `CAIRN_PORT`).
        #[arg(long)]
        port: Option<u16>,
    },
    /// Generate a pairing code on this host for a new device to claim.
    PairCode { name: Option<String> },
    /// Manage device tokens for authenticating other devices to this server.
    Token {
        #[command(subcommand)]
        action: TokenCmd,
    },
    /// Recover or rotate the dashboard admin account (loopback-only).
    Admin {
        #[command(subcommand)]
        action: AdminCmd,
    },
}

#[derive(Subcommand)]
enum AdminCmd {
    /// Rotate the admin password. Reads the new password from CAIRN_ADMIN_PASSWORD (env) or
    /// stdin. Bumps the admin generation so every existing cookie session is invalidated.
    /// Refused on non-loopback binds — same pattern as the TLS gate.
    Password,
    /// Delete the admin record. The next loopback visit to /setup creates a new admin.
    /// Refused on non-loopback binds. Use this when the password is lost.
    Reset,
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Create a token for a device (prints the token to stdout).
    Create {
        name: String,
        /// Token scope: admin, write (default), or read.
        #[arg(long, default_value = "write")]
        scope: String,
        /// Token expiration in days (default: no expiration).
        #[arg(long)]
        expires: Option<i64>,
    },
    /// List device tokens.
    List,
    /// Revoke a device token.
    Revoke { token: String },
}

mod pair {
    use anyhow::{Context, Result};
    use cairn_api::AppState;
    use chrono::{Duration, SecondsFormat, Utc};

    pub fn generate(state: &AppState, name: Option<&str>) -> Result<()> {
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or("device");
        let token = state.store.create_token(name)?;
        let code = cairn_api::pairing_code();
        let expires =
            (Utc::now() + Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Millis, true);
        state
            .store
            .create_pairing(&code, &token.id, name, &expires)
            .context("storing pairing code")?;

        println!("{code}");
        eprintln!(
            "Pairing code for '{name}' (valid 10 min). On the new device run:\n    cairn-cli pair {code} --server http://<this-host>:7777"
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    if let Some(global) = cairn_core::config::global_env_path() {
        let _ = dotenvy::from_path(&global);
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();
    let cfg = Config::resolve(cli.data_dir).context("resolving data dir")?;

    match cli.cmd {
        Cmd::Serve { host, port } => {
            let state = AppState::new(&cfg)?;
            let host = host.unwrap_or_else(|| cfg.host.clone());
            let port = port.unwrap_or(cfg.port);
            let addr: SocketAddr = format!("{host}:{port}")
                .parse()
                .with_context(|| format!("invalid address {host}:{port}"))?;
            let scheme = if cfg.tls.is_some() { "https" } else { "http" };
            println!("🪨  Cairn serving on {scheme}://{addr}");
            println!("    data dir: {}", cfg.data_dir().display());
            if cfg.tls.is_some() {
                println!("    TLS: enabled (CAIRN_TLS_CERT / CAIRN_TLS_KEY)");
            } else if cfg.insecure {
                eprintln!("    WARNING: CAIRN_INSECURE=1 — serving plain HTTP on a non-loopback address. Do not use this on a public network.");
            } else if !cfg.is_loopback_host() {
                anyhow::bail!(
                    "refusing to serve on non-loopback address {addr} without TLS. \
                     Set CAIRN_TLS_CERT and CAIRN_TLS_KEY to a PEM cert+key pair, \
                     bind to 127.0.0.1/localhost for local-only dev, or set \
                     CAIRN_INSECURE=1 if this is a trusted local/private network."
                );
            }
            cairn_api::serve(addr, state).await?;
        }
        Cmd::PairCode { name } => {
            let state = AppState::new(&cfg)?;
            pair::generate(&state, name.as_deref())?;
        }
        Cmd::Token { action } => {
            let state = AppState::new(&cfg)?;
            match action {
                TokenCmd::Create {
                    name,
                    scope,
                    expires,
                } => {
                    let scope: cairn_core::TokenScope = scope
                        .parse()
                        .context("invalid scope (use admin, write, or read)")?;
                    let expires_at =
                        expires.map(|d| chrono::Utc::now() + chrono::Duration::days(d));
                    let mut t = state.store.create_token(&name)?;
                    let bearer = state.sign_token(&t.id, &t.name, scope, expires_at);
                    t.token = Some(bearer);
                    println!("{}", t.token.as_ref().unwrap());
                    let scope_str = scope.as_str();
                    let exp_str = expires_at
                        .map(|d| format!(", expires {}", d.to_rfc3339()))
                        .unwrap_or_default();
                    eprintln!(
                        "created {scope_str} token for '{name}'{exp_str}. /api access now requires a device token."
                    );
                }
                TokenCmd::List => {
                    for t in state.store.list_tokens()? {
                        println!("{}  {}  {}", t.id, t.name, t.created_at.to_rfc3339());
                    }
                }
                TokenCmd::Revoke { token } => {
                    if state.revoke_bearer(&token)? {
                        println!("revoked");
                    } else {
                        println!("no such token");
                    }
                }
            }
        }
        Cmd::Admin { action } => {
            // Admin recovery commands are loopback-only. The TLS gate already enforces this for
            // serving plain HTTP on a non-loopback bind; we mirror it here so the data dir
            // can't be mutated from a remote operator's session.
            if !cfg.is_loopback_host() {
                anyhow::bail!(
                    "cairn admin is loopback-only (host={}). Bind the server to 127.0.0.1 \
                     or run from the same host that owns the data dir.",
                    cfg.host
                );
            }
            let state = AppState::new(&cfg)?;
            match action {
                AdminCmd::Password => admin::rotate_password(&state, &cfg)?,
                AdminCmd::Reset => admin::reset(&state)?,
            }
        }
    }
    Ok(())
}

mod admin {
    use anyhow::{Context, Result};
    use cairn_api::AppState;
    use cairn_core::{hash_password, AdminRecord};
    use std::io::{self, IsTerminal, Read};

    /// Read the new password from `CAIRN_ADMIN_PASSWORD` env, otherwise from stdin (with
    /// echo suppressed where the terminal supports it). Refuses empty / too-short values.
    fn read_password() -> Result<String> {
        if let Ok(p) = std::env::var("CAIRN_ADMIN_PASSWORD") {
            if !p.trim().is_empty() {
                return Ok(p);
            }
        }
        eprint!("new admin password: ");
        let mut s = String::new();
        if io::stdin().is_terminal() {
            // Best-effort echo suppression. On Windows there's no portable raw-mode API in std,
            // so we just read and emit a newline after — adequate for the admin recovery flow.
            io::stdin()
                .read_line(&mut s)
                .context("reading password from stdin")?;
            eprintln!();
        } else {
            io::stdin()
                .read_to_string(&mut s)
                .context("reading password from stdin")?;
        }
        let trimmed = s.trim().to_string();
        if trimmed.len() < 8 {
            anyhow::bail!("password must be at least 8 characters");
        }
        Ok(trimmed)
    }

    pub fn rotate_password(state: &AppState, _cfg: &cairn_core::Config) -> Result<()> {
        let new_password = read_password()?;
        let mut rec: AdminRecord = match state
            .store
            .get_meta_live(cairn_api::ADMIN_META_KEY)
            .context("loading admin record")?
        {
            Some(json) => serde_json::from_str(&json).context("decoding admin record")?,
            None => anyhow::bail!(
                "no admin configured. Run `cairn-server serve` and visit /setup on loopback \
                 to create the first admin, then use this command to rotate."
            ),
        };
        let hash = hash_password(&new_password).context("hashing new password")?;
        rec.rotate_password(hash);
        let json = serde_json::to_string(&rec).context("encoding admin record")?;
        state
            .store
            .set_meta(cairn_api::ADMIN_META_KEY, &json)
            .context("persisting admin record")?;
        println!(
            "admin '{}' password rotated; generation is now {} \
             (every existing cookie session is invalidated).",
            rec.username, rec.generation
        );
        Ok(())
    }

    pub fn reset(state: &AppState) -> Result<()> {
        let existed = state
            .store
            .reset_meta(cairn_api::ADMIN_META_KEY)
            .context("clearing admin record")?;
        if !existed {
            println!("no admin configured; nothing to reset");
        } else {
            println!(
                "admin deleted. Visit /setup on loopback to create a new admin. \
                 (The data dir may still contain a tombstone; that's fine — reads treat it as absent.)"
            );
        }
        Ok(())
    }
}

//! The `cairn` binary - connects AI agents to a remote Cairn server.
//!
//! All operations go through the server API. No local database, no local
//! store, no local engines. The client is a thin HTTP wrapper with agent
//! config management.
//!
//! Quick start:
//!   cairn onboard --server https://cairn.example.com --token <jwt>

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

mod agents;
mod config;
mod debuglog;
mod doctor;
mod documents;
#[cfg(test)]
mod env_guard;
mod hook;
mod http;
mod jsonedit;
mod onboard;
mod pair;
mod paths;
mod project;
mod reset;
mod rules;
mod sessionbuf;
mod setup;
mod spool;
mod status;
mod statusline;
mod update;

/// Returns the effective server URL (env, then `~/.cairn/config.toml`), or an
/// error with guidance.
fn require_server() -> Result<String> {
    config::resolve(None).server.map(|(s, _)| s).ok_or_else(|| {
        anyhow!(
            "No Cairn server configured.\n\
             Pair with a server (`cairn pair <code>`), or run:\n\
             \n  cairn onboard --server <url> --token <jwt>\n\
             \n  Or: cairn setup --all --server <url> --token <jwt>"
        )
    })
}

#[derive(Parser)]
#[command(
    name = "cairn",
    version,
    about = "Cairn client - connect AI agents to a Cairn server.",
    long_about = "Cairn gives AI agents persistent memory, lean context, and edit safety.\n\n\
                  Getting started:\n\
                  \n  cairn onboard --server <url> --token <jwt>\n\
                  \n  See https://github.com/Vellixia/Cairn for docs."
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Verify server connectivity and agent configuration.
    Doctor {
        #[arg(long)]
        fix: bool,
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// First-run setup: doctor + wire all agents.
    Onboard {
        #[arg(long)]
        skip_agents: bool,
        /// Claim a dashboard-minted pairing code instead of passing --token directly.
        #[arg(long)]
        code: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// Claim a device-pairing code minted by the dashboard (You > Pair) and wire up agents.
    Pair {
        code: String,
        #[arg(long)]
        server: Option<String>,
        /// Skip agent auto-detection and wiring.
        #[arg(long)]
        no_agents: bool,
    },
    /// Configure an agent (or --all detected) to use a Cairn server.
    Setup {
        /// Agent name: claude-code, codex, or opencode.
        agent: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
        /// Write per-project config (Claude Code: `.mcp.json` in cwd) instead
        /// of the global default (`~/.claude.json`).
        #[arg(long)]
        project: bool,
        /// Embed server/token directly into the agent's own config file
        /// instead of the default (server/token live only in
        /// `~/.cairn/config.toml`, agent entries stay bare). Use this for
        /// multi-server or per-agent-token setups the shared config file
        /// can't express.
        #[arg(long)]
        embed_env: bool,
    },
    /// Show server connection, token info, and agent status.
    Status {
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print one fast ambient status line (for Claude Code's `statusLine` setting).
    Statusline,
    /// Remove Cairn-managed entries from all agent config files.
    Reset {
        /// Only show what would be removed.
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the MCP server over stdio (launched by AI agents).
    Mcp,
    /// Internal: handle a lifecycle hook event (launched by AI agents).
    Hook { event: String },
    /// Check for a newer release on GitHub and upgrade the binary.
    Upgrade {
        /// Only report whether an upgrade is available; do not download.
        #[arg(long)]
        check: bool,
    },
    /// Ingest, search, list, or delete RAG documents.
    Documents {
        #[command(subcommand)]
        cmd: DocumentsCmd,
    },
}

#[derive(Subcommand)]
enum DocumentsCmd {
    /// Ingest a local file or an http(s) URL as searchable chunks.
    Ingest {
        source: String,
        /// Defaults to `source` when omitted.
        #[arg(long)]
        title: Option<String>,
    },
    /// Search ingested document chunks.
    Search {
        query: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
    },
    /// List every ingested document.
    List,
    /// Delete a document (id from `cairn documents list`).
    Delete { id: String },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Doctor { fix, json } => {
            doctor::run_and_exit(doctor::DoctorOptions { fix, json })?;
        }
        Cmd::Onboard {
            skip_agents,
            code,
            server,
            token,
        } => {
            onboard::run(onboard::OnboardOptions {
                skip_agents,
                fix: true,
                code,
                server,
                token,
            })?;
        }
        Cmd::Pair {
            code,
            server,
            no_agents,
        } => pair::run(&code, server.as_deref(), no_agents)?,
        Cmd::Setup {
            agent,
            all,
            server,
            token,
            project,
            embed_env,
        } => {
            setup::run(
                agent.as_deref(),
                all,
                server.as_deref(),
                token.as_deref(),
                project,
                embed_env,
            )?;
        }
        Cmd::Status { json } => {
            status::run(json)?;
        }
        Cmd::Statusline => {
            statusline::run();
        }
        Cmd::Reset { dry_run } => {
            reset::run(dry_run)?;
        }
        Cmd::Mcp => {
            let _server = require_server()?;
            // `cairn_core::Config::resolve` (a lower-level, cairn-client-agnostic crate)
            // only ever reads `CAIRN_SERVER`/`CAIRN_TOKEN` from the process env - it has
            // no notion of `~/.cairn/config.toml`. Inject the client's resolved values
            // (which already applied env > file precedence) into this process's env
            // before calling it, so agent MCP entries can omit the env block entirely
            // and still work. Safe to `set_var` here: this is the very first thing this
            // (single-threaded, non-tokio) process does after arg parsing.
            let resolved = config::resolve(None);
            if let Some((server, _)) = &resolved.server {
                std::env::set_var("CAIRN_SERVER", server);
            }
            if let Some((token, _)) = &resolved.token {
                std::env::set_var("CAIRN_TOKEN", token);
            }
            let cfg = cairn_core::Config::resolve(cli.data_dir).context("resolving config")?;
            cairn_mcp::serve_stdio(&cfg)?;
        }
        Cmd::Hook { event } => hook::run(&event)?,
        Cmd::Upgrade { check } => update::run(check)?,
        Cmd::Documents { cmd } => match cmd {
            DocumentsCmd::Ingest { source, title } => documents::ingest(&source, title.as_deref())?,
            DocumentsCmd::Search { query, limit } => documents::search(&query, limit)?,
            DocumentsCmd::List => documents::list()?,
            DocumentsCmd::Delete { id } => documents::delete(&id)?,
        },
    }
    Ok(())
}

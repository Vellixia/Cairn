//! The `cairn` binary - connects AI agents to a remote Cairn server.
//!
//! All operations go through the server API. No local database, no local
//! store, no local engines. The client is a thin HTTP wrapper with agent
//! config management.
//!
//! Quick start:
//!   cairn onboard --server https://cairn.example.com --token <jwt>

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

mod doctor;
mod documents;
mod hook;
mod onboard;
mod project;
mod reset;
mod rules;
mod setup;
mod spool;
mod status;
mod update;

/// Returns the server URL from CAIRN_SERVER env, or an error with guidance.
fn require_server() -> Result<String> {
    std::env::var("CAIRN_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow!(
                "No Cairn server configured.\n\
                 Set CAIRN_SERVER in your environment, or run:\n\
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
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
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
    },
    /// Show server connection, token info, and agent status.
    Status {
        /// Output machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Doctor { fix, json } => {
            doctor::run_and_exit(doctor::DoctorOptions {
                fix,
                interactive: std::io::stdout().is_terminal(),
                json,
            })?;
        }
        Cmd::Onboard {
            skip_agents,
            server,
            token,
        } => {
            onboard::run(onboard::OnboardOptions {
                skip_agents,
                fix: true,
                server,
                token,
            })?;
        }
        Cmd::Setup {
            agent,
            all,
            server,
            token,
            project,
        } => setup::run(
            agent.as_deref(),
            all,
            server.as_deref(),
            token.as_deref(),
            project,
        )?,
        Cmd::Status { json } => {
            status::run(json)?;
        }
        Cmd::Reset { dry_run } => {
            reset::run(dry_run)?;
        }
        Cmd::Mcp => {
            let _server = require_server()?;
            let cfg = cairn_core::Config::resolve(cli.data_dir).context("resolving config")?;
            cairn_mcp::serve_stdio(&cfg)?;
        }
        Cmd::Hook { event } => hook::run(&event)?,
        Cmd::Upgrade { check } => update::run(check)?,
        Cmd::Documents { cmd } => match cmd {
            DocumentsCmd::Ingest { source, title } => {
                documents::ingest(&source, title.as_deref())?
            }
            DocumentsCmd::Search { query, limit } => documents::search(&query, limit)?,
            DocumentsCmd::List => documents::list()?,
            DocumentsCmd::Delete { id } => documents::delete(&id)?,
        },
    }
    Ok(())
}

//! The `cairn` binary - connects AI agents to a Cairn server.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use cairn_core::Config;
use clap::{Parser, Subcommand};

mod doctor;
mod hook;
mod onboard;
mod rules;
mod setup;
mod update;

#[derive(Parser)]
#[command(name = "cairn", version, about = "Cairn client - connect AI agents to a Cairn server")]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Verify the local setup and server connectivity.
    Doctor {
        #[arg(long)]
        fix: bool,
    },
    /// Zero-prompt setup for first-run installs.
    Onboard {
        #[arg(long)]
        skip_agents: bool,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
    },
    /// Configure an agent (or --all detected agents) to use a Cairn server.
    Setup {
        agent: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
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
}

pub struct State {
    pub store: Arc<cairn_store::Store>,
    pub mem: Arc<cairn_memory::MemoryEngine>,
    pub guard: Arc<cairn_guard::Guard>,
    pub asm: Arc<cairn_assemble::Assembler>,
    pub shell: Arc<cairn_shell::ShellCompressor>,
    pub profile: Arc<cairn_profile::Profile>,
}

impl State {
    fn open(cfg: &Config) -> anyhow::Result<Self> {
        let store = Arc::new(cairn_store::Store::open(cfg)?);
        let mem = Arc::new(cairn_memory::MemoryEngine::new(store.clone()));
        Ok(Self {
            store: store.clone(),
            mem: mem.clone(),
            guard: Arc::new(cairn_guard::Guard::new(store.clone())),
            asm: Arc::new(cairn_assemble::Assembler::new(mem.clone())),
            shell: Arc::new(cairn_shell::ShellCompressor::new(store.clone())),
            profile: Arc::new(cairn_profile::Profile::new(mem)),
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();
    let cfg = Config::resolve(cli.data_dir).context("resolving data dir")?;

    match cli.cmd {
        Cmd::Doctor { fix } => {
            doctor::run_and_exit(doctor::DoctorOptions {
                fix,
                interactive: std::io::stdout().is_terminal(),
            })?;
        }
        Cmd::Onboard { skip_agents, server, token } => {
            onboard::run(onboard::OnboardOptions {
                skip_agents,
                fix: true,
                server,
                token,
            })?;
        }
        Cmd::Setup { agent, all, server, token } => {
            setup::run(agent.as_deref(), all, server.as_deref(), token.as_deref())?
        }
        Cmd::Mcp => {
            cairn_mcp::serve_stdio(&cfg)?;
        }
        Cmd::Hook { event } => hook::run(&cfg, &event)?,
        Cmd::Upgrade { check } => update::run(check)?,
    }
    Ok(())
}

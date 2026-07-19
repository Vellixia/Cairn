//! `cairn` CLI: thin IPC client over the local daemon (research R11).
//! Machine mode: global `--json` (contracts/cli-json-contract.md).

mod commands;
mod ipc;
mod output;
mod token;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cairn", version, about = "Cairn local session foundation")]
struct Cli {
    /// Emit the stable machine-readable JSON envelope on stdout.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize Cairn in the current repository.
    Init,
    /// Inspect exact repository state.
    Status {
        /// Page through the full ignored-file list instead of the summary.
        #[arg(long)]
        ignored: bool,
        /// Pagination cursor for --ignored.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Session lifecycle.
    #[command(subcommand)]
    Session(SessionCommand),
    /// Daemon operations.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Subcommand)]
enum SessionCommand {
    /// Start (or idempotently return) a session for a named agent.
    Start {
        #[arg(long)]
        agent: String,
        #[arg(long, env = "CAIRN_AGENT_INSTANCE")]
        agent_instance: Option<String>,
        #[arg(long)]
        agent_pid: Option<u32>,
    },
    /// Show the active session (adaptive resolution, FR-036).
    Show {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, env = "CAIRN_AGENT_INSTANCE")]
        agent_instance: Option<String>,
        #[arg(long)]
        agent_type: Option<String>,
    },
    /// Send an authenticated heartbeat (token via secure input, never argv).
    Heartbeat {
        #[arg(long)]
        session: String,
        #[arg(long, env = "CAIRN_AGENT_INSTANCE")]
        agent_instance: Option<String>,
        /// Read the resume token from stdin (one line).
        #[arg(long)]
        resume_token_stdin: bool,
        /// Read the resume token from a file.
        #[arg(long)]
        resume_token_file: Option<std::path::PathBuf>,
    },
    /// Reattach to a recovering session (token via secure input, never argv).
    Reattach {
        #[arg(long)]
        session: String,
        #[arg(long, env = "CAIRN_AGENT_INSTANCE")]
        agent_instance: Option<String>,
        #[arg(long)]
        resume_token_stdin: bool,
        #[arg(long)]
        resume_token_file: Option<std::path::PathBuf>,
    },
    /// Stop a session.
    Stop {
        #[arg(long)]
        session: Option<String>,
        #[arg(long, env = "CAIRN_AGENT_INSTANCE")]
        agent_instance: Option<String>,
        #[arg(long)]
        resume_token_stdin: bool,
        #[arg(long)]
        resume_token_file: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Show local daemon status.
    Status,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    let exit = match cli.command {
        Command::Init => commands::init::run(json).await,
        Command::Status { ignored, cursor } => commands::status::run(json, ignored, cursor).await,
        Command::Daemon(DaemonCommand::Status) => commands::daemon::run(json).await,
        Command::Session(cmd) => commands::session::run(json, cmd).await,
    };
    std::process::exit(exit);
}

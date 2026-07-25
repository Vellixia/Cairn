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
    /// Local project management.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Local immutable task revisions.
    #[command(subcommand)]
    Task(TaskCommand),
    /// Daemon operations.
    #[command(subcommand)]
    Daemon(DaemonCommand),
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Create a local project.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// List local projects.
    List {
        #[arg(long, value_parser = ["active", "archived"])]
        status: Option<String>,
        #[arg(long)]
        after_project_id: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show one project by ID, or by an exact name in human mode.
    Show {
        #[arg(long, conflicts_with = "project")]
        project_id: Option<String>,
        #[arg(long, conflicts_with = "project_id")]
        project: Option<String>,
    },
    /// Update project metadata, archive, or restore.
    Update {
        #[arg(long, conflicts_with = "project")]
        project_id: Option<String>,
        #[arg(long, conflicts_with = "project_id")]
        project: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, conflicts_with = "description")]
        clear_description: bool,
        #[arg(long, value_parser = ["active", "archived"])]
        status: Option<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Manage repository associations.
    #[command(subcommand)]
    Repository(ProjectRepositoryCommand),
}

#[derive(Subcommand)]
enum ProjectRepositoryCommand {
    /// Associate a registered repository with a project.
    Add {
        #[arg(long, conflicts_with = "project")]
        project_id: Option<String>,
        #[arg(long, conflicts_with = "project_id")]
        project: Option<String>,
        #[arg(long)]
        repository_id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Subcommand)]
enum TaskCommand {
    /// Create a task and immutable revision 1.
    Create {
        #[arg(long)]
        project_id: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        goal_contract: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Create the next immutable task revision.
    Revise {
        #[arg(long, conflicts_with = "task")]
        task_id: Option<String>,
        #[arg(long, conflicts_with = "task_id")]
        task: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        parent_revision_id: Option<String>,
        #[arg(long)]
        goal_contract: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// List tasks in one project.
    List {
        #[arg(long)]
        project_id: String,
        #[arg(long)]
        after_task_id: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show the latest or one explicit historical revision.
    Show {
        #[arg(long, conflicts_with = "task")]
        task_id: Option<String>,
        #[arg(long, conflicts_with = "task_id")]
        task: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        revision_id: Option<String>,
    },
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
        /// Explicitly request a constitutionally eligible local bootstrap session.
        #[arg(
            long,
            conflicts_with_all = ["project_id", "task_revision_id"]
        )]
        local_unbound: bool,
        /// Start already bound to this project (requires --task-revision-id).
        #[arg(long, requires = "task_revision_id", conflicts_with = "local_unbound")]
        project_id: Option<String>,
        /// Start already bound to this immutable revision (requires --project-id).
        #[arg(long, requires = "project_id", conflicts_with = "local_unbound")]
        task_revision_id: Option<String>,
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
    /// List sessions with additive project/revision scope filters.
    List {
        #[arg(long)]
        repository_id: Option<String>,
        #[arg(long)]
        project_id: Option<String>,
        #[arg(long)]
        task_revision_id: Option<String>,
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
    /// Bind one existing local bootstrap session to a project and immutable task revision.
    Bind {
        #[arg(long)]
        session: String,
        #[arg(long)]
        project_id: String,
        #[arg(long)]
        task_revision_id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Show local daemon status.
    Status,
}

#[tokio::main]
async fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let json = args.iter().any(|arg| arg == "--json");
            let exit = error.exit_code();
            if json && exit != 0 {
                let command = args
                    .iter()
                    .filter(|arg| !arg.starts_with('-'))
                    .take(2)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(".");
                let body = cairn_protocol::ErrorBody {
                    code: cairn_protocol::ErrorCode::Usage,
                    message: "invalid command arguments".into(),
                    data: None,
                };
                std::process::exit(output::emit(&command, true, Err(body)));
            }
            let _ = error.print();
            std::process::exit(exit);
        }
    };
    let json = cli.json;
    let exit = match cli.command {
        Command::Init => commands::init::run(json).await,
        Command::Status { ignored, cursor } => commands::status::run(json, ignored, cursor).await,
        Command::Daemon(DaemonCommand::Status) => commands::daemon::run(json).await,
        Command::Session(cmd) => commands::session::run(json, cmd).await,
        Command::Project(cmd) => commands::project::run(json, cmd).await,
        Command::Task(cmd) => commands::task::run(json, cmd).await,
    };
    std::process::exit(exit);
}

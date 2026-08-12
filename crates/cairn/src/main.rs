//! `cairn` — the developer's interface, the hook runtime, and the MCP server.

mod client;
mod connect;
mod hook;
mod mcp;
mod render;
mod update;

use cairn_core::domain::*;
use cairn_core::wire::*;
use clap::{Parser, Subcommand};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "cairn",
    version,
    about = "Persistent, project-aware memory for AI coding agents"
)]
struct Cli {
    /// Emit the stable JSON envelope instead of human output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register this repository as a Cairn project.
    Init,
    /// Project, repository, sessions and daemon state.
    Status,
    /// Install the Claude Code integration for this repository.
    Connect {
        #[arg(default_value = "claude-code")]
        agent: String,
    },
    /// Remove the Claude Code integration.
    Disconnect {
        #[arg(default_value = "claude-code")]
        agent: String,
    },
    /// Sessions.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Tasks.
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Durable memory.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Read a handoff.
    Handoff {
        #[command(subcommand)]
        action: HandoffAction,
    },
    /// Print the briefing a session would receive.
    Context {
        #[arg(long)]
        budget: Option<usize>,
        /// Which session to brief, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Capture exclusions.
    Privacy {
        #[command(subcommand)]
        action: PrivacyAction,
    },
    /// Delete stored data.
    Delete {
        #[command(subcommand)]
        action: DeleteAction,
    },
    /// Opt this project into server sync.
    Link {
        /// Join an existing shared project by identifier.
        #[arg(long)]
        project: Option<Uuid>,
        /// Create a new shared project.
        #[arg(long)]
        create: bool,
    },
    /// Opt this project back out.
    Unlink,
    /// Server credentials.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Synchronization.
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Daemon lifecycle.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Check for a newer release, and install it.
    Update {
        /// Report what is available without installing anything.
        #[arg(long)]
        check: bool,
    },
    /// Run the MCP server over stdio.
    Mcp,
    /// Claude Code hook entry point. Always exits 0.
    Hook { event: String },
}

#[derive(Subcommand)]
enum SessionAction {
    /// Every session in this project, newest first.
    List,
    Start {
        #[arg(long, default_value = "cairn-cli")]
        agent: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        task: Option<Uuid>,
    },
    Show {
        #[arg(long)]
        session: Option<Uuid>,
    },
    End {
        #[arg(long)]
        session: Option<Uuid>,
        #[arg(long, default_value = "completed")]
        status: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Subcommand)]
enum TaskAction {
    List {
        #[arg(long)]
        status: Option<String>,
    },
    Show {
        id: Uuid,
    },
    New {
        #[arg(long)]
        title: String,
        #[arg(long)]
        goal: String,
        /// Repeatable.
        #[arg(long = "criterion")]
        criteria: Vec<String>,
    },
    SetStatus {
        id: Uuid,
        status: String,
    },
}

#[derive(Subcommand)]
enum MemoryAction {
    Add {
        content: String,
        #[arg(long = "type", default_value = "fact")]
        kind: String,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_key: Option<String>,
        /// Never transmitted, even for a linked project.
        #[arg(long)]
        local_only: bool,
        /// Supporting observation ids. Optional, and never invented.
        #[arg(long = "evidence")]
        evidence: Vec<Uuid>,
        /// Which session recorded this, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
    },
    Search {
        query: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_key: Option<String>,
        #[arg(long = "type")]
        kind: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        limit: Option<i64>,
        /// Which session's task to rank by, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
    },
    Show {
        id: Uuid,
    },
    Forget {
        id: Uuid,
    },
}

#[derive(Subcommand)]
enum HandoffAction {
    Show {
        #[arg(long)]
        session: Option<Uuid>,
    },
}

#[derive(Subcommand)]
enum PrivacyAction {
    Exclude {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    Unexclude {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    List,
}

#[derive(Subcommand)]
enum DeleteAction {
    Observation {
        id: Uuid,
    },
    Memory {
        id: Uuid,
    },
    Handoff {
        id: Uuid,
    },
    Session {
        id: Uuid,
        /// Also delete the memories this session produced. Never the default.
        #[arg(long)]
        with_memories: bool,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Store the personal API token generated in the web UI.
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    Logout,
    /// Whether a credential is stored, and for which server.
    Status,
}

#[derive(Subcommand)]
enum TokenAction {
    Set {
        /// Read from stdin when omitted, so it never lands in shell history.
        token: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Subcommand)]
enum SyncAction {
    Status,
    Now,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Show the daemon's recent log.
    Logs {
        /// How many lines from the end.
        #[arg(long, default_value_t = 50)]
        tail: usize,
    },
    Start,
    Stop,
    Status,
}

/// Exit codes: 0 success, 1 user error, 2 Cairn unavailable.
const EXIT_USER_ERROR: i32 = 1;
const EXIT_UNAVAILABLE: i32 = 2;

fn main() {
    // The capture class is handled before any async runtime is built: a hook
    // runs once per tool call, and a runtime per call is the largest cost
    // Cairn adds to a session (SC-007).
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() >= 3 && argv[1] == "hook" && hook::run_blocking(&argv[2]) {
        std::process::exit(0);
    }
    run_async()
}

#[tokio::main]
async fn run_async() {
    let cli = Cli::parse();

    // The hook entry point is the exception to every rule below: it always
    // exits 0, whatever happened (FR-015).
    if let Command::Hook { event } = &cli.command {
        hook::run(event).await;
        std::process::exit(0);
    }
    if let Command::Mcp = &cli.command {
        if let Err(e) = mcp::serve().await {
            eprintln!("cairn mcp: {e}");
            std::process::exit(EXIT_UNAVAILABLE);
        }
        return;
    }

    match run(&cli).await {
        Ok(output) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Envelope::ok(output.value)).unwrap()
                );
            } else if !output.text.is_empty() {
                print!("{}", output.text);
            }
        }
        Err(e) => {
            let code = exit_code(&e);
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Envelope::err(e)).unwrap()
                );
            } else {
                eprintln!("cairn: {}: {}", e.code, e.message);
            }
            std::process::exit(code);
        }
    }
}

fn exit_code(e: &WireError) -> i32 {
    match e.code.as_str() {
        codes::DAEMON_UNAVAILABLE | codes::STORAGE_UNAVAILABLE | codes::SERVER_UNAVAILABLE => {
            EXIT_UNAVAILABLE
        }
        _ => EXIT_USER_ERROR,
    }
}

struct Output {
    value: serde_json::Value,
    text: String,
}

impl Output {
    fn plain(value: serde_json::Value) -> Self {
        Self {
            value,
            text: String::new(),
        }
    }
    fn with(value: serde_json::Value, text: String) -> Self {
        Self { value, text }
    }
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into())
}

fn parse_enum<T: std::str::FromStr>(what: &str, raw: &str) -> Result<T, WireError>
where
    T::Err: std::fmt::Display,
{
    raw.parse::<T>()
        .map_err(|e| WireError::invalid(format!("bad {what}: {e}")))
}

async fn run(cli: &Cli) -> Result<Output, WireError> {
    match &cli.command {
        Command::Hook { .. } | Command::Mcp => unreachable!("handled in main"),

        Command::Init => {
            let v = client::send(&Request::Init { cwd: cwd() }).await?;
            let name = v["project"]["name"].as_str().unwrap_or("project");
            Ok(Output::with(
                v.clone(),
                format!("Cairn is tracking {name}.\n"),
            ))
        }

        Command::Status => {
            let v = client::send(&Request::Status { cwd: cwd() }).await?;
            let payload: StatusPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            Ok(Output::with(v, render::status(&payload)))
        }

        Command::Connect { agent } => connect::connect(agent).await,
        Command::Disconnect { agent } => connect::disconnect(agent).await,

        Command::Session { action } => session(action).await,
        Command::Task { action } => task(action).await,
        Command::Memory { action } => memory(action).await,

        Command::Handoff { action } => {
            let HandoffAction::Show { session } = action;
            let v = client::send(&Request::HandoffLatest {
                cwd: cwd(),
                session_id: *session,
                agent_session_key: None,
            })
            .await?;
            let h: Handoff = serde_json::from_value(v["handoff"].clone())
                .map_err(|e| WireError::invalid(e.to_string()))?;
            Ok(Output::with(v, render::handoff(&h)))
        }

        Command::Context { budget, session } => {
            let v = client::send(&Request::Context {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                reason: Some(ContextReason::Refresh),
                token_budget: *budget,
            })
            .await?;
            let payload: ContextPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            Ok(Output::with(v, render::briefing(&payload)))
        }

        Command::Privacy { action } => privacy(action).await,
        Command::Delete { action } => delete(action).await,

        Command::Link { project, create } => {
            let v = client::send(&Request::Link {
                cwd: cwd(),
                server_project_id: *project,
                create: *create,
            })
            .await?;
            let text = if v["linked"].as_bool().unwrap_or(false) {
                format!("Linked to shared project {}.\n", v["server_project_id"])
            } else {
                let candidates = v["candidates"].as_array().cloned().unwrap_or_default();
                let mut t = String::from("Not linked.\n");
                if candidates.is_empty() {
                    t.push_str("No shared project matches this repository's remote.\n");
                } else {
                    t.push_str("Shared projects matching this remote:\n");
                    for c in candidates {
                        t.push_str(&format!("  {}  {}\n", c["id"], c["name"]));
                    }
                }
                t.push_str(
                    "Run `cairn link --create` for a new shared project, \
                     or `cairn link --project <id>` to join one.\n",
                );
                t
            };
            Ok(Output::with(v, text))
        }
        Command::Unlink => {
            let v = client::send(&Request::Unlink { cwd: cwd() }).await?;
            Ok(Output::with(
                v,
                "This project is local only again.\n".into(),
            ))
        }

        Command::Auth { action } => auth(action).await,
        Command::Update { check } => update_command(*check).await,
        Command::Sync { action } => sync(action).await,
        Command::Daemon { action } => daemon(action).await,
    }
}

async fn session(action: &SessionAction) -> Result<Output, WireError> {
    match action {
        SessionAction::List => {
            let v = client::send(&Request::SessionList { cwd: cwd() }).await?;
            let sessions: Vec<SessionSummary> =
                serde_json::from_value(v["sessions"].clone()).unwrap_or_default();
            let mut text = String::new();
            if sessions.is_empty() {
                text.push_str("No sessions yet.\n");
            }
            for s in &sessions {
                text.push_str(&format!(
                    "{}  {:<12} {:<14} {}  idle {}s\n",
                    s.id, s.status, s.agent, s.branch, s.idle_seconds
                ));
            }
            Ok(Output::with(v, text))
        }
        SessionAction::Start { agent, key, task } => {
            let v = client::send(&Request::SessionStart {
                cwd: cwd(),
                agent: agent.clone(),
                agent_session_key: key.clone(),
                task_id: *task,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!("Session {} started.\n", v["session"]["id"]),
            ))
        }
        SessionAction::Show { session } => {
            let v = client::send(&Request::SessionShow {
                cwd: cwd(),
                session_id: *session,
                agent_session_key: None,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&v["session"]).unwrap_or_default()
                ),
            ))
        }
        SessionAction::End {
            session,
            status,
            reason,
        } => {
            let status: SessionStatus = parse_enum("status", status)?;
            let v = client::send(&Request::SessionEnd {
                cwd: cwd(),
                session_id: *session,
                agent_session_key: None,
                status,
                reason: reason.clone(),
                // `cairn session end` waits for the durable handoff: nothing
                // holds a deadline over it (D22).
                wait_for_handoff: true,
            })
            .await?;
            Ok(Output::with(v, "Session ended; handoff written.\n".into()))
        }
    }
}

async fn task(action: &TaskAction) -> Result<Output, WireError> {
    match action {
        TaskAction::List { status } => {
            let status = match status {
                Some(s) => Some(parse_enum::<TaskStatus>("status", s)?),
                None => None,
            };
            let v = client::send(&Request::TaskList { cwd: cwd(), status }).await?;
            let tasks: Vec<Task> = serde_json::from_value(v["tasks"].clone()).unwrap_or_default();
            let mut text = String::new();
            if tasks.is_empty() {
                text.push_str("No tasks yet.\n");
            }
            for t in &tasks {
                text.push_str(&format!("{}  {:<12} {}\n", t.id, t.status, t.title));
            }
            Ok(Output::with(v, text))
        }
        TaskAction::Show { id } => {
            let v = client::send(&Request::TaskGet {
                cwd: cwd(),
                task_id: *id,
            })
            .await?;
            let t: Task = serde_json::from_value(v["task"].clone())
                .map_err(|e| WireError::invalid(e.to_string()))?;
            let mut text = format!("{}\n{}\nStatus: {}\n", t.title, t.goal, t.status);
            if !t.acceptance_criteria.is_empty() {
                text.push_str("Acceptance criteria:\n");
                for c in &t.acceptance_criteria {
                    text.push_str(&format!("- {c}\n"));
                }
            }
            Ok(Output::with(v, text))
        }
        TaskAction::New {
            title,
            goal,
            criteria,
        } => {
            let v = client::send(&Request::TaskCreate {
                cwd: cwd(),
                title: title.clone(),
                goal: goal.clone(),
                acceptance_criteria: criteria.clone(),
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!("Task {} created.\n", v["task"]["id"]),
            ))
        }
        TaskAction::SetStatus { id, status } => {
            let status: TaskStatus = parse_enum("status", status)?;
            let v = client::send(&Request::TaskUpdate {
                cwd: cwd(),
                task_id: *id,
                title: None,
                goal: None,
                acceptance_criteria: None,
                status: Some(status),
            })
            .await?;
            Ok(Output::with(v, format!("Task is now {status}.\n")))
        }
    }
}

async fn memory(action: &MemoryAction) -> Result<Output, WireError> {
    match action {
        MemoryAction::Add {
            content,
            kind,
            scope,
            scope_key,
            local_only,
            evidence,
            session,
        } => {
            let kind: MemoryType = parse_enum("type", kind)?;
            let scope = match scope {
                Some(s) => Some(parse_enum::<MemoryScope>("scope", s)?),
                None => None,
            };
            let v = client::send(&Request::MemoryCreate {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                kind,
                scope,
                scope_key: scope_key.clone(),
                content: content.clone(),
                evidence_observation_ids: evidence.clone(),
                local_only: *local_only,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!("Remembered {}.\n", v["memory"]["id"]),
            ))
        }
        MemoryAction::Search {
            query,
            scope,
            scope_key,
            kind,
            state,
            limit,
            session,
        } => {
            let q = MemoryQuery {
                query: query.clone(),
                scope: match scope {
                    Some(s) => Some(parse_enum("scope", s)?),
                    None => None,
                },
                scope_key: scope_key.clone(),
                kind: match kind {
                    Some(k) => Some(parse_enum("type", k)?),
                    None => None,
                },
                state: match state {
                    Some(s) => Some(parse_enum("state", s)?),
                    None => None,
                },
                limit: *limit,
            };
            let v = client::send(&Request::MemorySearch {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                query: q,
            })
            .await?;
            let payload: SearchPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            let mut text = String::new();
            if payload.results.is_empty() {
                text.push_str("No matching memory.\n");
            }
            for r in &payload.results {
                text.push_str(&format!(
                    "{}  [{}/{}] {}\n    from session {} · {} evidence\n",
                    r.id,
                    r.kind,
                    r.scope,
                    r.content,
                    r.provenance.session_id,
                    r.provenance.evidence_count
                ));
            }
            Ok(Output::with(v, text))
        }
        MemoryAction::Show { id } => {
            let v = client::send(&Request::MemoryGet {
                cwd: cwd(),
                memory_id: *id,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&v["memory"]).unwrap_or_default()
                ),
            ))
        }
        MemoryAction::Forget { id } => {
            let v = client::send(&Request::MemoryForget {
                cwd: cwd(),
                memory_id: *id,
            })
            .await?;
            Ok(Output::with(v, "Memory deleted.\n".into()))
        }
    }
}

async fn privacy(action: &PrivacyAction) -> Result<Output, WireError> {
    let request = match action {
        PrivacyAction::Exclude { path, command } => Request::PrivacyExclude {
            cwd: cwd(),
            path: path.clone(),
            command: command.clone(),
        },
        PrivacyAction::Unexclude { path, command } => Request::PrivacyUnexclude {
            cwd: cwd(),
            path: path.clone(),
            command: command.clone(),
        },
        PrivacyAction::List => Request::PrivacyList { cwd: cwd() },
    };
    let v = client::send(&request).await?;
    let mut text = String::from("Excluded paths:\n");
    for p in v["paths"].as_array().cloned().unwrap_or_default() {
        text.push_str(&format!("  {}\n", p.as_str().unwrap_or_default()));
    }
    text.push_str("Excluded commands:\n");
    for c in v["commands"].as_array().cloned().unwrap_or_default() {
        text.push_str(&format!("  {}\n", c.as_str().unwrap_or_default()));
    }
    Ok(Output::with(v, text))
}

async fn delete(action: &DeleteAction) -> Result<Output, WireError> {
    let (target, id, with_memories) = match action {
        DeleteAction::Observation { id } => (DeleteTarget::Observation, *id, false),
        DeleteAction::Memory { id } => (DeleteTarget::Memory, *id, false),
        DeleteAction::Handoff { id } => (DeleteTarget::Handoff, *id, false),
        DeleteAction::Session { id, with_memories } => (DeleteTarget::Session, *id, *with_memories),
    };
    let v = client::send(&Request::Delete {
        cwd: cwd(),
        target,
        id,
        with_memories,
    })
    .await?;
    let mut text = format!("Deleted {id}.\n");
    if target == DeleteTarget::Session && !with_memories {
        text.push_str(
            "The memories and handoffs this session produced were kept; \
             pass --with-memories to remove them too.\n",
        );
    }
    Ok(Output::with(v, text))
}

async fn auth(action: &AuthAction) -> Result<Output, WireError> {
    match action {
        AuthAction::Token {
            action: TokenAction::Set { token, server },
        } => {
            let token = match token {
                Some(t) => t.clone(),
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| WireError::invalid(e.to_string()))?;
                    buf.trim().to_string()
                }
            };
            if token.is_empty() {
                return Err(WireError::invalid("no token supplied"));
            }
            let v = client::send(&Request::AuthTokenSet {
                token,
                server_url: server.clone(),
            })
            .await?;
            Ok(Output::with(v, "Token stored.\n".into()))
        }
        AuthAction::Logout => {
            let v = client::send(&Request::AuthLogout).await?;
            Ok(Output::with(v, "Token removed.\n".into()))
        }
        AuthAction::Status => {
            let v = client::send(&Request::AuthStatus).await?;
            let authenticated = v
                .get("authenticated")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let server = v
                .get("server_url")
                .and_then(|u| u.as_str())
                .unwrap_or("not set");
            let text = format!(
                "Token   {}\nServer  {server}\n",
                if authenticated { "stored" } else { "none" },
            );
            Ok(Output::with(v, text))
        }
    }
}

async fn sync(action: &SyncAction) -> Result<Output, WireError> {
    match action {
        SyncAction::Status => {
            let v = client::send(&Request::SyncStatus { cwd: cwd() }).await?;
            let s: SyncStatusPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            let mut text = format!(
                "Linked       {}\nPending      {}\nFailed       {}\nLast success {}\n",
                if s.linked { "yes" } else { "no" },
                s.pending,
                s.failed,
                s.last_success_at
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "never".into())
            );
            for f in &s.failures {
                text.push_str(&format!(
                    "  failed {} {}: {}\n",
                    f.entity_type, f.entity_id, f.error
                ));
            }
            Ok(Output::with(v, text))
        }
        SyncAction::Now => {
            let v = client::send(&Request::SyncNow { cwd: cwd() }).await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "applied {}, duplicate {}, rejected {}, pulled {}\n",
                    v["applied"], v["duplicate"], v["rejected"], v["pulled"]
                ),
            ))
        }
    }
}

async fn daemon(action: &DaemonAction) -> Result<Output, WireError> {
    match action {
        DaemonAction::Logs { tail } => {
            let path = cairn_core::paths::daemon_log_path();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            let shown: Vec<&str> = lines.iter().rev().take(*tail).rev().copied().collect();
            let body = if shown.is_empty() {
                format!(
                    "No daemon log yet at {}.\nIt fills once the daemon starts.\n",
                    path.display()
                )
            } else {
                format!("{}\n", shown.join("\n"))
            };
            Ok(Output::with(
                serde_json::json!({
                    "path": path.display().to_string(),
                    "lines": shown,
                }),
                body,
            ))
        }
        DaemonAction::Start => {
            if client::daemon_running().await {
                return Ok(Output::with(
                    serde_json::json!({"running": true}),
                    "Already running.\n".into(),
                ));
            }
            client::start_daemon()?;
            let v = client::send(&Request::DaemonStatus).await?;
            Ok(Output::with(v, "Daemon started.\n".into()))
        }
        DaemonAction::Stop => {
            if !client::daemon_running().await {
                return Ok(Output::with(
                    serde_json::json!({"running": false}),
                    "Not running.\n".into(),
                ));
            }
            let v = client::send(&Request::DaemonShutdown).await?;
            Ok(Output::with(v, "Daemon stopping.\n".into()))
        }
        DaemonAction::Status => {
            if !client::daemon_running().await {
                return Ok(Output::plain(serde_json::json!({"running": false})));
            }
            let v = client::send(&Request::DaemonStatus).await?;
            Ok(Output::with(
                v.clone(),
                format!("Running since {}\n", v["started_at"]),
            ))
        }
    }
}

/// `cairn update` — report, and install when asked.
async fn update_command(check_only: bool) -> Result<Output, WireError> {
    let outcome = if check_only {
        update::check().await?
    } else {
        update::apply().await?
    };

    let mut text = format!("Installed  {}\n", outcome.current);
    match &outcome.latest {
        Some(latest) => {
            text.push_str(&format!("Latest     {}\n", latest.version));
            if outcome.installed {
                text.push_str("\nUpdated. Restart the daemon so it runs the new build:\n");
                text.push_str("  cairn daemon stop && cairn daemon start\n");
            } else if outcome.update_available {
                text.push_str(&format!("\n{} is available.\n", latest.version));
                text.push_str("Run `cairn update` to install it, or read about it first:\n");
                text.push_str(&format!("  {}\n", latest.url));
            } else {
                text.push_str("\nAlready up to date.\n");
            }
        }
        None => text.push_str("Latest     could not be determined\n"),
    }

    let value = serde_json::json!({
        "current": outcome.current,
        "latest": outcome.latest,
        "update_available": outcome.update_available,
        "installed": outcome.installed,
        "installed_to": outcome
            .installed_to
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
    });
    Ok(Output::with(value, text))
}

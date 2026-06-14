//! The `cairn` binary.
//!
//! `cairn serve` starts the server + embedded web UI. The other subcommands operate directly on
//! the local store so you can poke at the engine without a running server (handy for tests).

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use cairn_api::AppState;
use cairn_core::{Config, NewMemory};
use clap::{Parser, Subcommand};

mod hook;
mod install;
mod sync;

#[derive(Parser)]
#[command(
    name = "cairn",
    version,
    about = "The context & reliability layer for AI agents"
)]
struct Cli {
    /// Override the data directory (defaults to the OS data dir; use /data in Docker).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the Cairn server (HTTP API + web control plane).
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 7777)]
        port: u16,
    },
    /// Store a memory.
    Remember { content: String },
    /// Recall memories matching a query.
    Recall {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Print the session-start bootstrap (wakeup) memories.
    Wakeup {
        #[arg(long, default_value_t = 12)]
        limit: usize,
    },
    /// Record a standing preference (e.g. `cairn prefer always use ripgrep`).
    Prefer {
        #[arg(trailing_var_arg = true, num_args = 1..)]
        rule: Vec<String>,
    },
    /// Show basic stats.
    Stats,
    /// Verify the local setup.
    Doctor,
    /// Pair this device with a Cairn server using a code from the web UI. (coming soon)
    Pair { code: String },
    /// Configure an agent (or --all detected agents) to use this server. (coming soon)
    Install {
        /// Agent name, e.g. claude-code, codex, cursor. Omit with --all.
        agent: Option<String>,
        /// Configure every detected agent.
        #[arg(long)]
        all: bool,
    },
    /// Log in to a Cairn server. (coming soon)
    Login {
        /// Server URL.
        server: Option<String>,
    },
    /// Update the cairn binary in place. (coming soon)
    Update,
    /// Run the MCP server over stdio (point your agent's MCP config at `cairn mcp`).
    Mcp,
    /// Run a command and print compressed output; the full output is retained and recoverable.
    Run {
        /// The command to run, e.g. `cairn run -- cargo test`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Internal: handle a Claude Code lifecycle hook event (reads JSON on stdin).
    Hook { event: String },
    /// Manage device tokens for authenticating other devices to this server.
    Token {
        #[command(subcommand)]
        action: TokenCmd,
    },
    /// Sync memory with another Cairn server (last-write-wins).
    Sync {
        /// Server base URL, e.g. http://192.168.1.10:7777
        #[arg(long)]
        server: String,
        /// Device token for the remote server (if it requires auth).
        #[arg(long)]
        token: Option<String>,
    },
    /// Export all memories as JSON (to a file, or stdout if omitted).
    Export { path: Option<PathBuf> },
    /// Import memories from a JSON file (last-write-wins).
    Import { path: PathBuf },
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Create a token for a device (prints the token to stdout).
    Create { name: String },
    /// List device tokens.
    List,
    /// Revoke a device token.
    Revoke { token: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Log to stderr so `cairn mcp` keeps stdout clean for the JSON-RPC protocol.
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
            let addr: SocketAddr = format!("{host}:{port}")
                .parse()
                .with_context(|| format!("invalid address {host}:{port}"))?;
            println!("🪨  Cairn serving on http://{addr}");
            println!("    data dir: {}", cfg.data_dir().display());
            cairn_api::serve(addr, state).await?;
        }
        Cmd::Remember { content } => {
            let state = AppState::new(&cfg)?;
            let m = state.mem.remember(NewMemory::new(content))?;
            println!(
                "remembered {} ({}/{})",
                &m.id[..8],
                m.kind.as_str(),
                m.tier.as_str()
            );
        }
        Cmd::Recall { query, limit } => {
            let state = AppState::new(&cfg)?;
            let hits = state.mem.recall(&query, limit)?;
            if hits.is_empty() {
                println!("(no matches)");
            }
            for h in hits {
                println!("[{:.2}] {}", h.score, h.memory.content);
            }
        }
        Cmd::Wakeup { limit } => {
            let state = AppState::new(&cfg)?;
            for m in state.mem.wakeup(limit)? {
                println!("· ({}) {}", m.kind.as_str(), m.content);
            }
        }
        Cmd::Prefer { rule } => {
            let state = AppState::new(&cfg)?;
            let m = state.profile.prefer(&rule.join(" "))?;
            println!("noted preference: {}", m.content);
        }
        Cmd::Stats => {
            let state = AppState::new(&cfg)?;
            println!("memories: {}", state.store.count_memories()?);
        }
        Cmd::Doctor => {
            let _ = AppState::new(&cfg)?;
            println!("cairn doctor: ok");
            println!("  data dir : {}", cfg.data_dir().display());
            println!("  database : {}", cfg.db_path().display());
            println!("  blobs    : {}", cfg.blobs_dir().display());
        }
        Cmd::Pair { code } => coming_soon(&format!("pairing this device with code {code}")),
        Cmd::Install { agent, all } => install::run(agent.as_deref(), all)?,
        Cmd::Login { server } => coming_soon(&format!(
            "logging in to {}",
            server.as_deref().unwrap_or("<server>")
        )),
        Cmd::Update => coming_soon("self-updating the cairn binary"),
        Cmd::Mcp => {
            // No stdout banner here: stdout is the MCP channel.
            let server = cairn_mcp::McpServer::new(&cfg)?;
            server.serve_stdio()?;
        }
        Cmd::Run { command } => {
            if command.is_empty() {
                anyhow::bail!("usage: cairn run -- <command>");
            }
            let state = AppState::new(&cfg)?;
            let output = std::process::Command::new(&command[0])
                .args(&command[1..])
                .output()
                .with_context(|| format!("running `{}`", command.join(" ")))?;
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            let c = state.shell.compress(&command.join(" "), &combined)?;
            print!("{}", c.output);
            if !c.output.ends_with('\n') {
                println!();
            }
            eprintln!(
                "[cairn: {} → {} lines, {:.0}% saved · recover full output with `expand {}`]",
                c.original_lines,
                c.compressed_lines,
                c.saved_ratio * 100.0,
                c.original_hash
            );
            std::process::exit(output.status.code().unwrap_or(0));
        }
        Cmd::Hook { event } => hook::run(&cfg, &event)?,
        Cmd::Token { action } => {
            let state = AppState::new(&cfg)?;
            match action {
                TokenCmd::Create { name } => {
                    let t = state.store.create_token(&name)?;
                    println!("{}", t.token);
                    eprintln!(
                        "created token for '{}'. /api access now requires a device token.",
                        t.name
                    );
                }
                TokenCmd::List => {
                    for t in state.store.list_tokens()? {
                        println!("{}  {}  {}", t.token, t.name, t.created_at.to_rfc3339());
                    }
                }
                TokenCmd::Revoke { token } => {
                    if state.store.revoke_token(&token)? {
                        println!("revoked");
                    } else {
                        println!("no such token");
                    }
                }
            }
        }
        Cmd::Sync { server, token } => {
            let state = AppState::new(&cfg)?;
            sync::run(&state.store, &server, token.as_deref())?;
        }
        Cmd::Export { path } => {
            let state = AppState::new(&cfg)?;
            let mems = state.store.all_memories()?;
            let json = serde_json::to_string_pretty(&mems)?;
            match path {
                Some(p) => {
                    std::fs::write(&p, json)?;
                    println!("exported {} memories to {}", mems.len(), p.display());
                }
                None => println!("{json}"),
            }
        }
        Cmd::Import { path } => {
            let state = AppState::new(&cfg)?;
            let text = std::fs::read_to_string(&path)?;
            let mems: Vec<cairn_core::Memory> = serde_json::from_str(&text)?;
            let mut applied = 0usize;
            for m in &mems {
                if state.store.upsert_memory(m)? {
                    applied += 1;
                }
            }
            println!("imported {applied} of {} memories", mems.len());
        }
    }
    Ok(())
}

/// Friendly placeholder for commands whose full behavior arrives in a later phase.
fn coming_soon(what: &str) {
    println!("cairn: {what} — coming soon in a later build.");
}

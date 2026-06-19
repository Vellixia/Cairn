//! The `cairn-cli` binary.
//!
//! `cairn-cli` connects AI agents to a Cairn server and runs local tools against the local store.
//! When `CAIRN_HELIX_URL` is set it talks to a local HelixDB; when `CAIRN_SERVER` is set it proxies
//! through the remote Cairn HTTP API.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use cairn_core::{Config, NewMemory};
use clap::{Parser, Subcommand};

mod bench;
mod hook;
mod pair;
mod pool;
mod rules;
mod setup;
mod sync;
mod update;

#[derive(Parser)]
#[command(
    name = "cairn-cli",
    version,
    about = "Cairn client — connect AI agents to a Cairn server"
)]
struct Cli {
    /// Override the data directory (defaults to the OS data dir).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
    /// Record a standing preference (e.g. `cairn-cli prefer always use ripgrep`).
    Prefer {
        #[arg(trailing_var_arg = true, num_args = 1..)]
        rule: Vec<String>,
    },
    /// Set the current task anchor — the goal re-injected at session start.
    Anchor {
        #[arg(trailing_var_arg = true, num_args = 1..)]
        goal: Vec<String>,
    },
    /// Snapshot tracked files so you can roll back later.
    Checkpoint {
        #[arg(trailing_var_arg = true, num_args = 0..)]
        label: Vec<String>,
    },
    /// Roll back tracked files to a checkpoint (undo damage).
    Rollback { id: String },
    /// List checkpoints (newest first).
    Checkpoints,
    /// Show basic stats.
    Stats,
    /// Verify the local setup.
    Doctor,
    /// Measure the token savings Cairn gives on a codebase.
    Bench { path: Option<PathBuf> },
    /// Pair this device with a Cairn server using a code from the host.
    Pair {
        code: String,
        /// Server base URL, e.g. http://192.168.1.10:7777
        #[arg(long)]
        server: String,
    },
    /// Configure an agent (or --all detected agents) to use a Cairn server.
    Setup {
        /// Agent name: claude-code, cursor, vscode, windsurf, opencode. Omit (with --all) to auto-detect.
        agent: Option<String>,
        /// Configure every detected agent.
        #[arg(long)]
        all: bool,
        /// Remote Cairn server URL (defaults to `CAIRN_SERVER`).
        #[arg(long)]
        server: Option<String>,
        /// Device token for the remote server (defaults to `CAIRN_TOKEN`).
        #[arg(long)]
        token: Option<String>,
    },
    /// Write per-agent instruction files that tell the model to use Cairn's tools.
    Rules {
        /// Agent: claude-code, cursor, vscode, windsurf, opencode, agents. Omit with --all.
        agent: Option<String>,
        /// Write rules for every supported agent.
        #[arg(long)]
        all: bool,
    },
    /// Log in to a Cairn server. (coming soon)
    Login {
        /// Server URL.
        server: Option<String>,
    },
    /// Update the cairn-cli binary in place to the latest GitHub release.
    Update,
    /// Run the MCP server over stdio (point your agent's MCP config at `cairn-cli mcp`).
    Mcp,
    /// Run a command and print compressed output; the full output is retained and recoverable.
    Run {
        /// The command to run, e.g. `cairn-cli run -- cargo test`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Internal: handle a Claude Code lifecycle hook event (reads JSON on stdin).
    Hook { event: String },
    /// Sync memory with another Cairn server (last-write-wins).
    Sync {
        /// Server base URL, e.g. http://192.168.1.10:7777
        #[arg(long)]
        server: String,
        /// Device token for the remote server (if it requires auth).
        #[arg(long)]
        token: Option<String>,
    },
    /// Contribute your shareable knowledge to a server's collective pool (sanitized first).
    Contribute {
        /// Pool server base URL, e.g. http://pool.example.com:7777
        #[arg(long)]
        server: String,
        /// Device token for the remote server (if it requires auth).
        #[arg(long)]
        token: Option<String>,
    },
    /// Pull a server's collective pool into your local memory.
    Pull {
        /// Pool server base URL.
        #[arg(long)]
        server: String,
        /// Device token for the remote server (if it requires auth).
        #[arg(long)]
        token: Option<String>,
    },
    /// Export all memories as JSON (to a file, or stdout if omitted).
    Export {
        path: Option<PathBuf>,
        /// Sanitize for sharing: redact secrets/PII and drop memories that contain hard secrets.
        #[arg(long)]
        share: bool,
    },
    /// Import memories from a JSON file (last-write-wins), or a share bundle with `--share`.
    Import {
        path: PathBuf,
        /// Treat the file as a sanitized share bundle and ingest it as shared memories.
        #[arg(long)]
        share: bool,
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
        Cmd::Remember { content } => {
            let s = State::open(&cfg)?;
            let m = s.mem.remember(NewMemory::new(content))?;
            println!(
                "remembered {} ({}/{})",
                &m.id[..8],
                m.kind.as_str(),
                m.tier.as_str()
            );
        }
        Cmd::Recall { query, limit } => {
            let s = State::open(&cfg)?;
            let hits = s.mem.recall(&query, limit)?;
            if hits.is_empty() {
                println!("(no matches)");
            }
            for h in hits {
                println!("[{:.2}] {}", h.score, h.memory.content);
            }
        }
        Cmd::Wakeup { limit } => {
            let s = State::open(&cfg)?;
            for m in s.mem.wakeup(limit)? {
                println!("· ({}) {}", m.kind.as_str(), m.content);
            }
        }
        Cmd::Prefer { rule } => {
            let s = State::open(&cfg)?;
            let m = s.profile.prefer(&rule.join(" "))?;
            if m.suspicious {
                println!("noted preference (flagged suspicious): {}", m.content);
            } else {
                println!("noted preference: {}", m.content);
            }
        }
        Cmd::Anchor { goal } => {
            let s = State::open(&cfg)?;
            let goal = goal.join(" ");
            s.guard.set_anchor(&goal)?;
            println!("task anchor set: {goal}");
        }
        Cmd::Checkpoint { label } => {
            let s = State::open(&cfg)?;
            let label = if label.is_empty() {
                "checkpoint".to_string()
            } else {
                label.join(" ")
            };
            let cp = s.guard.checkpoint(&label)?;
            println!("checkpoint {} created ({} files tracked)", cp.id, cp.files);
        }
        Cmd::Rollback { id } => {
            let s = State::open(&cfg)?;
            let r = s.guard.rollback(&id)?;
            println!(
                "rolled back {}: {} restored, {} skipped",
                id,
                r.restored.len(),
                r.skipped.len()
            );
        }
        Cmd::Checkpoints => {
            let s = State::open(&cfg)?;
            for cp in s.guard.list_checkpoints()? {
                println!(
                    "{}  {}  ({} files)  {}",
                    cp.id,
                    cp.created_at.to_rfc3339(),
                    cp.files,
                    cp.label
                );
            }
        }
        Cmd::Stats => {
            let s = State::open(&cfg)?;
            println!("memories: {}", s.store.count_memories()?);
        }
        Cmd::Doctor => {
            println!("cairn-cli doctor");
            println!("  data dir     : {}", cfg.data_dir().display());
            println!("  blobs        : {}", cfg.blobs_dir().display());
            println!(
                "  helix url    : {}",
                cfg.helix_url
                    .as_deref()
                    .unwrap_or("(not set — CAIRN_HELIX_URL required)")
            );
            println!(
                "  server       : {}",
                cfg.default_server
                    .as_deref()
                    .unwrap_or("(not set — CAIRN_SERVER optional)")
            );
            println!("  embed        : {}", cfg.embed.provider);
            match State::open(&cfg) {
                Ok(s) => {
                    let n = s.store.count_memories()?;
                    println!("  helix        : ok");
                    println!("  memories     : {n}");
                    println!("cairn-cli doctor: ok");
                }
                Err(e) => {
                    println!("  helix        : FAILED — {e}");
                    anyhow::bail!("cairn-cli doctor: setup incomplete");
                }
            }
        }
        Cmd::Bench { path } => {
            let s = State::open(&cfg)?;
            let root = path
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            bench::run(&s, &root)?;
        }
        Cmd::Pair { code, server } => {
            let s = State::open(&cfg)?;
            pair::claim(&s, &server, &code)?;
        }
        Cmd::Setup {
            agent,
            all,
            server,
            token,
        } => setup::run(agent.as_deref(), all, server.as_deref(), token.as_deref())?,
        Cmd::Rules { agent, all } => rules::run(agent.as_deref(), all)?,
        Cmd::Login { server } => coming_soon(&format!(
            "logging in to {}",
            server.as_deref().unwrap_or("<server>")
        )),
        Cmd::Update => update::run()?,
        Cmd::Mcp => {
            cairn_mcp::serve_stdio(&cfg)?;
        }
        Cmd::Run { command } => {
            if command.is_empty() {
                anyhow::bail!("usage: cairn-cli run -- <command>");
            }
            let s = State::open(&cfg)?;
            let output = std::process::Command::new(&command[0])
                .args(&command[1..])
                .output()
                .with_context(|| format!("running `{}`", command.join(" ")))?;
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&output.stderr));
            let c = s.shell.compress(&command.join(" "), &combined)?;
            print!("{}", c.output);
            if !c.output.ends_with('\n') {
                println!();
            }
            eprintln!(
                "[cairn-cli: {} → {} lines, {:.0}% saved · recover full output with `expand {}`]",
                c.original_lines,
                c.compressed_lines,
                c.saved_ratio * 100.0,
                c.original_hash
            );
            std::process::exit(output.status.code().unwrap_or(0));
        }
        Cmd::Hook { event } => hook::run(&cfg, &event)?,
        Cmd::Sync { server, token } => {
            let s = State::open(&cfg)?;
            sync::run(&s.store, &server, token.as_deref())?;
        }
        Cmd::Contribute { server, token } => {
            let s = State::open(&cfg)?;
            pool::contribute(&s.store, &server, token.as_deref())?;
        }
        Cmd::Pull { server, token } => {
            let s = State::open(&cfg)?;
            pool::pull(&s.mem, &server, token.as_deref())?;
        }
        Cmd::Export { path, share } => {
            let s = State::open(&cfg)?;
            let mems = s.store.all_memories()?;
            let json = if share {
                build_share_bundle(&mems)?
            } else {
                serde_json::to_string_pretty(&mems)?
            };
            match path {
                Some(p) => {
                    std::fs::write(&p, json)?;
                    let what = if share {
                        "shareable memories"
                    } else {
                        "memories"
                    };
                    println!("exported {what} to {}", p.display());
                }
                None => println!("{json}"),
            }
        }
        Cmd::Import { path, share } => {
            let s = State::open(&cfg)?;
            let text = std::fs::read_to_string(&path)?;
            if share {
                let bundle: cairn_share::ShareBundle = serde_json::from_str(&text)?;
                let news = bundle.into_new_memories();
                let total = news.len();
                for nm in news {
                    s.mem.remember(nm)?;
                }
                println!("ingested {total} shared memories (deduplicated against existing)");
            } else {
                let mems: Vec<cairn_core::Memory> = serde_json::from_str(&text)?;
                let mut applied = 0usize;
                for m in &mems {
                    if s.store.upsert_memory(m)? {
                        applied += 1;
                    }
                }
                println!("imported {applied} of {} memories", mems.len());
            }
        }
    }
    Ok(())
}

fn build_share_bundle(mems: &[cairn_core::Memory]) -> anyhow::Result<String> {
    let san = cairn_share::Sanitizer::new();
    let (bundle, stats) = san.bundle(mems);
    eprintln!(
        "[cairn share: {} scanned → {} shareable ({} need review), {} withheld as private]",
        stats.total, stats.shared, stats.needs_review, stats.withheld
    );
    Ok(serde_json::to_string_pretty(&bundle)?)
}

fn coming_soon(what: &str) {
    println!("cairn-cli: {what} — coming soon in a later build.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::NewMemory;

    #[test]
    fn share_bundle_redacts_pii_and_withholds_hard_secrets() {
        let clean = NewMemory::new("use BM25 for recall ranking").into_memory();
        let pii =
            NewMemory::new("reach the team at ops@example.com about the rollout").into_memory();
        let leak = format!(
            "api_key = sk-ant-{}",
            "api03-abcdefghijklmnopqrstuvwxyz0123"
        );
        let secret = NewMemory::new(&leak).into_memory();

        let json = build_share_bundle(&[clean, pii, secret]).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mems = v["memories"].as_array().unwrap();

        assert_eq!(mems.len(), 2);
        assert!(json.contains("[redacted:email]"));
        assert!(!json.contains("ops@example.com"));
        assert!(!json.contains("abcdefghijklmnopqrstuvwxyz0123"));
        assert_eq!(v["schema"], "cairn-share-bundle");
    }
}

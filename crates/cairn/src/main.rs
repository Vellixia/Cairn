//! `cairn` setup plus hidden hook and MCP adapters.
#![recursion_limit = "512"]

mod client;
mod hook;
mod mcp;
mod render;

use cairn_core::wire::{codes, Request, WireError};
#[cfg(test)]
use clap::CommandFactory;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cairn",
    version,
    about = "Persistent, project-aware memory for AI coding agents",
    disable_help_subcommand = true
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
    /// Set up this repository for Cairn.
    Setup,
    /// Run MCP adapter over stdio.
    #[command(hide = true)]
    Mcp,
    /// Run agent hook adapter. Always exits 0.
    #[command(hide = true)]
    Hook {
        event: String,
        #[arg(long)]
        agent: Option<String>,
    },
}

const EXIT_USER_ERROR: i32 = 1;
const EXIT_UNAVAILABLE: i32 = 2;

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() >= 3 && argv[1] == "hook" && hook::run_blocking(&argv[2]) {
        std::process::exit(0);
    }
    run_async()
}

#[tokio::main]
async fn run_async() {
    let cli = Cli::parse();
    let json = cli.json;
    match cli.command {
        Command::Hook { event, agent } => {
            let _ = agent;
            hook::run(&event).await;
        }
        Command::Mcp => {
            if let Err(error) = mcp::serve().await {
                eprintln!("cairn mcp: {error}");
                std::process::exit(EXIT_UNAVAILABLE);
            }
        }
        Command::Setup => match setup().await {
            Ok(value) => print!("{}", render_setup_success(&value, json)),
            Err(error) => {
                if json {
                    println!("{}", render_setup_error(&error, true));
                } else {
                    eprint!("{}", render_setup_error(&error, false));
                }
                std::process::exit(exit_code(&error));
            }
        },
    }
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".into())
}

async fn setup() -> Result<serde_json::Value, WireError> {
    let value = client::send(&Request::Init { cwd: cwd() }).await?;
    Ok(value)
}

fn render_setup_success(value: &serde_json::Value, json: bool) -> String {
    if json {
        serde_json::to_string_pretty(&cairn_core::wire::Envelope::ok(value.clone()))
            .expect("setup envelope serializes")
            + "\n"
    } else {
        let name = value["project"]["name"].as_str().unwrap_or("project");
        format!("Cairn is tracking {name}.\n")
    }
}

fn render_setup_error(error: &WireError, json: bool) -> String {
    if json {
        serde_json::to_string_pretty(&cairn_core::wire::Envelope::err(error.clone()))
            .expect("setup error envelope serializes")
            + "\n"
    } else {
        format!("cairn: {}: {}\n", error.code, error.message)
    }
}

fn exit_code(error: &WireError) -> i32 {
    match error.code.as_str() {
        codes::DAEMON_UNAVAILABLE | codes::STORAGE_UNAVAILABLE | codes::SERVER_UNAVAILABLE => {
            EXIT_UNAVAILABLE
        }
        _ => EXIT_USER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_surface_exposes_only_setup() {
        assert!(Cli::try_parse_from(["cairn", "setup"]).is_ok());

        let help = Cli::command().render_help().to_string();
        assert_eq!(listed_commands(&help), vec!["setup"]);
        assert!(Cli::try_parse_from(["cairn", "hook", "session-start"]).is_ok());
        assert!(Cli::try_parse_from(["cairn", "mcp"]).is_ok());
        assert!(Cli::try_parse_from(["cairn", "init"]).is_err());
    }

    #[test]
    fn setup_renders_stable_text_and_json_envelopes() {
        let value = serde_json::json!({ "project": { "name": "demo" } });
        assert_eq!(render_setup_success(&value, false), "Cairn is tracking demo.\n");

        let json: serde_json::Value = serde_json::from_str(&render_setup_success(&value, true))
            .expect("setup JSON");
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"], value);

        let error = WireError::invalid("bad setup");
        let json: serde_json::Value = serde_json::from_str(&render_setup_error(&error, true))
            .expect("setup error JSON");
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], codes::INVALID_REQUEST);
        assert_eq!(render_setup_error(&error, false), "cairn: invalid_request: bad setup\n");
    }

    #[test]
    fn default_help_matches_golden() {
        assert_eq!(
            normalize_help(&Cli::command().render_help().to_string()),
            normalize_help(include_str!("../tests/snapshots/default-help.txt")),
        );
    }

    fn normalize_help(help: &str) -> String {
        help.replace("\r\n", "\n")
    }

    fn listed_commands(help: &str) -> Vec<&str> {
        help.split_once("Commands:\n")
            .and_then(|(_, rest)| rest.split_once("\n\nOptions:"))
            .map(|(commands, _)| {
                commands
                    .lines()
                    .filter_map(|line| line.strip_prefix("  "))
                    .filter_map(|line| line.split_whitespace().next())
                    .collect()
            })
            .unwrap_or_default()
    }
}

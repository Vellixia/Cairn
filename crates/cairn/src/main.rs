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
use std::io::IsTerminal;
use std::path::Path;
use uuid::Uuid;

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
    let credentials = headless_credentials()?;
    let web_url = credentials
        .as_ref()
        .map(|credentials| {
            credentials
                .web_url
                .clone()
                .unwrap_or_else(|| credentials.url.clone())
        })
        .or_else(|| cairn_core::CairnConfig::load().server_url);
    if let Some(credentials) = credentials {
        let account_id = authenticated_account(&credentials).await?;
        if let Some(expected) = credentials.account_id {
            if expected != account_id {
                return Err(WireError::new(
                    codes::UNAUTHORIZED,
                    "credential account mismatch",
                ));
            }
        }
        persist_credentials(&credentials, account_id).map_err(|error| {
            WireError::new(
                codes::STORAGE_UNAVAILABLE,
                format!("could not save credentials: {error}"),
            )
        })?;
    }
    let mut value = client::send(&Request::Init { cwd: cwd() }).await?;
    if let (Some(object), Some(web_url)) = (value.as_object_mut(), web_url) {
        object.insert("web_url".into(), serde_json::Value::String(web_url));
    }
    Ok(value)
}

struct HeadlessCredentials {
    url: String,
    token: String,
    account_id: Option<Uuid>,
    web_url: Option<String>,
}

fn headless_credentials() -> Result<Option<HeadlessCredentials>, WireError> {
    let url = std::env::var("CAIRN_SERVER_URL").ok();
    let token = std::env::var("CAIRN_SERVER_TOKEN").ok();
    let account_id = std::env::var("CAIRN_ACCOUNT_ID").ok();
    let web_url = std::env::var("CAIRN_WEB_URL").ok();
    if url.is_some() || token.is_some() || account_id.is_some() || web_url.is_some() {
        return parse_credentials(url, token, account_id, web_url);
    }
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
        .map_err(|error| WireError::invalid(format!("could not read setup input: {error}")))?;
    if input.trim().is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(&input)
        .map_err(|_| WireError::invalid("setup input must be JSON credentials"))?;
    parse_credentials(
        value
            .get("server_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        value
            .get("server_token")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        value
            .get("account_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        value
            .get("web_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    )
}

fn parse_credentials(
    url: Option<String>,
    token: Option<String>,
    account_id: Option<String>,
    web_url: Option<String>,
) -> Result<Option<HeadlessCredentials>, WireError> {
    let (Some(url), Some(token)) = (url, token) else {
        return Err(WireError::invalid("setup needs both server URL and token"));
    };
    if url.trim().is_empty() || token.trim().is_empty() {
        return Err(WireError::invalid("setup needs both server URL and token"));
    }
    let account_id = account_id
        .filter(|id| !id.is_empty())
        .map(|id| Uuid::parse_str(&id).map_err(|_| WireError::invalid("invalid account id")))
        .transpose()?;
    Ok(Some(HeadlessCredentials {
        url,
        token,
        account_id,
        web_url: web_url.filter(|url| !url.trim().is_empty()),
    }))
}

async fn authenticated_account(credentials: &HeadlessCredentials) -> Result<Uuid, WireError> {
    let base = credentials.url.trim_end_matches('/');
    let response = reqwest::Client::new()
        .get(format!("{base}/api/auth/me"))
        .bearer_auth(&credentials.token)
        .send()
        .await
        .map_err(|_| {
            WireError::new(
                codes::SERVER_UNAVAILABLE,
                "could not verify server credential",
            )
        })?;
    if !response.status().is_success() {
        return Err(WireError::new(
            codes::UNAUTHORIZED,
            "server rejected credential",
        ));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| WireError::new(codes::SERVER_UNAVAILABLE, "invalid credential response"))?;
    body.get("id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(|| WireError::new(codes::SERVER_UNAVAILABLE, "invalid credential response"))
}

fn persist_credentials(credentials: &HeadlessCredentials, account_id: Uuid) -> std::io::Result<()> {
    cairn_core::paths::ensure_home()?;
    persist_credentials_at(
        credentials,
        account_id,
        &cairn_core::paths::config_path(),
        &cairn_core::paths::token_path(),
    )
}

fn persist_credentials_at(
    credentials: &HeadlessCredentials,
    account_id: Uuid,
    config_path: &Path,
    token_path: &Path,
) -> std::io::Result<()> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = token_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut config = cairn_core::CairnConfig::load_from(config_path).unwrap_or_default();
    config.server_url = Some(credentials.url.trim_end_matches('/').to_owned());
    config.server_account_id = Some(account_id);
    config.save_to(config_path)?;
    write_token(token_path, &credentials.token)
}

fn write_token(path: &Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn render_setup_success(value: &serde_json::Value, json: bool) -> String {
    if json {
        serde_json::to_string_pretty(&cairn_core::wire::Envelope::ok(value.clone()))
            .expect("setup envelope serializes")
            + "\n"
    } else {
        let name = value["project"]["name"].as_str().unwrap_or("project");
        let mut output = format!("Cairn is tracking {name}.\n");
        if let Some(web_url) = value.get("web_url").and_then(serde_json::Value::as_str) {
            output.push_str(&format!("Web: {web_url}\n"));
        }
        if let Some(warnings) = value
            .pointer("/integrations/warnings")
            .and_then(serde_json::Value::as_array)
        {
            for warning in warnings {
                let agent = warning["agent"].as_str().unwrap_or("agent");
                let kind = warning["kind"].as_str().unwrap_or("integration");
                let detail = warning["detail"].as_str().unwrap_or("conflict");
                output.push_str(&format!("Integration warning ({agent}/{kind}): {detail}\n"));
            }
        }
        if let Some(migration) = value.get("legacy_migration") {
            let status = migration["status"].as_str().unwrap_or("unknown");
            output.push_str(&format!("Legacy Task migration: {status}.\n"));
            if let Some(detail) = migration["detail"].as_str() {
                output.push_str(&format!("Detail: {detail}\n"));
            }
            for key in ["backup", "manifest", "bundle"] {
                if let Some(path) = migration[key].as_str() {
                    output.push_str(&format!("{key}: {path}\n"));
                }
            }
        }
        output
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
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
        let value = serde_json::json!({ "project": { "name": "demo" }, "web_url": "https://cairn.example.com", "integrations": { "warnings": [{ "agent": "codex", "kind": "mcp", "detail": "edited by user" }] }, "legacy_migration": { "status": "warning", "detail": "artifact conflict", "backup": "/tmp/legacy.sqlite", "manifest": "/tmp/legacy.manifest.json", "bundle": "/tmp/removed_feature.json" } });
        assert_eq!(render_setup_success(&value, false), "Cairn is tracking demo.\nWeb: https://cairn.example.com\nIntegration warning (codex/mcp): edited by user\nLegacy Task migration: warning.\nDetail: artifact conflict\nbackup: /tmp/legacy.sqlite\nmanifest: /tmp/legacy.manifest.json\nbundle: /tmp/removed_feature.json\n");

        let json: serde_json::Value =
            serde_json::from_str(&render_setup_success(&value, true)).expect("setup JSON");
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"], value);

        let error = WireError::invalid("bad setup");
        let json: serde_json::Value =
            serde_json::from_str(&render_setup_error(&error, true)).expect("setup error JSON");
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], codes::INVALID_REQUEST);
        assert_eq!(
            render_setup_error(&error, false),
            "cairn: invalid_request: bad setup\n"
        );
    }

    #[test]
    fn default_help_matches_golden() {
        assert_eq!(
            normalize_help(&Cli::command().render_help().to_string()),
            normalize_help(include_str!("../tests/snapshots/default-help.txt")),
        );
    }

    #[test]
    fn headless_credentials_require_complete_pair() {
        assert!(parse_credentials(Some("https://server".into()), None, None, None).is_err());
        assert!(parse_credentials(None, Some("secret".into()), None, None).is_err());
    }

    #[test]
    fn credential_persistence_is_idempotent_and_keeps_token_private() {
        let dir = tempfile::tempdir().unwrap();
        let credentials = HeadlessCredentials {
            url: "https://server/".into(),
            token: "secret".into(),
            account_id: None,
            web_url: None,
        };
        let account = Uuid::now_v7();
        let config = dir.path().join("config.json");
        let token = dir.path().join("token");
        persist_credentials_at(&credentials, account, &config, &token).unwrap();
        persist_credentials_at(&credentials, account, &config, &token).unwrap();
        let saved = cairn_core::CairnConfig::load_from(&config).unwrap();
        assert_eq!(saved.server_url.as_deref(), Some("https://server"));
        assert_eq!(saved.server_account_id, Some(account));
        assert_eq!(std::fs::read_to_string(&token).unwrap(), "secret");
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&token).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn headless_credential_uses_authenticated_account() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let account = Uuid::now_v7();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let read = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /api/auth/me"));
            assert!(request.contains("authorization: Bearer secret"));
            let body = format!(r#"{{"id":"{account}"}}"#);
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes(),
                )
                .await
                .unwrap();
        });
        let credentials = HeadlessCredentials {
            url: format!("http://{address}"),
            token: "secret".into(),
            account_id: None,
            web_url: None,
        };
        assert_eq!(authenticated_account(&credentials).await.unwrap(), account);
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

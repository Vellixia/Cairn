//! `cairn-cli setup [agent|--all]` — wire AI agents up to a Cairn server.
//!
//! Every merge is **non-destructive**: existing config is preserved and our entries are added
//! idempotently (running twice changes nothing). Each agent is configured in its own native
//! format:
//!
//! - **Claude Code** — project `.mcp.json` (the `cairn-cli` MCP server) **and** `.claude/settings.json`
//!   lifecycle hooks.
//! - **Cursor** — project `.cursor/mcp.json` (`mcpServers` schema).
//! - **VS Code** — project `.vscode/mcp.json` (`servers` schema).
//! - **Windsurf** — `~/.codeium/windsurf/mcp_config.json` (`mcpServers` schema).
//! - **OpenCode** — `%APPDATA%\OpenCode\opencode.json` on Windows, `~/.config/opencode/opencode.json`
//!   on Unix (`mcp` top-level key with `{ type, command, environment, enabled }` entries).
//!
//! When `--server` is passed, the MCP server entry includes `CAIRN_SERVER` and `CAIRN_TOKEN` env
//! vars so `cairn-cli mcp` runs in remote-proxy mode; otherwise it runs in local HelixDB mode.
//!
//! `--all` configures only the agents it actually detects (project markers or home-dir install);
//! naming an agent explicitly configures it regardless.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

const KNOWN: &[&str] = &["claude-code", "cursor", "vscode", "windsurf", "opencode"];

pub fn run(agent: Option<&str>, all: bool, server: Option<&str>, token: Option<&str>) -> Result<()> {
    let project = std::env::current_dir()?;
    let home = home_dir();

    if all {
        let mut configured = 0usize;
        for id in KNOWN {
            if detect(id, &project, home.as_deref()) {
                install_agent(id, &project, home.as_deref(), server, token)?;
                configured += 1;
            }
        }
        if configured == 0 {
            println!("cairn-cli: no supported agents detected here or in your home directory.");
            println!("Install one explicitly, e.g. `cairn-cli setup claude-code`.");
            println!("Supported: {}.", KNOWN.join(", "));
        } else {
            println!("\nStart the server with `cairn serve`, then open a session in your agent.");
        }
        return Ok(());
    }

    let requested = agent.unwrap_or("claude-code");
    let id = canonical_id(requested).ok_or_else(|| {
        anyhow!(
            "unknown agent '{requested}'. Supported: {}.",
            KNOWN.join(", ")
        )
    })?;
    install_agent(id, &project, home.as_deref(), server, token)?;
    println!("\nStart the server with `cairn serve`, then open a session in your agent.");
    Ok(())
}

fn canonical_id(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "claude-code" | "claude" | "claudecode" | "cc" => Some("claude-code"),
        "cursor" => Some("cursor"),
        "vscode" | "code" | "vs-code" => Some("vscode"),
        "windsurf" | "codeium" => Some("windsurf"),
        "opencode" | "oc" => Some("opencode"),
        _ => None,
    }
}

fn detect(id: &str, project: &Path, home: Option<&Path>) -> bool {
    let home_has = |rel: &str| home.is_some_and(|h| h.join(rel).exists());
    match id {
        "claude-code" => {
            project.join(".claude").exists()
                || project.join(".mcp.json").exists()
                || home_has(".claude")
                || home_has(".claude.json")
        }
        "cursor" => project.join(".cursor").exists() || home_has(".cursor"),
        "vscode" => project.join(".vscode").exists(),
        "windsurf" => home_has(".codeium/windsurf"),
        "opencode" => opencode_config_path().exists() || project.join(".opencode").exists(),
        _ => false,
    }
}

fn install_agent(
    id: &str,
    project: &Path,
    home: Option<&Path>,
    server: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    match id {
        "claude-code" => install_claude_code(project, server, token)?,
        "cursor" => install_mcp_only(
            "Cursor",
            &project.join(".cursor").join("mcp.json"),
            "mcpServers",
            server,
            token,
        )?,
        "vscode" => install_mcp_only(
            "VS Code",
            &project.join(".vscode").join("mcp.json"),
            "servers",
            server,
            token,
        )?,
        "windsurf" => {
            let home = home.context("cannot locate your home directory for the Windsurf config")?;
            install_mcp_only(
                "Windsurf",
                &home
                    .join(".codeium")
                    .join("windsurf")
                    .join("mcp_config.json"),
                "mcpServers",
                server,
                token,
            )?
        }
        "opencode" => install_opencode(server, token)?,
        other => anyhow::bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    }
    crate::rules::write_for(id, project)?;
    Ok(())
}

/// OpenCode's global config path. OpenCode follows XDG-ish directories on all platforms:
/// `~/.config/opencode/opencode.json` on Windows and Unix alike.
fn opencode_config_path() -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| {
            home_dir().unwrap_or_else(|| PathBuf::from("."))
        });
    config_home.join(".config").join("opencode").join("opencode.json")
}

fn install_opencode(server: Option<&str>, token: Option<&str>) -> Result<()> {
    let path = opencode_config_path();
    let mut cfg = read_object(&path)?;
    let mcp = cfg.entry("mcp").or_insert_with(|| json!({}));
    let mcp_obj = mcp
        .as_object_mut()
        .with_context(|| format!("{}: 'mcp' is not an object", path.display()))?;

    let mut env = Map::new();
    if let Some(s) = server {
        env.insert("CAIRN_SERVER".into(), json!(s));
    }
    if let Some(t) = token {
        env.insert("CAIRN_TOKEN".into(), json!(t));
    }

    // Use an absolute path to the current cairn-cli binary so the OpenCode
    // launcher can find it regardless of PATH.
    let cli_exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "cairn-cli".into());

    let entry = if env.is_empty() {
        json!({
            "type": "local",
            "command": [cli_exe, "mcp"],
            "enabled": true
        })
    } else {
        json!({
            "type": "local",
            "command": [cli_exe, "mcp"],
            "environment": Value::Object(env),
            "enabled": true
        })
    };
    mcp_obj.insert("cairn".into(), entry);

    write_json(&path, &Value::Object(cfg))?;
    println!("\u{2713} Configured OpenCode:");
    println!("  - {}  (MCP server: {})", path.display(), cli_exe);
    Ok(())
}

fn cairn_server(server: Option<&str>, token: Option<&str>) -> Value {
    let mut env = Map::new();
    if let Some(s) = server {
        env.insert("CAIRN_SERVER".into(), json!(s));
    }
    if let Some(t) = token {
        env.insert("CAIRN_TOKEN".into(), json!(t));
    }
    if env.is_empty() {
        json!({ "command": "cairn-cli", "args": ["mcp"] })
    } else {
        json!({ "command": "cairn-cli", "args": ["mcp"], "env": Value::Object(env) })
    }
}

fn install_mcp_only(
    label: &str,
    path: &Path,
    schema_key: &str,
    server: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    merge_mcp_server(path, schema_key, server, token)?;
    println!("\u{2713} Configured {label}:");
    println!("  - {}  (MCP server: cairn-cli)", path.display());
    Ok(())
}

fn merge_mcp_server(path: &Path, schema_key: &str, server: Option<&str>, token: Option<&str>) -> Result<()> {
    let mut obj = read_object(path)?;
    let servers = obj
        .entry(schema_key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{}: '{schema_key}' is not an object", path.display()))?;
    servers.insert("cairn".into(), cairn_server(server, token));
    write_json(path, &Value::Object(obj))
}

fn install_claude_code(dir: &Path, server: Option<&str>, token: Option<&str>) -> Result<()> {
    let mcp_path = dir.join(".mcp.json");
    merge_mcp_server(&mcp_path, "mcpServers", server, token)?;

    let settings_path = dir.join(".claude").join("settings.json");
    let mut settings = read_object(&settings_path)?;
    {
        let hooks = settings
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("settings.json: hooks is not an object")?;
        add_hook(hooks, "SessionStart", "cairn-cli hook SessionStart", None);
        add_hook(
            hooks,
            "UserPromptSubmit",
            "cairn-cli hook UserPromptSubmit",
            None,
        );
        add_hook(
            hooks,
            "PostToolUse",
            "cairn-cli hook PostToolUse",
            Some("Edit|Write|MultiEdit|NotebookEdit"),
        );
        add_hook(hooks, "SessionEnd", "cairn-cli hook SessionEnd", None);
    }
    write_json(&settings_path, &Value::Object(settings))?;

    println!("\u{2713} Configured Claude Code:");
    println!("  - {}  (MCP server: cairn-cli)", mcp_path.display());
    println!(
        "  - {}  (hooks: SessionStart, UserPromptSubmit, PostToolUse, SessionEnd)",
        settings_path.display()
    );
    Ok(())
}

fn add_hook(hooks: &mut Map<String, Value>, event: &str, command: &str, matcher: Option<&str>) {
    let groups = hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut();
    let Some(groups) = groups else { return };

    let already = groups.iter().any(|g| {
        g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
        })
    });
    if !already {
        let mut group = json!({ "hooks": [{ "type": "command", "command": command }] });
        if let Some(m) = matcher {
            group["matcher"] = json!(m);
        }
        groups.push(group);
    }
}

fn read_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(value.as_object().cloned().unwrap_or_default())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn claude_code_setup_is_idempotent_and_non_destructive() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"model":"opus","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
        )
        .unwrap();

        install_claude_code(dir.path(), None, None).unwrap();
        install_claude_code(dir.path(), None, None).unwrap();

        let settings = read_json(&dir.path().join(".claude/settings.json"));
        assert_eq!(settings["model"], "opus");
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        let cairn_count = starts
            .iter()
            .filter(|g| g["hooks"][0]["command"] == "cairn-cli hook SessionStart")
            .count();
        assert_eq!(cairn_count, 1);
        assert!(starts.iter().any(|g| g["hooks"][0]["command"] == "echo hi"));
        assert!(settings["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["hooks"][0]["command"] == "cairn-cli hook PostToolUse"));

        let mcp = read_json(&dir.path().join(".mcp.json"));
        assert_eq!(mcp["mcpServers"]["cairn"]["command"], "cairn-cli");
    }

    #[test]
    fn cursor_and_vscode_use_their_own_paths_and_schemas() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();

        install_agent("cursor", p, None, None, None).unwrap();
        let cursor = read_json(&p.join(".cursor/mcp.json"));
        assert_eq!(cursor["mcpServers"]["cairn"]["command"], "cairn-cli");
        assert_eq!(cursor["mcpServers"]["cairn"]["args"][0], "mcp");

        install_agent("vscode", p, None, None, None).unwrap();
        let vscode = read_json(&p.join(".vscode/mcp.json"));
        assert_eq!(vscode["servers"]["cairn"]["command"], "cairn-cli");
        assert!(vscode.get("mcpServers").is_none());
    }

    #[test]
    fn windsurf_writes_into_the_home_config_and_preserves_existing() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let cfg = home.path().join(".codeium/windsurf/mcp_config.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        fs::write(&cfg, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();

        install_agent("windsurf", project.path(), Some(home.path()), None, None).unwrap();

        let v = read_json(&cfg);
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["cairn"]["command"], "cairn-cli");
    }

    #[test]
    fn opencode_writes_into_the_home_config_and_includes_remote_env() {
        let cfg = tempfile::tempdir().unwrap().path().join("opencode.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();

        install_opencode_with_path(
            Some("http://example.com:7777"),
            Some("tok-123"),
            &cfg,
        )
        .unwrap();

        let v = read_json(&cfg);
        assert_eq!(v["mcp"]["cairn"]["command"][0], "cairn-cli");
        assert_eq!(v["mcp"]["cairn"]["command"][1], "mcp");
        assert_eq!(v["mcp"]["cairn"]["type"], "local");
        assert_eq!(v["mcp"]["cairn"]["enabled"], true);
        assert_eq!(
            v["mcp"]["cairn"]["environment"]["CAIRN_SERVER"],
            "http://example.com:7777"
        );
        assert_eq!(v["mcp"]["cairn"]["environment"]["CAIRN_TOKEN"], "tok-123");
    }

    #[test]
    fn opencode_local_mode_omits_environment() {
        let cfg = tempfile::tempdir().unwrap().path().join("opencode.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();

        install_opencode_with_path(None, None, &cfg).unwrap();

        let v = read_json(&cfg);
        assert_eq!(v["mcp"]["cairn"]["command"][0], "cairn-cli");
        assert!(v["mcp"]["cairn"]["environment"].is_null());
    }

    fn install_opencode_with_path(
        server: Option<&str>,
        token: Option<&str>,
        path: &Path,
    ) -> Result<()> {
        let mut cfg = read_object(path)?;
        let mcp = cfg.entry("mcp").or_insert_with(|| json!({}));
        let mcp_obj = mcp
            .as_object_mut()
            .with_context(|| format!("{}: 'mcp' is not an object", path.display()))?;
        let mut env = Map::new();
        if let Some(s) = server {
            env.insert("CAIRN_SERVER".into(), json!(s));
        }
        if let Some(t) = token {
            env.insert("CAIRN_TOKEN".into(), json!(t));
        }
        let entry = if env.is_empty() {
            json!({
                "type": "local",
                "command": ["cairn-cli", "mcp"],
                "enabled": true
            })
        } else {
            json!({
                "type": "local",
                "command": ["cairn-cli", "mcp"],
                "environment": Value::Object(env),
                "enabled": true
            })
        };
        mcp_obj.insert("cairn".into(), entry);
        write_json(path, &Value::Object(cfg))
    }

    #[test]
    fn detect_scopes_all_to_present_agents() {
        // Use a temp XDG_CONFIG_HOME so the real OpenCode config does not leak into detection.
        let project = tempfile::tempdir().unwrap();
        let config_home = tempfile::tempdir().unwrap();
        let (p, c) = (project.path(), config_home.path());

        std::env::set_var("XDG_CONFIG_HOME", c);

        assert!(!detect("cursor", p, Some(c)));
        assert!(!detect("vscode", p, Some(c)));
        assert!(!detect("windsurf", p, Some(c)));
        assert!(!detect("opencode", p, Some(c)));

        fs::create_dir_all(p.join(".cursor")).unwrap();
        fs::create_dir_all(p.join(".vscode")).unwrap();
        fs::create_dir_all(c.join(".codeium/windsurf")).unwrap();

        fs::create_dir_all(c.join(".config/opencode")).unwrap();
        fs::write(c.join(".config/opencode/opencode.json"), "{}").unwrap();

        assert!(detect("cursor", p, Some(c)));
        assert!(detect("vscode", p, Some(c)));
        assert!(detect("windsurf", p, Some(c)));
        assert!(detect("opencode", p, Some(c)));
    }

    #[test]
    fn canonical_id_resolves_aliases_and_rejects_unknown() {
        assert_eq!(canonical_id("claude"), Some("claude-code"));
        assert_eq!(canonical_id("Cursor"), Some("cursor"));
        assert_eq!(canonical_id("code"), Some("vscode"));
        assert_eq!(canonical_id("codeium"), Some("windsurf"));
        assert_eq!(canonical_id("opencode"), Some("opencode"));
        assert_eq!(canonical_id("oc"), Some("opencode"));
        assert!(canonical_id("emacs").is_none());
    }
}

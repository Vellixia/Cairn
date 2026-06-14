//! `cairn install [agent|--all]` — wire AI agents up to this Cairn install.
//!
//! Every merge is **non-destructive**: existing config is preserved and our entries are added
//! idempotently (running twice changes nothing). Each agent is configured in its own native
//! format:
//!
//! - **Claude Code** — project `.mcp.json` (the `cairn` MCP server) **and** `.claude/settings.json`
//!   lifecycle hooks (SessionStart/UserPromptSubmit/PostToolUse/SessionEnd). The only agent with a
//!   hook system, so the only one that gets hooks.
//! - **Cursor** — project `.cursor/mcp.json` (`mcpServers` schema).
//! - **VS Code** — project `.vscode/mcp.json` (`servers` schema).
//! - **Windsurf** — `~/.codeium/windsurf/mcp_config.json` (`mcpServers` schema).
//!
//! `--all` configures only the agents it actually detects (project markers or home-dir install);
//! naming an agent explicitly configures it regardless.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Agents Cairn can configure, in the order `--all` tries them.
const KNOWN: &[&str] = &["claude-code", "cursor", "vscode", "windsurf"];

pub fn run(agent: Option<&str>, all: bool) -> Result<()> {
    let project = std::env::current_dir()?;
    let home = home_dir();

    if all {
        let mut configured = 0usize;
        for id in KNOWN {
            if detect(id, &project, home.as_deref()) {
                install_agent(id, &project, home.as_deref())?;
                configured += 1;
            }
        }
        if configured == 0 {
            println!("cairn: no supported agents detected here or in your home directory.");
            println!("Install one explicitly, e.g. `cairn install claude-code`.");
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
    install_agent(id, &project, home.as_deref())?;
    println!("\nStart the server with `cairn serve`, then open a session in your agent.");
    Ok(())
}

/// Map an agent name (and its aliases) to a canonical id, or `None` if unknown.
fn canonical_id(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "claude-code" | "claude" | "claudecode" | "cc" => Some("claude-code"),
        "cursor" => Some("cursor"),
        "vscode" | "code" | "vs-code" => Some("vscode"),
        "windsurf" | "codeium" => Some("windsurf"),
        _ => None,
    }
}

/// Is this agent present (project marker or home-dir install)? Used to scope `--all`.
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
        _ => false,
    }
}

fn install_agent(id: &str, project: &Path, home: Option<&Path>) -> Result<()> {
    match id {
        "claude-code" => install_claude_code(project),
        "cursor" => install_mcp_only(
            "Cursor",
            &project.join(".cursor").join("mcp.json"),
            "mcpServers",
        ),
        "vscode" => install_mcp_only(
            "VS Code",
            &project.join(".vscode").join("mcp.json"),
            "servers",
        ),
        "windsurf" => {
            let home = home.context("cannot locate your home directory for the Windsurf config")?;
            install_mcp_only(
                "Windsurf",
                &home
                    .join(".codeium")
                    .join("windsurf")
                    .join("mcp_config.json"),
                "mcpServers",
            )
        }
        other => bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    }
}

/// The MCP server entry every agent points at.
fn cairn_server() -> Value {
    json!({ "command": "cairn", "args": ["mcp"] })
}

/// Merge the `cairn` MCP server into `path` under `schema_key`, then report it.
fn install_mcp_only(label: &str, path: &Path, schema_key: &str) -> Result<()> {
    merge_mcp_server(path, schema_key)?;
    println!("\u{2713} Configured {label}:");
    println!("  - {}  (MCP server: cairn)", path.display());
    Ok(())
}

/// Insert the `cairn` server under `schema_key` in the JSON object at `path` (creating the file
/// and parents as needed), preserving everything else.
fn merge_mcp_server(path: &Path, schema_key: &str) -> Result<()> {
    let mut obj = read_object(path)?;
    let servers = obj
        .entry(schema_key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{}: '{schema_key}' is not an object", path.display()))?;
    servers.insert("cairn".into(), cairn_server());
    write_json(path, &Value::Object(obj))
}

fn install_claude_code(dir: &Path) -> Result<()> {
    // 1) Register the MCP server in .mcp.json.
    let mcp_path = dir.join(".mcp.json");
    merge_mcp_server(&mcp_path, "mcpServers")?;

    // 2) Add lifecycle hooks in .claude/settings.json.
    let settings_path = dir.join(".claude").join("settings.json");
    let mut settings = read_object(&settings_path)?;
    {
        let hooks = settings
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("settings.json: hooks is not an object")?;
        add_hook(hooks, "SessionStart", "cairn hook SessionStart", None);
        add_hook(
            hooks,
            "UserPromptSubmit",
            "cairn hook UserPromptSubmit",
            None,
        );
        add_hook(
            hooks,
            "PostToolUse",
            "cairn hook PostToolUse",
            Some("Edit|Write|MultiEdit|NotebookEdit"),
        );
        add_hook(hooks, "SessionEnd", "cairn hook SessionEnd", None);
    }
    write_json(&settings_path, &Value::Object(settings))?;

    println!("\u{2713} Configured Claude Code:");
    println!("  - {}  (MCP server: cairn)", mcp_path.display());
    println!(
        "  - {}  (hooks: SessionStart, UserPromptSubmit, PostToolUse, SessionEnd)",
        settings_path.display()
    );
    Ok(())
}

/// Add a command hook for `event` (optionally scoped to a tool `matcher`) if an identical one
/// isn't already present.
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

/// Read a JSON object from `path`, or an empty object if missing/blank/non-object.
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
    fn claude_code_install_is_idempotent_and_non_destructive() {
        let dir = tempfile::tempdir().unwrap();
        // Pre-existing settings with an unrelated hook and key.
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"model":"opus","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
        )
        .unwrap();

        install_claude_code(dir.path()).unwrap();
        install_claude_code(dir.path()).unwrap(); // twice — must not duplicate

        let settings = read_json(&dir.path().join(".claude/settings.json"));
        assert_eq!(settings["model"], "opus"); // unrelated key preserved
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        let cairn_count = starts
            .iter()
            .filter(|g| g["hooks"][0]["command"] == "cairn hook SessionStart")
            .count();
        assert_eq!(cairn_count, 1, "cairn hook added exactly once");
        assert!(starts.iter().any(|g| g["hooks"][0]["command"] == "echo hi"));
        assert!(settings["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["hooks"][0]["command"] == "cairn hook PostToolUse"));

        let mcp = read_json(&dir.path().join(".mcp.json"));
        assert_eq!(mcp["mcpServers"]["cairn"]["command"], "cairn");
    }

    #[test]
    fn cursor_and_vscode_use_their_own_paths_and_schemas() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();

        install_agent("cursor", p, None).unwrap();
        let cursor = read_json(&p.join(".cursor/mcp.json"));
        assert_eq!(cursor["mcpServers"]["cairn"]["command"], "cairn");
        assert_eq!(cursor["mcpServers"]["cairn"]["args"][0], "mcp");

        install_agent("vscode", p, None).unwrap();
        let vscode = read_json(&p.join(".vscode/mcp.json"));
        // VS Code uses the `servers` key, not `mcpServers`.
        assert_eq!(vscode["servers"]["cairn"]["command"], "cairn");
        assert!(vscode.get("mcpServers").is_none());
    }

    #[test]
    fn windsurf_writes_into_the_home_config_and_preserves_existing() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        // Pre-existing Windsurf config with an unrelated server.
        let cfg = home.path().join(".codeium/windsurf/mcp_config.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        fs::write(&cfg, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();

        install_agent("windsurf", project.path(), Some(home.path())).unwrap();

        let v = read_json(&cfg);
        assert_eq!(v["mcpServers"]["other"]["command"], "x"); // preserved
        assert_eq!(v["mcpServers"]["cairn"]["command"], "cairn"); // added
    }

    #[test]
    fn detect_scopes_all_to_present_agents() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let (p, h) = (project.path(), home.path());

        // Nothing present yet.
        assert!(!detect("cursor", p, Some(h)));
        assert!(!detect("vscode", p, Some(h)));
        assert!(!detect("windsurf", p, Some(h)));

        // Project markers / home installs flip detection on.
        fs::create_dir_all(p.join(".cursor")).unwrap();
        fs::create_dir_all(p.join(".vscode")).unwrap();
        fs::create_dir_all(h.join(".codeium/windsurf")).unwrap();
        assert!(detect("cursor", p, Some(h)));
        assert!(detect("vscode", p, Some(h)));
        assert!(detect("windsurf", p, Some(h)));
    }

    #[test]
    fn canonical_id_resolves_aliases_and_rejects_unknown() {
        assert_eq!(canonical_id("claude"), Some("claude-code"));
        assert_eq!(canonical_id("Cursor"), Some("cursor"));
        assert_eq!(canonical_id("code"), Some("vscode"));
        assert_eq!(canonical_id("codeium"), Some("windsurf"));
        assert!(canonical_id("emacs").is_none());
    }
}

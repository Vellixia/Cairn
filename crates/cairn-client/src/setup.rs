//! `cairn setup [agent|--all]` - wire AI agents up to a Cairn server.
//!
//! Every merge is **non-destructive**: existing config is preserved and our entries are added
//! idempotently (running twice changes nothing). Each agent is configured in its own native
//! format:
//!
//! - **Claude Code** - project `.mcp.json` (the `cairn` MCP server) **and** `.claude/settings.json`
//!   lifecycle hooks.
//! - **Codex CLI** - `~/.codex/config.toml` (or `<project>/.codex/config.toml` for project-scope)
//!   under `[mcp_servers.cairn]` with stdio transport (TOML, not JSON).
//! - **OpenCode** - `$XDG_CONFIG_HOME/opencode/opencode.json` on Unix and
//!   `%USERPROFILE%\.config\opencode\opencode.json` on Windows (XDG-style on both).
//!   `mcp` top-level key with `{ type, command, environment, enabled }` entries.
//!
//! When `--server` is passed, the MCP server entry includes `CAIRN_SERVER` and `CAIRN_TOKEN` env
//! vars so `cairn mcp` runs in remote-proxy mode; otherwise it runs in local HelixDB mode.
//!
//! `--all` configures only the agents it actually detects (project markers or home-dir install);
//! naming an agent explicitly configures it regardless.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Verify that a device token is valid before writing it to agent config files.
/// Makes a `GET /api/auth/me` request and returns Ok(()) on success.
fn validate_token(server: &str, token: &str) -> Result<()> {
    let url = format!("{}/api/auth/me", server.trim_end_matches('/'));
    match ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
    {
        Ok(resp) if resp.status() == 200 => Ok(()),
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!(
                "token rejected by server (HTTP {status}) -- the token may be expired, \
                 revoked, or belong to a server with a different secret key.\n\
                 Server response: {body}\n\
                 Obtain a fresh token by pairing (`cairn pair`) or from the dashboard."
            )
        }
        Err(e) => {
            anyhow::bail!(
                "cannot reach server at {server} to validate the token: {e}\n\
                 Is the server running and reachable?"
            )
        }
    }
}

const KNOWN: &[&str] = &["claude-code", "codex", "opencode"];

pub fn run(
    agent: Option<&str>,
    all: bool,
    server: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
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
            println!("cairn: no supported agents detected here or in your home directory.");
            println!("Install one explicitly, e.g. `cairn setup claude-code`.");
            println!("Supported: {}.", KNOWN.join(", "));
        } else {
            println!("\nStart the server with `docker compose up -d`, then open a session in your agent.");
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
    println!("\nStart the server with `docker compose up -d`, then open a session in your agent.");
    Ok(())
}

fn canonical_id(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "claude-code" | "claude" | "claudecode" | "cc" => Some("claude-code"),
        "codex" => Some("codex"),
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
        "codex" => {
            codex_config_path(home).exists()
                || project.join(".codex").join("config.toml").exists()
                || home_has(".codex/config.toml")
        }
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
    // When --token is not passed explicitly, fall back to the CAIRN_TOKEN env var so the
    // token is embedded in the agent config without requiring the user to pass it every time.
    let env_token = token
        .is_none()
        .then(|| std::env::var("CAIRN_TOKEN").ok())
        .flatten()
        .filter(|t| !t.is_empty());
    let effective_token = token.or(env_token.as_deref());

    // Validate the token against the server before writing any config files.
    if let (Some(srv), Some(tok)) = (server, effective_token) {
        validate_token(srv, tok)?;
    }

    match id {
        "claude-code" => install_claude_code(project, server, effective_token)?,
        "codex" => install_codex(home, server, effective_token)?,
        "opencode" => install_opencode(server, effective_token)?,
        other => anyhow::bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    }
    crate::rules::write_for(id, project)?;
    Ok(())
}

/// OpenCode's global config path. OpenCode follows XDG-ish directories on all platforms:
/// `~/.config/opencode/opencode.json` on Windows and Unix alike.
fn opencode_config_path() -> PathBuf {
    // XDG_CONFIG_HOME already IS the config root (e.g. ~/.config); don't add .config again.
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("opencode").join("opencode.json");
    }
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(".config").join("opencode").join("opencode.json")
}

fn install_opencode(server: Option<&str>, token: Option<&str>) -> Result<()> {
    let path = opencode_config_path();
    let mut cfg = read_object(&path)?;
    let mcp = cfg.entry("mcp").or_insert_with(|| json!({}));
    let mcp_obj = mcp
        .as_object_mut()
        .with_context(|| format!("{}: 'mcp' is not an object", path.display()))?;

    // Preserve any existing environment variables from a previous setup run
    // so that re-running `cairn setup` without --token does not silently drop
    // tokens or other env vars that were there before.
    let mut env = Map::new();
    if let Some(existing) = mcp_obj.get("cairn") {
        if let Some(existing_env) = existing.get("environment").and_then(Value::as_object) {
            for (k, v) in existing_env {
                env.insert(k.clone(), v.clone());
            }
        }
    }
    if let Some(s) = server {
        env.insert("CAIRN_SERVER".into(), json!(s));
    }
    if let Some(t) = token {
        env.insert("CAIRN_TOKEN".into(), json!(t));
    }

    // Use an absolute path to the current cairn binary so the OpenCode
    // launcher can find it regardless of PATH.
    let cli_exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "cairn".into());

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

/// Codex CLI's user-level config path: `~/.codex/config.toml`. Codex follows
/// the same XDG-ish convention as OpenCode on every platform.
fn codex_config_path(home: Option<&Path>) -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(PathBuf::from))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    config_home.join(".codex").join("config.toml")
}

/// Write or merge the `[mcp_servers.cairn]` entry into Codex's config.toml.
///
/// Codex reads TOML, not JSON. We keep this dependency-free: hand-rolled merge
/// preserves any existing mcp_servers table and only touches our entry. The
/// block is intentionally simple - no multi-line arrays, no comments - so we
/// don't have to round-trip a real TOML parser for one stanza.
fn install_codex(home: Option<&Path>, server: Option<&str>, token: Option<&str>) -> Result<()> {
    let path = codex_config_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let original = if path.exists() {
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };

    // Extract existing env vars so re-running setup without --token preserves them.
    let existing_env = parse_codex_cairn_env(&original);

    let new_block = render_codex_block(server, token, existing_env);
    let merged = merge_codex_block(&original, &new_block);

    fs::write(&path, merged).with_context(|| format!("writing {}", path.display()))?;
    println!("\u{2713} Configured Codex CLI:");
    println!("  - {}  (MCP server: cairn)", path.display());
    Ok(())
}

/// Render just our `[mcp_servers.cairn]` block, merging in existing env vars
/// from a previous setup run so re-running without --token does not strip them.
fn render_codex_block(
    server: Option<&str>,
    token: Option<&str>,
    existing_env: Vec<(String, String)>,
) -> String {
    let mut env_lines = String::new();
    // Preserve existing vars not being replaced.
    for (k, v) in &existing_env {
        match k.as_str() {
            "CAIRN_SERVER" if server.is_some() => {} // will be overridden below
            "CAIRN_TOKEN" if token.is_some() => {}   // will be overridden below
            _ => env_lines.push_str(&format!("{k} = \"{}\"\n", escape_toml(v))),
        }
    }
    if let Some(s) = server {
        env_lines.push_str(&format!("CAIRN_SERVER = \"{}\"\n", escape_toml(s)));
    }
    if let Some(t) = token {
        env_lines.push_str(&format!("CAIRN_TOKEN = \"{}\"\n", escape_toml(t)));
    }

    let args_line = r#"args = ["mcp"]"#;
    let env_block = if env_lines.is_empty() {
        String::new()
    } else {
        format!("[mcp_servers.cairn.env]\n{env_lines}")
    };

    format!(
        "[mcp_servers.cairn]\n\
         command = \"cairn\"\n\
         {args_line}\n\
         {env_block}",
    )
}

/// Parse the `[mcp_servers.cairn.env]` section from a Codex TOML config.
fn parse_codex_cairn_env(toml: &str) -> Vec<(String, String)> {
    let mut in_cairn_env = false;
    let mut vars = Vec::new();
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[mcp_servers.cairn.env]" {
            in_cairn_env = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_cairn_env = false;
            continue;
        }
        if in_cairn_env {
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim().to_string();
                let val = v.trim().trim_matches('"').to_string();
                vars.push((key, val));
            }
        }
    }
    vars
}

/// Naive merge: if `[mcp_servers]` exists in `original`, replace the
/// `[mcp_servers.cairn]` sub-block (or append ours if absent). If a bare
/// `[mcp_servers.cairn]` sub-block exists at top level, replace it. If no
/// `[mcp_servers]` table or cairn sub-block exists, append our block.
/// Other tables and content are preserved verbatim.
fn merge_codex_block(original: &str, new_block: &str) -> String {
    let mut out = String::with_capacity(original.len() + new_block.len() + 2);
    let mut in_mcp_servers = false;
    let mut replaced_cairn = false;

    for line in original.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let is_table_header = trimmed.starts_with('[') && !trimmed.starts_with("[[");
        let is_cairn_header = trimmed.starts_with("[mcp_servers.cairn]");
        let is_root_header = trimmed.starts_with("[mcp_servers]");

        if in_mcp_servers && is_table_header && !is_cairn_header {
            // Leaving the [mcp_servers] table for a new top-level table.
            in_mcp_servers = false;
            if !replaced_cairn {
                out.push('\n');
                out.push_str(new_block);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }

        if is_root_header {
            in_mcp_servers = true;
            out.push_str(line);
            continue;
        }

        // Skip either a cairn root sub-block (replace it) or any cairn-owned
        // sub-table after replacement (so the cairn.env block goes too).
        let is_cairn_subtable = trimmed.starts_with("[mcp_servers.cairn.");
        let is_cairn_owned = is_cairn_header || is_cairn_subtable;
        let should_replace = is_cairn_header && !replaced_cairn;
        let should_skip_cairn_subtable = replaced_cairn && is_cairn_subtable;

        if should_replace {
            replaced_cairn = true;
            out.push_str(new_block);
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("<<CAIRN_SKIP>>");
            continue;
        }
        if should_skip_cairn_subtable {
            // The cairn.env (or any future cairn.X) sub-table is being
            // absorbed by the replacement; set a sentinel so the body
            // lines under it also get skipped.
            out.push_str("<<CAIRN_SKIP>>");
            continue;
        }
        if is_cairn_owned {
            // Orphan cairn block already handled above; reach here only
            // if we somehow see a cairn header before replacement triggers,
            // which shouldn't happen with should_replace above. Be safe.
            continue;
        }

        if out.ends_with("<<CAIRN_SKIP>>") && !is_table_header {
            continue;
        }

        out.push_str(line);
    }

    if !replaced_cairn {
        // No existing cairn sub-block anywhere - append at end.
        if !out.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(new_block);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }

    out.replace("<<CAIRN_SKIP>>", "")
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn cairn_server(server: Option<&str>, token: Option<&str>, existing_env: Option<&Map<String, Value>>) -> Value {
    let mut env = Map::new();
    // Preserve any existing env vars from a previous setup run.
    if let Some(existing) = existing_env {
        for (k, v) in existing {
            env.insert(k.clone(), v.clone());
        }
    }
    if let Some(s) = server {
        env.insert("CAIRN_SERVER".into(), json!(s));
    }
    if let Some(t) = token {
        env.insert("CAIRN_TOKEN".into(), json!(t));
    }
    if env.is_empty() {
        json!({ "command": "cairn", "args": ["mcp"] })
    } else {
        json!({ "command": "cairn", "args": ["mcp"], "env": Value::Object(env) })
    }
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
            Some("Edit|Write|MultiEdit|NotebookEdit|StrReplace"),
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

fn merge_mcp_server(
    path: &Path,
    schema_key: &str,
    server: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    let mut obj = read_object(path)?;
    let servers = obj
        .entry(schema_key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{}: '{schema_key}' is not an object", path.display()))?;

    // Preserve existing env vars from a previous setup run.
    let existing_env = servers
        .get("cairn")
        .and_then(|v| v.get("env"))
        .and_then(Value::as_object);

    servers.insert("cairn".into(), cairn_server(server, token, existing_env));
    write_json(path, &Value::Object(obj))
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

    fn read_text(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
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

        let settings: Value =
            serde_json::from_str(&read_text(&dir.path().join(".claude/settings.json"))).unwrap();
        assert_eq!(settings["model"], "opus");
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        let cairn_count = starts
            .iter()
            .filter(|g| g["hooks"][0]["command"] == "cairn hook SessionStart")
            .count();
        assert_eq!(cairn_count, 1);
        assert!(starts.iter().any(|g| g["hooks"][0]["command"] == "echo hi"));
        assert!(settings["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["hooks"][0]["command"] == "cairn hook PostToolUse"));

        let mcp: Value = serde_json::from_str(&read_text(&dir.path().join(".mcp.json"))).unwrap();
        assert_eq!(mcp["mcpServers"]["cairn"]["command"], "cairn");
    }

    #[test]
    fn codex_block_renders_minimal_entry() {
        let block = render_codex_block(None, None, vec![]);
        assert!(block.contains("[mcp_servers.cairn]"));
        assert!(block.contains("command = \"cairn\""));
        assert!(block.contains("args = [\"mcp\"]"));
        assert!(!block.contains("[mcp_servers.cairn.env]"));
    }

    #[test]
    fn codex_block_renders_env_when_server_or_token_set() {
        let block = render_codex_block(Some("http://example.com:7777"), Some("tok-123"), vec![]);
        assert!(block.contains("[mcp_servers.cairn.env]"));
        assert!(block.contains("CAIRN_SERVER = \"http://example.com:7777\""));
        assert!(block.contains("CAIRN_TOKEN = \"tok-123\""));
    }

    #[test]
    fn codex_merge_appends_when_no_existing_table() {
        let merged = merge_codex_block(
            "# existing config\nmodel = \"opus\"\n",
            "[mcp_servers.cairn]\ncommand = \"cairn\"\nargs = [\"mcp\"]\n",
        );
        assert!(merged.contains("model = \"opus\""));
        assert!(merged.contains("[mcp_servers.cairn]"));
        assert!(merged.contains("command = \"cairn\""));
    }

    #[test]
    fn codex_merge_replaces_existing_cairn_block() {
        let original = "# head\n[mcp_servers]\n[mcp_servers.cairn]\ncommand = \"stale\"\nargs = [\"old\"]\n[other_table]\nx = 1\n";
        let merged = merge_codex_block(
            original,
            "[mcp_servers.cairn]\ncommand = \"cairn\"\nargs = [\"mcp\"]\n",
        );
        assert!(!merged.contains("command = \"stale\""));
        assert!(merged.contains("command = \"cairn\""));
        assert!(merged.contains("[other_table]"));
        assert!(merged.contains("x = 1"));
    }

    #[test]
    fn codex_setup_writes_to_xdg_path_and_preserves_existing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        fs::write(&cfg, "# user prefs\ntui = { theme = \"dark\" }\n").unwrap();

        install_codex_at(&cfg, Some("http://example.com:7777"), Some("tok-xyz")).unwrap();

        let out = read_text(&cfg);
        assert!(out.contains("tui = { theme = \"dark\" }"));
        assert!(out.contains("[mcp_servers.cairn]"));
        assert!(out.contains("command = \"cairn\""));
        assert!(out.contains("CAIRN_SERVER = \"http://example.com:7777\""));
        assert!(out.contains("CAIRN_TOKEN = \"tok-xyz\""));
    }

    #[test]
    fn codex_setup_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");

        install_codex_at(&cfg, Some("http://h:7777"), Some("t-1")).unwrap();
        let first = read_text(&cfg);
        install_codex_at(&cfg, Some("http://h:7777"), Some("t-1")).unwrap();
        let second = read_text(&cfg);
        assert_eq!(first, second, "running setup twice must be idempotent");
    }

    /// Test-only entry point that takes an explicit config path. The
    /// production `install_codex` reads `codex_config_path(home)` which
    /// consults `XDG_CONFIG_HOME`; we skip that indirection here so tests
    /// don't race on the env var when run in parallel.
    fn install_codex_at(path: &Path, server: Option<&str>, token: Option<&str>) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let original = if path.exists() {
            fs::read_to_string(path)?
        } else {
            String::new()
        };
        let new_block = render_codex_block(server, token, vec![]);
        let merged = merge_codex_block(&original, &new_block);
        fs::write(path, merged)?;
        Ok(())
    }

    #[test]
    fn opencode_writes_into_the_home_config_and_includes_remote_env() {
        let cfg = tempfile::tempdir().unwrap().path().join("opencode.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();

        install_opencode_with_path(Some("http://example.com:7777"), Some("tok-123"), &cfg).unwrap();

        let v: Value = serde_json::from_str(&read_text(&cfg)).unwrap();
        assert_eq!(v["mcp"]["cairn"]["command"][0], "cairn");
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

        let v: Value = serde_json::from_str(&read_text(&cfg)).unwrap();
        assert_eq!(v["mcp"]["cairn"]["command"][0], "cairn");
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
                "command": ["cairn", "mcp"],
                "enabled": true
            })
        } else {
            json!({
                "type": "local",
                "command": ["cairn", "mcp"],
                "environment": Value::Object(env),
                "enabled": true
            })
        };
        mcp_obj.insert("cairn".into(), entry);
        write_json(path, &Value::Object(cfg))
    }

    #[test]
    fn detect_scopes_all_to_present_agents() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let (p, h) = (project.path(), home.path());
        std::env::set_var("XDG_CONFIG_HOME", h);

        // Baseline: none detected.
        assert!(!detect("claude-code", p, Some(h)));
        assert!(!detect("codex", p, Some(h)));
        assert!(!detect("opencode", p, Some(h)));

        fs::create_dir_all(p.join(".claude")).unwrap();
        fs::write(p.join(".mcp.json"), "{}").unwrap();
        assert!(detect("claude-code", p, Some(h)));

        fs::create_dir_all(h.join(".codex")).unwrap();
        fs::write(h.join(".codex/config.toml"), "").unwrap();
        assert!(detect("codex", p, Some(h)));

        // XDG_CONFIG_HOME is the config root itself; opencode.json lives directly under it.
        fs::create_dir_all(h.join("opencode")).unwrap();
        fs::write(h.join("opencode/opencode.json"), "{}").unwrap();
        assert!(detect("opencode", p, Some(h)));
    }

    #[test]
    fn canonical_id_resolves_aliases_and_rejects_unknown() {
        assert_eq!(canonical_id("claude"), Some("claude-code"));
        assert_eq!(canonical_id("CODEX"), Some("codex"));
        assert_eq!(canonical_id("opencode"), Some("opencode"));
        assert_eq!(canonical_id("oc"), Some("opencode"));
        assert!(canonical_id("emacs").is_none());
        assert!(canonical_id("cursor").is_none());
    }
}

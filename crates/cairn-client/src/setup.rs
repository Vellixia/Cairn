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
//! vars so `cairn mcp` runs in remote-proxy mode; otherwise it runs in local mode.
//!
//! `--all` configures only the agents it actually detects (project markers or home-dir install);
//! naming an agent explicitly configures it regardless.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Absolute path to the current cairn binary, with a "cairn" fallback.
/// Used in agent config files so the MCP server and hooks work regardless
/// of PATH resolution (especially on Windows).
fn cairn_exe() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cairn".to_string())
}

/// Normalize a hook command string to its canonical form for dedup comparison.
///
/// Returns just the suffix after the cairn binary path (e.g. `"hook SessionStart"`),
/// ignoring whether the original was `"cairn hook SessionStart"`,
/// `"D:\\code\\Cairn\\target\\debug\\cairn.exe hook SessionStart"`, or any other
/// absolute/bare path to the same binary. Used to coalesce duplicates left by
/// multiple `cairn setup` runs from different binary locations.
#[cfg(test)]
fn hook_suffix(command: &str) -> String {
    // Strategy: tokenize on whitespace, look for a leading token whose basename
    // (after stripping any path separators) is `cairn` or `cairn.exe`, and treat
    // the next token as the subcommand. If subcommand is "hook", return the rest
    // of the string. Case-insensitive throughout.
    let mut tokens = command.split_whitespace();
    let first = match tokens.next() {
        Some(t) => t,
        None => return command.to_string(),
    };
    let first_lower = first.to_ascii_lowercase();
    let basename = first_lower
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(&first_lower);
    let is_cairn_exe = basename == "cairn" || basename == "cairn.exe";
    if !is_cairn_exe {
        return command.to_string();
    }
    let rest_tokens: Vec<&str> = tokens.collect();
    if rest_tokens.is_empty() {
        return command.to_string();
    }
    // Skip the literal "hook" subcommand token; everything after is the event name.
    if rest_tokens[0].eq_ignore_ascii_case("hook") {
        rest_tokens[1..].join(" ")
    } else {
        command.to_string()
    }
}

/// True when `command` is a `cairn hook <event>` invocation, regardless of whether
/// the binary is referenced by bare name or absolute path.
#[cfg(test)]
pub fn is_cairn_hook(command: &str, event: &str) -> bool {
    let suffix = hook_suffix(command).to_ascii_lowercase();
    // `hook_suffix` returns the event name(s) without the "hook" subcommand token.
    // Compare against the event exactly; surrounding quotes (legacy codex paths)
    // are also accepted because some agents wrap event names in quotes.
    let event_lower = event.to_ascii_lowercase();
    suffix == event_lower || suffix == format!("\"{event_lower}\"")
}

/// True when `command` is any `cairn hook <anything>` invocation (event-agnostic).
/// Used by `cairn reset` to strip every cairn-owned hook regardless of event name.
pub fn is_any_cairn_hook(command: &str) -> bool {
    // `hook_suffix` strips the binary + "hook" subcommand tokens; if it found a
    // cairn binary in front, what remains is the event name. If it didn't find
    // one, it returns the full command unchanged -- in which case this returns false.
    let original = command.trim_start().to_ascii_lowercase();
    let first = original.split_whitespace().next().unwrap_or("");
    let basename = first.rsplit(['\\', '/']).next().unwrap_or(first);
    basename == "cairn" || basename == "cairn.exe"
}

/// Drop pre-existing cairn-owned entries for the given event so a re-run replaces
/// stale bare/path duplicates with the current absolute-path entry. Returns the
/// number of entries removed.
///
/// `event` is the agent-specific event name (e.g. `"Stop"` in Codex maps to
/// `cairn hook SessionEnd`), so we match on `is_any_cairn_hook` rather than the
/// event-scoped `is_cairn_hook`. This collapses bare-name and absolute-path
/// duplicates from prior setup runs regardless of which cairn event they bridge to.
fn dedup_cairn_hooks(arr: &mut Vec<Value>, _event: &str) -> usize {
    let before = arr.len();
    arr.retain(|g| {
        // Keep the group iff none of its inner commands is a cairn hook invocation.
        !g.get("hooks")
            .and_then(|v| v.as_array())
            .map(|hs| {
                hs.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .is_some_and(is_any_cairn_hook)
                })
            })
            .unwrap_or(false)
    });
    before - arr.len()
}

/// Verify that a device token is valid before writing it to agent config files.
/// Makes an authenticated `GET /api/memory/wakeup?limit=1` request and returns Ok(())
/// when the server answers 200.
fn validate_token(server: &str, token: &str) -> Result<()> {
    let url = format!("{}/api/memory/wakeup?limit=1", server.trim_end_matches('/'));
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
    project_flag: bool,
) -> Result<()> {
    // Fall back to CAIRN_SERVER env var when --server is not passed explicitly.
    let env_server = server
        .is_none()
        .then(|| std::env::var("CAIRN_SERVER").ok())
        .flatten();
    let effective_server = server.or(env_server.as_deref());
    let project = std::env::current_dir()?;
    let home = home_dir();

    // `--project` overrides the default global scope for Claude Code so the user
    // can opt into per-project config when they want it. Other agents ignore scope
    // because their config locations are inherently user-level (Codex: ~/.codex;
    // OpenCode: ~/.config/opencode).
    let scope = if project_flag {
        Scope::Project
    } else {
        Scope::Global
    };

    if all {
        let mut configured = 0usize;
        for id in KNOWN {
            if detect(id, &project, home.as_deref()) {
                install_agent(
                    id,
                    &project,
                    home.as_deref(),
                    effective_server,
                    token,
                    scope,
                )?;
                configured += 1;
            }
        }
        if configured == 0 {
            println!("cairn: no supported agents detected here or in your home directory.");
            println!("Install one explicitly, e.g. `cairn setup claude-code`.");
            println!("Supported: {}.", KNOWN.join(", "));
        } else if let Some(srv) = effective_server {
            println!("\nCairn server: {srv}. Open a session in your agent.");
        } else {
            println!("\nNo server configured. Run with --server <url> or set CAIRN_SERVER.");
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
    install_agent(
        id,
        &project,
        home.as_deref(),
        effective_server,
        token,
        scope,
    )?;
    if let Some(srv) = effective_server {
        println!("\nCairn server: {srv}. Open a session in your agent.");
    } else {
        println!("\nNo server configured. Run with --server <url> or set CAIRN_SERVER.");
    }
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
    scope: Scope,
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
        "claude-code" => install_claude_code(project, home, scope, server, effective_token)?,
        "codex" => install_codex(home, server, effective_token)?,
        "opencode" => install_opencode(server, effective_token)?,
        other => anyhow::bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    }
    crate::rules::write_for(id, project)?;
    Ok(())
}

/// Config-file scope for agent installation.
///
/// `Global` writes to the user-level config (e.g. `~/.claude.json`, `~/.config/opencode/opencode.json`)
/// so the same setup applies to every project the user opens. `Project` writes to a
/// per-project location (e.g. `<cwd>/.mcp.json`, `<cwd>/.claude/settings.json`) so the
/// configuration only takes effect in the current repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
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

    // Write the plugin file. OpenCode auto-loads every .js/.ts file in its `plugins/`
    // directory at startup, so there is nothing to register in opencode.json. The
    // `plugin` array is for npm packages; listing a local path there makes OpenCode
    // load the plugin twice (every lifecycle hook would fire twice). Strip any such
    // entry an older cairn version may have written so upgrades self-heal.
    let plugin_path = write_opencode_plugin()?;
    strip_cairn_plugin_entries(&mut cfg);

    write_json(&path, &Value::Object(cfg))?;
    println!("✓ Configured OpenCode:");
    println!("  - {}  (MCP server: {})", path.display(), cli_exe);
    println!(
        "  - {}  (plugin: session + tool hooks, auto-loaded)",
        plugin_path.display()
    );

    let _ = (server, token);
    Ok(())
}

/// Remove any cairn plugin entries from opencode.json's `plugin` array.
///
/// OpenCode auto-loads every local plugin file in `~/.config/opencode/plugins/` at
/// startup, so the cairn plugin must NOT be listed here — that array is for npm
/// packages, and a local path in it makes OpenCode load the plugin a second time
/// (firing every lifecycle hook twice). Older cairn versions wrote such an entry;
/// stripping it on every `setup` makes upgrades self-heal. Only touches the array
/// when it already exists (never creates an empty `plugin` key).
fn strip_cairn_plugin_entries(cfg: &mut Map<String, Value>) {
    if let Some(plugins) = cfg.get_mut("plugin").and_then(Value::as_array_mut) {
        plugins.retain(|p| {
            p.as_str()
                .map(|s| {
                    let normalized = s.replace('\\', "/").to_ascii_lowercase();
                    !normalized.ends_with("/plugins/cairn.js") && normalized != "plugins/cairn.js"
                })
                .unwrap_or(true)
        });
    }
}

/// Write a minimal OpenCode plugin that bridges lifecycle events to `cairn hook`.
/// Returns the absolute path to the plugin file so the caller can register it
/// in `opencode.json`.
fn write_opencode_plugin() -> Result<PathBuf> {
    let config_dir = opencode_config_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".opencode"));
    let plugins_dir = config_dir.join("plugins");
    fs::create_dir_all(&plugins_dir)?;
    let plugin_path = plugins_dir.join("cairn.js");

    // Use the absolute path to the cairn binary so the plugin works regardless of
    // PATH resolution at OpenCode startup. serde_json gives us correct JSON
    // escaping (backslashes, quotes) on either Windows or Unix paths.
    let exe_json = serde_json::to_string(&cairn_exe())?;

    let plugin_content = format!(
        r#"// Cairn lifecycle plugin. Bridges OpenCode session/tool events to `cairn hook`.
// Managed by `cairn setup` -- changes will be preserved across runs.
// Uses the OpenCode `Plugin` API (see `@opencode-ai/plugin`) so we can react
// to `chat.message` (UserPromptSubmit equivalent) in addition to session and
// tool events.
// @ts-check
const CAIRN_EXE = {exe_json}

async function fireHook($, event, payload) {{
  try {{
    const body = JSON.stringify(payload ?? {{}})
    await $`echo ${{body}} | "${{CAIRN_EXE}}" hook ${{event}}`.quiet().nothrow()
  }} catch (e) {{
    console.error(`[cairn] hook ${{event}} failed:`, e?.message ?? e)
  }}
}}

/** @type {{ import("@opencode-ai/plugin").Plugin }} */
export const CairnPlugin = async ({{ $ }}) => {{
  try {{
    await $`"${{CAIRN_EXE}}" --version`.quiet().nothrow()
  }} catch {{
    console.warn("[cairn] cairn binary not found at " + CAIRN_EXE + " -- plugin disabled")
    return {{}}
  }}

  return {{
    event: async ({{ event }}) => {{
      const type = event?.type
      if (type === "session.created") {{
        await fireHook($, "SessionStart")
      }} else if (type === "session.deleted" || type === "session.idle") {{
        await fireHook($, "SessionEnd")
      }} else if (
        type === "message.part.updated" &&
        event?.properties?.part?.type === "tool" &&
        event?.properties?.part?.state?.status === "completed"
      ) {{
        const part = event.properties.part
        await fireHook($, "PostToolUse", {{
          tool_name: part.tool ?? "unknown",
          tool_input: part.state?.input ?? {{}},
        }})
      }}
    }},
    "chat.message": async (input, output) => {{
      const text = output?.parts?.map((p) => p?.text ?? "").join("\n") ?? ""
      try {{
        await $`echo ${{JSON.stringify({{ prompt: text }})}} | "${{CAIRN_EXE}}" hook UserPromptSubmit`
          .quiet()
          .nothrow()
      }} catch (e) {{
        console.error("[cairn] hook UserPromptSubmit failed:", e?.message ?? e)
      }}
      return {{ message: output?.message }}
    }},
  }}
}}
"#
    );

    fs::write(&plugin_path, plugin_content)?;
    Ok(plugin_path)
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

    let merged = upsert_codex_cairn(&original, server, token)
        .with_context(|| format!("editing {}", path.display()))?;

    fs::write(&path, merged).with_context(|| format!("writing {}", path.display()))?;
    println!("\u{2713} Configured Codex CLI:");
    println!("  - {}  (MCP server: cairn)", path.display());

    // Write Codex lifecycle hooks so Cairn fires at SessionStart, UserPromptSubmit,
    // PostToolUse, and Stop (=> SessionEnd) automatically.
    if let Some(h) = home {
        write_codex_hooks(h, server, token)?;
    }

    Ok(())
}

/// Write Codex lifecycle hooks to `~/.codex/hooks.json`, idempotently merging
/// with any existing hooks from other tools (e.g. lean-ctx).
fn write_codex_hooks(home: &Path, server: Option<&str>, token: Option<&str>) -> Result<()> {
    let hooks_path = home.join(".codex").join("hooks.json");
    let mut hooks_cfg = read_object(&hooks_path)?;
    let hooks_obj = hooks_cfg
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{}: 'hooks' is not an object", hooks_path.display()))?;

    // Helper: append a hook entry to an event array, dropping any stale cairn-owned
    // duplicates first so the result is idempotent across re-runs from different
    // binary paths (bare `cairn`, debug build, release install).
    let mut add_hook = |event: &str, hook: Value| {
        let arr = hooks_obj
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("hooks entry should be an array");
        dedup_cairn_hooks(arr, event);
        arr.push(hook);
    };

    let exe = cairn_exe();

    add_hook(
        "SessionStart",
        json!({
            "matcher": "startup|resume|clear|compact",
            "hooks": [{ "type": "command", "command": format!("{exe} hook SessionStart") }]
        }),
    );

    add_hook(
        "UserPromptSubmit",
        json!({
            "hooks": [{ "type": "command", "command": format!("{exe} hook UserPromptSubmit") }]
        }),
    );

    add_hook(
        "PostToolUse",
        json!({
            "matcher": "apply_patch|Edit|Write",
            "hooks": [{ "type": "command", "command": format!("{exe} hook PostToolUse") }]
        }),
    );

    add_hook(
        "Stop",
        json!({
            "hooks": [{ "type": "command", "command": format!("{exe} hook SessionEnd") }]
        }),
    );

    write_json(&hooks_path, &Value::Object(hooks_cfg))?;
    println!(
        "  - {}  (hooks: SessionStart, UserPromptSubmit, PostToolUse, Stop)",
        hooks_path.display()
    );
    let _ = (server, token);
    Ok(())
}

/// Upsert the `[mcp_servers.cairn]` table into a Codex `config.toml`, preserving every other
/// table, comment, and whitespace via `toml_edit`. Existing vars in `[mcp_servers.cairn.env]`
/// are kept unless `server`/`token` override them. Idempotent: running twice on the same
/// inputs yields byte-identical output. Replaces the previous hand-rolled string surgery,
/// which mishandled inline comments, `=`/`#` inside quoted values, and multi-line strings.
fn upsert_codex_cairn(original: &str, server: Option<&str>, token: Option<&str>) -> Result<String> {
    use toml_edit::{value, Array, DocumentMut, Item, Table};

    let mut doc = original
        .parse::<DocumentMut>()
        .context("Codex config.toml is not valid TOML; refusing to overwrite it")?;

    // Create `[mcp_servers]` only if absent; when we create it, mark it implicit so we emit
    // `[mcp_servers.cairn]` without a bare empty `[mcp_servers]` header. If it already exists
    // (possibly with other servers or direct keys), leave its formatting untouched.
    let created_servers = !doc.contains_key("mcp_servers");
    let servers = doc
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("config.toml: `mcp_servers` is not a table")?;
    if created_servers {
        servers.set_implicit(true);
    }

    let cairn = servers
        .entry("cairn")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("config.toml: `mcp_servers.cairn` is not a table")?;

    cairn["command"] = value(cairn_exe());
    let mut args = Array::new();
    args.push("mcp");
    cairn["args"] = value(args);

    // Materialize the env sub-table only when there's something to store — either an override
    // to write or a pre-existing env to preserve. Setting only the given keys keeps any other
    // env vars the user added.
    let has_existing_env = cairn.get("env").and_then(Item::as_table).is_some();
    if server.is_some() || token.is_some() || has_existing_env {
        let env = cairn
            .entry("env")
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .context("config.toml: `mcp_servers.cairn.env` is not a table")?;
        if let Some(s) = server {
            env["CAIRN_SERVER"] = value(s);
        }
        if let Some(t) = token {
            env["CAIRN_TOKEN"] = value(t);
        }
    }

    Ok(doc.to_string())
}

fn cairn_server(
    server: Option<&str>,
    token: Option<&str>,
    existing_env: Option<&Map<String, Value>>,
) -> Value {
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
        json!({ "command": cairn_exe(), "args": ["mcp"] })
    } else {
        json!({ "command": cairn_exe(), "args": ["mcp"], "env": Value::Object(env) })
    }
}

fn install_claude_code(
    project: &Path,
    home: Option<&Path>,
    scope: Scope,
    server: Option<&str>,
    token: Option<&str>,
) -> Result<()> {
    // Global (default) writes the MCP entry to `~/.claude.json` so the same setup
    // applies to every project. Project-scope (`--project`) writes to the
    // per-project `.mcp.json`. Lifecycle hooks always go to the project's
    // `.claude/settings.json` because Claude Code's hook system is project-scoped.
    let (mcp_path, mcp_key, scope_label) = match scope {
        Scope::Global => {
            let home = home.ok_or_else(|| {
                anyhow!("global scope requires a home directory (set $HOME or $USERPROFILE)")
            })?;
            (
                home.join(".claude.json"),
                "mcpServers",
                "global (~/.claude.json)",
            )
        }
        Scope::Project => (
            project.join(".mcp.json"),
            "mcpServers",
            "project (.mcp.json)",
        ),
    };
    merge_mcp_server(&mcp_path, mcp_key, server, token)?;

    let settings_path = project.join(".claude").join("settings.json");
    let mut settings = read_object(&settings_path)?;
    {
        let hooks = settings
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("settings.json: hooks is not an object")?;
        let exe = cairn_exe();
        add_hook(
            hooks,
            "SessionStart",
            &format!("{exe} hook SessionStart"),
            None,
        );
        add_hook(
            hooks,
            "UserPromptSubmit",
            &format!("{exe} hook UserPromptSubmit"),
            None,
        );
        add_hook(
            hooks,
            "PostToolUse",
            &format!("{exe} hook PostToolUse"),
            // Claude Code's edit-family tool names. `StrReplace` was never a real tool, so it
            // matched nothing; the file-mutating tools are Edit, Write, MultiEdit, NotebookEdit.
            Some("Edit|Write|MultiEdit|NotebookEdit"),
        );
        add_hook(hooks, "SessionEnd", &format!("{exe} hook SessionEnd"), None);
    }
    write_json(&settings_path, &Value::Object(settings))?;

    println!("✓ Configured Claude Code ({scope_label}):");
    println!("  - {}  (MCP server: cairn)", mcp_path.display());
    println!(
        "  - {}  (hooks: SessionStart, UserPromptSubmit, PostToolUse, SessionEnd)",
        settings_path.display()
    );
    println!("  - Run /mcp in Claude Code to approve the cairn server");
    Ok(())
}

fn add_hook(hooks: &mut Map<String, Value>, event: &str, command: &str, matcher: Option<&str>) {
    let groups = hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut();
    let Some(groups) = groups else { return };

    // Strip any cairn-owned entries for this event first - re-runs from different
    // binary paths (bare `cairn`, debug build, release install) must coalesce into
    // exactly one entry per event regardless of which exe the user ran from.
    dedup_cairn_hooks(groups, event);

    // Also drop any non-cairn entry whose `command` is byte-identical to ours, to
    // guard against accidental manual duplicates without losing other tools' hooks.
    let already_exact = groups.iter().any(|g| {
        g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
        })
    });
    if !already_exact {
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

        install_claude_code(dir.path(), None, Scope::Project, None, None).unwrap();
        install_claude_code(dir.path(), None, Scope::Project, None, None).unwrap();

        let settings: Value =
            serde_json::from_str(&read_text(&dir.path().join(".claude/settings.json"))).unwrap();
        assert_eq!(settings["model"], "opus");
        let exe = super::cairn_exe();
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        let cairn_count = starts
            .iter()
            .filter(|g| g["hooks"][0]["command"] == format!("{exe} hook SessionStart"))
            .count();
        assert_eq!(cairn_count, 1);
        assert!(starts.iter().any(|g| g["hooks"][0]["command"] == "echo hi"));
        assert!(settings["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["hooks"][0]["command"] == format!("{exe} hook PostToolUse")));

        let mcp: Value = serde_json::from_str(&read_text(&dir.path().join(".mcp.json"))).unwrap();
        assert_eq!(mcp["mcpServers"]["cairn"]["command"], exe);
    }

    #[test]
    fn codex_upsert_writes_cairn_table_and_env() {
        let exe = super::cairn_exe();
        let out = upsert_codex_cairn("", Some("http://example.com:7777"), Some("tok-123")).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some(exe.as_str())
        );
        assert_eq!(doc["mcp_servers"]["cairn"]["args"][0].as_str(), Some("mcp"));
        assert_eq!(
            doc["mcp_servers"]["cairn"]["env"]["CAIRN_SERVER"].as_str(),
            Some("http://example.com:7777")
        );
        assert_eq!(
            doc["mcp_servers"]["cairn"]["env"]["CAIRN_TOKEN"].as_str(),
            Some("tok-123")
        );
    }

    #[test]
    fn codex_upsert_omits_env_when_no_server_or_token() {
        let out = upsert_codex_cairn("", None, None).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(
            doc["mcp_servers"]["cairn"].get("env").is_none(),
            "no env sub-table when there is nothing to put in it"
        );
    }

    #[test]
    fn codex_upsert_replaces_stale_cairn_but_keeps_other_servers() {
        let exe = super::cairn_exe();
        let original = "# head\n[mcp_servers.cairn]\ncommand = \"stale\"\nargs = [\"old\"]\n\n[mcp_servers.other]\ncommand = \"foo\"\n";
        let out = upsert_codex_cairn(original, None, None).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some(exe.as_str())
        );
        assert_ne!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some("stale")
        );
        // A foreign MCP server is untouched.
        assert_eq!(doc["mcp_servers"]["other"]["command"].as_str(), Some("foo"));
    }

    #[test]
    fn codex_upsert_preserves_comments_other_tables_and_quoted_specials() {
        // The old hand-rolled parser mangled inline comments and `=`/`#` inside quoted values.
        let original = "\
# user preferences — keep me!
model = \"opus\"  # inline comment

[tui]
theme = \"dark\"

[mcp_servers.other]
command = \"foo\"

[mcp_servers.other.env]
WEIRD = \"a=b#c\"
";
        let out = upsert_codex_cairn(original, Some("http://h:7777"), None).unwrap();
        assert!(out.contains("# user preferences — keep me!"));
        assert!(out.contains("model = \"opus\"  # inline comment"));
        assert!(out.contains("[tui]"));
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        // The tricky quoted value survives byte-for-byte.
        assert_eq!(
            doc["mcp_servers"]["other"]["env"]["WEIRD"].as_str(),
            Some("a=b#c")
        );
        assert_eq!(
            doc["mcp_servers"]["cairn"]["env"]["CAIRN_SERVER"].as_str(),
            Some("http://h:7777")
        );
    }

    #[test]
    fn codex_upsert_is_idempotent() {
        let first = upsert_codex_cairn("", Some("http://h:7777"), Some("t-1")).unwrap();
        let second = upsert_codex_cairn(&first, Some("http://h:7777"), Some("t-1")).unwrap();
        assert_eq!(first, second, "re-running upsert must be byte-identical");
    }

    #[test]
    fn codex_upsert_preserves_existing_token_when_omitted() {
        // First run sets the token; a later run without --token must not drop it.
        let first = upsert_codex_cairn("", Some("http://h:7777"), Some("keep-me")).unwrap();
        let second = upsert_codex_cairn(&first, Some("http://h:7777"), None).unwrap();
        let doc = second.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["env"]["CAIRN_TOKEN"].as_str(),
            Some("keep-me")
        );
    }

    #[test]
    fn codex_setup_writes_to_xdg_path_and_preserves_existing_keys() {
        let exe = super::cairn_exe();
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        fs::write(&cfg, "# user prefs\ntui = { theme = \"dark\" }\n").unwrap();

        install_codex_at(&cfg, Some("http://example.com:7777"), Some("tok-xyz")).unwrap();

        let out = read_text(&cfg);
        assert!(out.contains("# user prefs"));
        assert!(out.contains("tui = { theme = \"dark\" }"));
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some(exe.as_str())
        );
        assert_eq!(
            doc["mcp_servers"]["cairn"]["env"]["CAIRN_SERVER"].as_str(),
            Some("http://example.com:7777")
        );
        assert_eq!(
            doc["mcp_servers"]["cairn"]["env"]["CAIRN_TOKEN"].as_str(),
            Some("tok-xyz")
        );
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
        let merged = upsert_codex_cairn(&original, server, token)?;
        fs::write(path, merged)?;
        Ok(())
    }

    #[test]
    fn opencode_writes_into_the_home_config_and_includes_remote_env() {
        let exe = super::cairn_exe();
        let cfg = tempfile::tempdir().unwrap().path().join("opencode.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();

        install_opencode_with_path(Some("http://example.com:7777"), Some("tok-123"), &cfg).unwrap();

        let v: Value = serde_json::from_str(&read_text(&cfg)).unwrap();
        assert_eq!(v["mcp"]["cairn"]["command"][0], exe);
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
        let exe = super::cairn_exe();
        let cfg = tempfile::tempdir().unwrap().path().join("opencode.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();

        install_opencode_with_path(None, None, &cfg).unwrap();

        let v: Value = serde_json::from_str(&read_text(&cfg)).unwrap();
        assert_eq!(v["mcp"]["cairn"]["command"][0], exe);
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
        let exe = super::cairn_exe();
        let entry = if env.is_empty() {
            json!({
                "type": "local",
                "command": [exe, "mcp"],
                "enabled": true
            })
        } else {
            json!({
                "type": "local",
                "command": [exe, "mcp"],
                "environment": Value::Object(env),
                "enabled": true
            })
        };
        mcp_obj.insert("cairn".into(), entry);
        // Mirror production install_opencode: strip any stale cairn plugin-array entry.
        strip_cairn_plugin_entries(&mut cfg);
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

    #[test]
    fn hook_suffix_strips_binary_paths_case_insensitively() {
        let fake_exe = "C:\\Users\\foo\\.local\\bin\\cairn.exe";
        // Bare-name input strips to the event name only (no "hook" prefix).
        assert_eq!(hook_suffix("cairn hook SessionStart"), "SessionStart");
        // Mixed-case input preserves case in the event name.
        assert_eq!(hook_suffix("CAIRN HOOK SessionStart"), "SessionStart");
        // Absolute-path basename input strips correctly.
        assert_eq!(
            hook_suffix(&format!("{fake_exe} hook SessionStart")),
            "SessionStart"
        );
        // Bare cairn.exe name (no path) also works.
        assert_eq!(
            hook_suffix("cairn.exe hook UserPromptSubmit"),
            "UserPromptSubmit"
        );
        // Absolute path with a different parent dir also strips correctly.
        assert_eq!(
            hook_suffix("D:\\old\\path\\cairn.exe hook PostToolUse"),
            "PostToolUse"
        );
    }

    #[test]
    fn is_cairn_hook_matches_across_path_variants() {
        // Use a synthetic absolute path with `cairn.exe` as the basename so the
        // test doesn't depend on the current test binary's name (which has a hash
        // suffix like `cairn-08d8c1119e25a88c.exe`).
        let fake_exe = "C:\\Users\\foo\\.local\\bin\\cairn.exe";
        assert!(is_cairn_hook("cairn hook PostToolUse", "PostToolUse"));
        assert!(is_cairn_hook(
            &format!("{fake_exe} hook PostToolUse"),
            "PostToolUse"
        ));
        assert!(is_cairn_hook(
            "D:\\old\\path\\cairn.exe hook PostToolUse",
            "PostToolUse"
        ));
        assert!(!is_cairn_hook("echo hi", "PostToolUse"));
        assert!(!is_cairn_hook("cairn hook SessionStart", "PostToolUse"));
    }

    #[test]
    fn add_hook_dedups_by_binary_path() {
        let exe = "C:\\Users\\foo\\.local\\bin\\cairn.exe";
        let mut hooks = Map::new();
        // Simulate a stale debug-build entry left over from a previous run.
        let stale = "D:\\old\\path\\cairn.exe hook PostToolUse";
        let stale_entry = json!({
            "matcher": "Edit|Write",
            "hooks": [{ "type": "command", "command": stale }]
        });
        hooks.insert("PostToolUse".into(), json!([stale_entry]));
        add_hook(
            &mut hooks,
            "PostToolUse",
            &format!("{exe} hook PostToolUse"),
            Some("Edit|Write"),
        );
        let arr = hooks["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "stale entry should have been replaced");
        assert_eq!(
            arr[0]["hooks"][0]["command"],
            json!(format!("{exe} hook PostToolUse"))
        );
    }

    #[test]
    fn claude_code_global_scope_writes_to_home_dot_claude_json() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let exe = super::cairn_exe();

        install_claude_code(project.path(), Some(home.path()), Scope::Global, None, None).unwrap();

        let global_path = home.path().join(".claude.json");
        let v: Value = serde_json::from_str(&read_text(&global_path)).unwrap();
        assert_eq!(v["mcpServers"]["cairn"]["command"], exe);

        // Project .mcp.json must NOT have been written in global scope.
        assert!(!project.path().join(".mcp.json").exists());
    }

    #[test]
    fn claude_code_project_scope_writes_to_dot_mcp_json() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let exe = super::cairn_exe();

        install_claude_code(
            project.path(),
            Some(home.path()),
            Scope::Project,
            None,
            None,
        )
        .unwrap();

        let v: Value = serde_json::from_str(&read_text(&project.path().join(".mcp.json"))).unwrap();
        assert_eq!(v["mcpServers"]["cairn"]["command"], exe);

        // Home .claude.json must NOT have been written in project scope.
        assert!(!home.path().join(".claude.json").exists());
    }

    #[test]
    fn strip_cairn_plugin_entries_removes_all_cairn_paths_keeps_foreign() {
        // A config left double-registered by an older cairn version: the plugin
        // appears both as an absolute path and a bare relative path, alongside a
        // user's own plugin. Stripping must remove every cairn entry and keep the
        // foreign one — OpenCode auto-loads the local file, so none should remain.
        let mut root = Map::new();
        root.insert(
            "plugin".into(),
            json!([
                "C:\\Users\\foo\\.config\\opencode\\plugins\\cairn.js",
                "./plugins/agentmemory-capture.ts",
                "plugins/cairn.js"
            ]),
        );

        strip_cairn_plugin_entries(&mut root);

        let plugins = root["plugin"].as_array().unwrap();
        let cairn_count = plugins
            .iter()
            .filter(|p| {
                p.as_str()
                    .map(|s| s.to_ascii_lowercase().contains("cairn.js"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(cairn_count, 0, "every cairn plugin entry must be stripped");
        assert!(plugins
            .iter()
            .any(|p| p.as_str() == Some("./plugins/agentmemory-capture.ts")));
    }

    #[test]
    fn strip_cairn_plugin_entries_leaves_missing_key_absent() {
        // Never create an empty `plugin` key when there wasn't one.
        let mut root = Map::new();
        strip_cairn_plugin_entries(&mut root);
        assert!(!root.contains_key("plugin"));
    }

    #[test]
    fn install_opencode_strips_stale_plugin_entry_and_never_adds_one() {
        let cfg = tempfile::tempdir().unwrap().path().join("opencode.json");
        fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        // Simulate a config left double-registered by an older cairn version.
        fs::write(
            &cfg,
            r#"{"plugin":["plugins/cairn.js","./plugins/agentmemory-capture.ts"]}"#,
        )
        .unwrap();

        install_opencode_with_path(Some("http://example.com:7777"), Some("tok-123"), &cfg).unwrap();

        let v: Value = serde_json::from_str(&read_text(&cfg)).unwrap();
        // The MCP server is registered...
        assert_eq!(v["mcp"]["cairn"]["command"][0], super::cairn_exe());
        // ...the stale cairn plugin entry is gone (OpenCode auto-loads the local file)...
        let plugins = v["plugin"].as_array().unwrap();
        assert!(
            !plugins.iter().any(|p| p
                .as_str()
                .map(|s| s.to_ascii_lowercase().contains("cairn.js"))
                .unwrap_or(false)),
            "setup must strip the local cairn plugin from the `plugin` array"
        );
        // ...and the user's own plugin survives.
        assert!(plugins
            .iter()
            .any(|p| p.as_str() == Some("./plugins/agentmemory-capture.ts")));
    }
}

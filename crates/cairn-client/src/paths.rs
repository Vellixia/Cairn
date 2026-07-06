//! Shared filesystem paths for agent config files and Cairn's own state dir.
//!
//! Every path helper used to be copy-pasted across `setup.rs`, `doctor.rs`,
//! `status.rs`, and `reset.rs` (up to six near-identical `home_dir()` bodies).
//! One bug fix now touches one function instead of four.

use std::path::{Path, PathBuf};

/// Home directory: `$HOME` on Unix, `%USERPROFILE%` on Windows. Filters out an
/// empty value (some CI environments export `HOME=""`).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Cairn's own state directory: `~/.cairn` (config.toml, spool.jsonl, logs/,
/// per-session file buffers).
pub fn cairn_home() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".cairn"))
}

/// Absolute path to the current cairn binary, with a "cairn" fallback. Used in
/// agent config files so the MCP server and hooks work regardless of PATH
/// resolution (especially on Windows).
pub fn cairn_exe() -> String {
    std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cairn".to_string())
}

/// Claude Code's global MCP config: `~/.claude.json`.
pub fn claude_global_config(home: &Path) -> PathBuf {
    home.join(".claude.json")
}

/// Claude Code's per-project MCP config: `<project>/.mcp.json`.
pub fn claude_project_mcp(project: &Path) -> PathBuf {
    project.join(".mcp.json")
}

/// Claude Code's lifecycle-hooks file. Always project-scoped regardless of
/// where the MCP server entry itself lives (Claude Code's hook system has no
/// user-level equivalent).
pub fn claude_settings(project: &Path) -> PathBuf {
    project.join(".claude").join("settings.json")
}

/// OpenCode's global config path. OpenCode follows XDG-ish directories on all
/// platforms: `~/.config/opencode/opencode.json` on Windows and Unix alike.
pub fn opencode_config_path() -> PathBuf {
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

/// OpenCode's generated plugin file, derived from the config path's parent so
/// it always follows the same `XDG_CONFIG_HOME` resolution as the config
/// itself. (Previously hardcoded to `~/.config/opencode/plugins/cairn.js` in
/// `reset.rs`, which silently failed to clean up when `XDG_CONFIG_HOME` was set
/// to something else — the plugin file and the config disagreed on where they
/// lived.)
pub fn opencode_plugin_path() -> PathBuf {
    let config_dir = opencode_config_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".opencode"));
    config_dir.join("plugins").join("cairn.js")
}

/// Codex CLI's user-level config path: `~/.codex/config.toml`. Codex follows
/// the same XDG-ish convention as OpenCode on every platform.
pub fn codex_config_path(home: Option<&Path>) -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(Path::to_path_buf))
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    config_home.join(".codex").join("config.toml")
}

/// Codex CLI's lifecycle-hooks file: `~/.codex/hooks.json`.
pub fn codex_hooks_path(home: &Path) -> PathBuf {
    home.join(".codex").join("hooks.json")
}

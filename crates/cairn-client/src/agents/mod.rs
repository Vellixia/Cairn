//! Per-agent installer registry. Each supported AI agent (Claude Code, Codex,
//! OpenCode) implements the [`Agent`] trait; `cairn setup`/`doctor`/`status`/
//! `reset` all go through the [`AGENTS`] registry instead of hand-rolling
//! per-agent path/detect/install/remove logic four times over. Adding a new
//! agent is one new file in this directory plus one line in [`AGENTS`].

mod claude_code;
mod codex;
mod opencode;

pub use claude_code::ClaudeCode;
pub use codex::Codex;
pub use opencode::OpenCode;

use anyhow::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Config-file scope for agent installation.
///
/// `Global` writes to the user-level config (e.g. `~/.claude.json`,
/// `~/.config/opencode/opencode.json`) so the same setup applies to every
/// project the user opens. `Project` writes to a per-project location (e.g.
/// `<cwd>/.mcp.json`) so the configuration only takes effect in the current
/// repo. Agents whose config is inherently user-level (Codex, OpenCode) ignore
/// this and always behave as `Global`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project,
}

/// Everything an `install()` call needs. Borrowing rather than owning keeps
/// this cheap to build per-agent in a loop (`setup`).
///
/// No `server`/`token` fields: since the v0.8.0 client redesign, agent config files never
/// embed credentials - they get a bare MCP entry referencing `cairn mcp`/`cairn hook`, which
/// read `~/.cairn/config.toml` themselves. The caller (`setup::run`) persists server/token to
/// that shared file directly; per-agent `install()` never needs to see them.
pub struct InstallCtx<'a> {
    pub project: &'a Path,
    pub home: Option<&'a Path>,
    pub scope: Scope,
}

/// One file an `install()` call touched, for the human-readable summary
/// printed after `cairn setup`.
pub struct InstalledFile {
    pub path: PathBuf,
    pub note: &'static str,
}

pub struct InstallReport {
    pub files: Vec<InstalledFile>,
    /// An extra line printed after the file list (e.g. Claude Code's
    /// "run /mcp to approve the server").
    pub hint: Option<&'static str>,
}

impl InstallReport {
    pub fn print(&self, agent_label: &str) {
        println!("\u{2713} Configured {agent_label}:");
        for f in &self.files {
            println!("  - {}  ({})", f.path.display(), f.note);
        }
        if let Some(h) = self.hint {
            println!("  - {h}");
        }
    }
}

/// One concrete cleanup step `cairn reset` can either print (`--dry-run`) or
/// execute. Building removal as data means dry-run and real execution can
/// never drift apart from each other — both walk the same `Vec<RemovalAction>`
/// and the dry-run path literally cannot do anything `apply()` wouldn't.
pub enum RemovalAction {
    /// Remove `inner_key` from the JSON object at `top_key` inside `file`.
    RemoveJsonKey {
        file: PathBuf,
        top_key: &'static str,
        inner_key: &'static str,
    },
    /// Strip every cairn-owned hook entry for `event` from `file`'s
    /// `hooks.<event>` array, preserving any other tool's entries for the same
    /// event.
    StripCairnHooks { file: PathBuf, event: &'static str },
    /// Remove the `[mcp_servers.cairn]` table from a Codex-shaped `config.toml`.
    RemoveCodexTable { file: PathBuf },
    /// Remove cairn entries from OpenCode's `plugin` array (legacy
    /// double-registration cleanup).
    StripOpenCodePluginArray { file: PathBuf },
    /// Delete a whole file outright (e.g. the generated OpenCode plugin).
    DeleteFile { file: PathBuf },
    /// Remove the `<!-- BEGIN CAIRN -->...<!-- END CAIRN -->` block from a
    /// shared instructions file (CLAUDE.md/AGENTS.md), preserving everything
    /// else in it.
    StripManagedBlock { file: PathBuf },
}

impl RemovalAction {
    pub fn target(&self) -> &Path {
        match self {
            RemovalAction::RemoveJsonKey { file, .. }
            | RemovalAction::StripCairnHooks { file, .. }
            | RemovalAction::RemoveCodexTable { file }
            | RemovalAction::StripOpenCodePluginArray { file }
            | RemovalAction::DeleteFile { file }
            | RemovalAction::StripManagedBlock { file } => file,
        }
    }

    fn what(&self) -> &'static str {
        match self {
            RemovalAction::RemoveJsonKey { .. } => "cairn entry",
            RemovalAction::StripCairnHooks { .. } => "Cairn hooks",
            RemovalAction::RemoveCodexTable { .. } => "cairn MCP table",
            RemovalAction::StripOpenCodePluginArray { .. } => "cairn plugin array entry",
            RemovalAction::DeleteFile { .. } => "file",
            RemovalAction::StripManagedBlock { .. } => "managed block",
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "Would remove {} from: {}",
            self.what(),
            self.target().display()
        )
    }

    fn done_message(&self) -> String {
        format!("Removed {} from: {}", self.what(), self.target().display())
    }

    /// What this action would do, computed once from the file's current
    /// content. `would_change()` and `apply()` both go through this so
    /// `--dry-run` can never report something different from what actually
    /// happens - there is exactly one place that decides "is there something
    /// here to remove."
    fn compute(&self) -> Result<Effect> {
        use crate::jsonedit::{
            read_object, remove_codex_cairn_block, remove_managed_block, strip_cairn_hooks,
            strip_cairn_plugin_entries,
        };
        match self {
            RemovalAction::RemoveJsonKey {
                file,
                top_key,
                inner_key,
            } => {
                let mut obj = read_object(file)?;
                let Some(inner) = obj.get_mut(*top_key).and_then(Value::as_object_mut) else {
                    return Ok(Effect::NoChange);
                };
                if inner.remove(*inner_key).is_none() {
                    return Ok(Effect::NoChange);
                }
                Ok(Effect::WriteJson {
                    file: file.clone(),
                    value: Value::Object(obj),
                })
            }
            RemovalAction::StripCairnHooks { file, event } => {
                let mut obj = read_object(file)?;
                let Some(hooks) = obj.get_mut("hooks").and_then(Value::as_object_mut) else {
                    return Ok(Effect::NoChange);
                };
                if !strip_cairn_hooks(hooks, event) {
                    return Ok(Effect::NoChange);
                }
                if hooks.is_empty() {
                    obj.remove("hooks");
                }
                Ok(Effect::WriteJson {
                    file: file.clone(),
                    value: Value::Object(obj),
                })
            }
            RemovalAction::RemoveCodexTable { file } => {
                if !file.exists() {
                    return Ok(Effect::NoChange);
                }
                let text = std::fs::read_to_string(file)?;
                let cleaned = remove_codex_cairn_block(&text);
                if cleaned == text {
                    return Ok(Effect::NoChange);
                }
                Ok(Effect::WriteText {
                    file: file.clone(),
                    text: cleaned,
                })
            }
            RemovalAction::StripOpenCodePluginArray { file } => {
                let mut obj = read_object(file)?;
                if !strip_cairn_plugin_entries(&mut obj) {
                    return Ok(Effect::NoChange);
                }
                Ok(Effect::WriteJson {
                    file: file.clone(),
                    value: Value::Object(obj),
                })
            }
            RemovalAction::DeleteFile { file } => {
                if !file.exists() {
                    return Ok(Effect::NoChange);
                }
                Ok(Effect::Delete { file: file.clone() })
            }
            RemovalAction::StripManagedBlock { file } => {
                if !file.exists() {
                    return Ok(Effect::NoChange);
                }
                let text = std::fs::read_to_string(file)?;
                match remove_managed_block(&text) {
                    Some(cleaned) if cleaned != text => Ok(Effect::WriteText {
                        file: file.clone(),
                        text: cleaned,
                    }),
                    _ => Ok(Effect::NoChange),
                }
            }
        }
    }

    /// Read-only probe for `--dry-run`: would `apply()` change anything?
    pub fn would_change(&self) -> Result<bool> {
        Ok(!matches!(self.compute()?, Effect::NoChange))
    }

    /// Execute this one action. Returns `Ok(true)` if it actually changed
    /// something on disk (used to track the total "N Cairn entries removed"
    /// count), `Ok(false)` if there was nothing to remove (e.g. running reset
    /// twice, or the file/agent was never configured).
    pub fn apply(&self) -> Result<bool> {
        match self.compute()? {
            Effect::NoChange => Ok(false),
            Effect::WriteJson { file, value } => {
                crate::jsonedit::write_json(&file, &value)?;
                println!("{}", self.done_message());
                Ok(true)
            }
            Effect::WriteText { file, text } => {
                std::fs::write(&file, text)?;
                println!("{}", self.done_message());
                Ok(true)
            }
            Effect::Delete { file } => {
                std::fs::remove_file(&file)?;
                println!("{}", self.done_message());
                Ok(true)
            }
        }
    }
}

/// The concrete write (or delete) a [`RemovalAction`] would make, decided once
/// by `compute()` and shared by both `would_change()` (dry-run) and `apply()`
/// (real execution).
enum Effect {
    WriteJson { file: PathBuf, value: Value },
    WriteText { file: PathBuf, text: String },
    Delete { file: PathBuf },
    NoChange,
}

pub trait Agent: Sync {
    /// Canonical lowercase id, e.g. `"claude-code"`. Also the string `cairn
    /// setup <id>` accepts.
    fn id(&self) -> &'static str;
    /// Human-readable name for print output, e.g. `"Claude Code"`.
    fn label(&self) -> &'static str;
    /// Extra names `cairn setup <name>` accepts (aliases), NOT including `id()`.
    fn aliases(&self) -> &'static [&'static str];
    fn detect(&self, project: &Path, home: Option<&Path>) -> bool;
    fn install(&self, ctx: &InstallCtx) -> Result<InstallReport>;
    fn removal_plan(&self, project: &Path, home: Option<&Path>) -> Vec<RemovalAction>;
    /// Non-fatal config-health issues `cairn doctor` should surface (e.g.
    /// duplicate hooks, double-registered plugins) — empty when nothing to flag.
    fn health(&self, project: &Path, home: Option<&Path>) -> Vec<String>;
}

pub static AGENTS: &[&dyn Agent] = &[&ClaudeCode, &Codex, &OpenCode];

/// All known ids, for error messages (`"unknown agent 'x'. Supported: ..."`).
pub fn ids() -> Vec<&'static str> {
    AGENTS.iter().map(|a| a.id()).collect()
}

/// Resolve a user-supplied agent name (id or alias, case-insensitive) to its
/// registry entry.
pub fn find(name: &str) -> Option<&'static dyn Agent> {
    let lower = name.to_ascii_lowercase();
    AGENTS
        .iter()
        .copied()
        .find(|a| a.id() == lower || a.aliases().contains(&lower.as_str()))
}

/// Every agent whose `detect()` finds a marker in `project` or `home`.
pub fn detect_all(project: &Path, home: Option<&Path>) -> Vec<&'static dyn Agent> {
    AGENTS
        .iter()
        .copied()
        .filter(|a| a.detect(project, home))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_resolves_aliases_and_rejects_unknown() {
        assert_eq!(find("claude").map(Agent::id), Some("claude-code"));
        assert_eq!(find("CC").map(Agent::id), Some("claude-code"));
        assert_eq!(find("CODEX").map(Agent::id), Some("codex"));
        assert_eq!(find("oc").map(Agent::id), Some("opencode"));
        assert!(find("emacs").is_none());
        assert!(find("cursor").is_none());
        assert!(find("claude-desktop").is_none());
    }

    #[test]
    fn registry_ids_match_known_agents() {
        assert_eq!(ids(), vec!["claude-code", "codex", "opencode"]);
    }

    #[test]
    fn detect_all_scopes_to_present_agents() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let (p, h) = (project.path(), home.path());
        let h_str = h.to_string_lossy().into_owned();

        // `opencode_config_path`/`codex_config_path` read `XDG_CONFIG_HOME`
        // unconditionally; pin it to `home` for this test's duration so it
        // can't race against any other test touching the same env var.
        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&h_str))], || {
            assert!(detect_all(p, Some(h)).is_empty());

            std::fs::create_dir_all(p.join(".claude")).unwrap();
            let found = detect_all(p, Some(h));
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].id(), "claude-code");

            std::fs::create_dir_all(h.join(".codex")).unwrap();
            std::fs::write(h.join(".codex/config.toml"), "").unwrap();
            let found = detect_all(p, Some(h));
            assert!(found.iter().any(|a| a.id() == "codex"));

            // XDG_CONFIG_HOME is the config root itself; opencode.json lives
            // directly under it.
            std::fs::create_dir_all(h.join("opencode")).unwrap();
            std::fs::write(h.join("opencode/opencode.json"), "{}").unwrap();
            let found = detect_all(p, Some(h));
            assert!(found.iter().any(|a| a.id() == "opencode"));
        });
    }
}

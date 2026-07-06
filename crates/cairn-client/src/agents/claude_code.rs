//! Claude Code: project `.mcp.json` / global `~/.claude.json` (the `cairn` MCP
//! server) plus project-scoped `.claude/settings.json` lifecycle hooks.

use super::{Agent, InstallCtx, InstallReport, InstalledFile, RemovalAction, Scope};
use crate::jsonedit::{add_hook, read_object, write_json};
use crate::paths;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::path::Path;

pub struct ClaudeCode;

impl Agent for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["claude", "claudecode", "cc"]
    }

    fn detect(&self, project: &Path, home: Option<&Path>) -> bool {
        let home_has = |rel: &str| home.is_some_and(|h| h.join(rel).exists());
        project.join(".claude").exists()
            || project.join(".mcp.json").exists()
            || home_has(".claude")
            || home_has(".claude.json")
    }

    fn install(&self, ctx: &InstallCtx) -> Result<InstallReport> {
        let mcp_path = match ctx.scope {
            Scope::Global => {
                let home = ctx.home.ok_or_else(|| {
                    anyhow!("global scope requires a home directory (set $HOME or $USERPROFILE)")
                })?;
                paths::claude_global_config(home)
            }
            Scope::Project => paths::claude_project_mcp(ctx.project),
        };
        merge_mcp_server(&mcp_path, "mcpServers")?;

        // v0.8.0 Sprint 10 (B6, layer 2): the skill carries the full playbook and loads
        // on-demand, at the same scope as the MCP entry above - a project-scoped MCP
        // connection gets a project-scoped skill, a global one gets a global skill.
        let skill_path = skill_path(ctx.project, ctx.home, ctx.scope)?;
        if let Some(parent) = skill_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&skill_path, cairn_mcp::guidance::skill_md())
            .with_context(|| format!("writing {}", skill_path.display()))?;

        let settings_path = paths::claude_settings(ctx.project);
        let mut settings = read_object(&settings_path)?;
        {
            let hooks = settings
                .entry("hooks")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context("settings.json: hooks is not an object")?;
            let exe = paths::cairn_exe();
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
                // Claude Code's edit-family tool names.
                Some("Edit|Write|MultiEdit|NotebookEdit"),
            );
            add_hook(hooks, "SessionEnd", &format!("{exe} hook SessionEnd"), None);
            // v0.8.0 client redesign: fires BEFORE Claude Code destroys/summarizes its
            // context window, so a session's state gets flushed even if it never reaches
            // a clean SessionEnd - see `hook::RemoteClient::flush_session_state`.
            add_hook(hooks, "PreCompact", &format!("{exe} hook PreCompact"), None);
            // v0.8.0 Sprint 10 (C-2): opt-in real-time guard. Registered unconditionally like
            // every other hook - the hook itself checks `hooks.guard` at runtime and is a
            // silent no-op (implicit allow) when it's off, so toggling the setting later never
            // requires re-running setup. Claude-Code-only: Codex/OpenCode hooks don't support
            // returning a permission decision today. Edit/Write/MultiEdit/NotebookEdit get
            // verified against the guard; Bash gets sanitized for secrets before it runs.
            add_hook(
                hooks,
                "PreToolUse",
                &format!("{exe} hook PreToolUse"),
                Some("Edit|Write|MultiEdit|NotebookEdit|Bash"),
            );
        }
        write_json(&settings_path, &Value::Object(settings))?;

        Ok(InstallReport {
            files: vec![
                InstalledFile {
                    path: mcp_path,
                    note: "MCP server: cairn",
                },
                InstalledFile {
                    path: skill_path,
                    note: "skill: cairn playbook (loads on-demand)",
                },
                InstalledFile {
                    path: settings_path,
                    note:
                        "hooks: SessionStart, UserPromptSubmit, PostToolUse, SessionEnd, PreCompact",
                },
            ],
            hint: Some("Run /mcp in Claude Code to approve the cairn server"),
        })
    }

    fn removal_plan(&self, project: &Path, home: Option<&Path>) -> Vec<RemovalAction> {
        let mut actions = vec![
            RemovalAction::RemoveJsonKey {
                file: paths::claude_project_mcp(project),
                top_key: "mcpServers",
                inner_key: "cairn",
            },
            RemovalAction::DeleteFile {
                file: skill_dir(project).join("SKILL.md"),
            },
        ];
        if let Some(h) = home {
            actions.push(RemovalAction::RemoveJsonKey {
                file: paths::claude_global_config(h),
                top_key: "mcpServers",
                inner_key: "cairn",
            });
            actions.push(RemovalAction::DeleteFile {
                file: skill_dir(h).join("SKILL.md"),
            });
        }
        let settings = paths::claude_settings(project);
        for event in [
            "SessionStart",
            "UserPromptSubmit",
            "PostToolUse",
            "SessionEnd",
            "PreCompact",
            "PreToolUse",
        ] {
            actions.push(RemovalAction::StripCairnHooks {
                file: settings.clone(),
                event,
            });
        }
        actions
    }

    /// Flags a skill file written by an older binary (rev marker doesn't match the guidance
    /// module's current [`cairn_mcp::guidance::GUIDANCE_REV`]) so `doctor --fix` re-runs
    /// `install()` to refresh it.
    fn health(&self, project: &Path, home: Option<&Path>) -> Vec<String> {
        let want = format!("cairn-guidance-rev: {}", cairn_mcp::guidance::GUIDANCE_REV);
        let candidates: Vec<std::path::PathBuf> = [Some(skill_dir(project)), home.map(skill_dir)]
            .into_iter()
            .flatten()
            .map(|dir| dir.join("SKILL.md"))
            .collect();

        let mut issues: Vec<String> = candidates
            .iter()
            .filter(|f| f.exists())
            .filter(|f| {
                !std::fs::read_to_string(f)
                    .unwrap_or_default()
                    .contains(&want)
            })
            .map(|f| format!("{} has a stale guidance revision", f.display()))
            .collect();

        // Only flag a MISSING skill when Claude Code is actually in use - otherwise every
        // `cairn doctor` run would complain about a skill nobody asked for.
        if issues.is_empty() && self.detect(project, home) && !candidates.iter().any(|f| f.exists())
        {
            issues.push(
                "no Cairn skill installed (run `cairn setup claude-code` to add the playbook)"
                    .to_string(),
            );
        }
        issues
    }
}

/// `.claude/skills/cairn/` under `root` (either the project root or the home directory,
/// depending on scope - both use the same `.claude/skills/cairn/` layout).
fn skill_dir(root: &Path) -> std::path::PathBuf {
    root.join(".claude").join("skills").join("cairn")
}

fn skill_path(project: &Path, home: Option<&Path>, scope: Scope) -> Result<std::path::PathBuf> {
    match scope {
        Scope::Project => Ok(skill_dir(project).join("SKILL.md")),
        Scope::Global => {
            let home = home.ok_or_else(|| {
                anyhow!("global scope requires a home directory (set $HOME or $USERPROFILE)")
            })?;
            Ok(skill_dir(home).join("SKILL.md"))
        }
    }
}

fn merge_mcp_server(path: &Path, schema_key: &str) -> Result<()> {
    let mut obj = read_object(path)?;
    let servers = obj
        .entry(schema_key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{}: '{schema_key}' is not an object", path.display()))?;

    // v0.8.0 client redesign: server/token live only in `~/.cairn/config.toml` (written by
    // the caller), never in the agent's own config - so this is always a bare entry. Any
    // `env` block a pre-config-file `setup` run left behind is dropped (de-tokenized) rather
    // than carried forward.
    servers.insert("cairn".into(), cairn_server());
    write_json(path, &Value::Object(obj))
}

fn cairn_server() -> Value {
    let exe = paths::cairn_exe();
    json!({ "command": exe, "args": ["mcp"] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::InstallCtx;

    fn read_text(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn setup_is_idempotent_and_non_destructive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"model":"opus","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
        )
        .unwrap();

        let ctx = InstallCtx {
            project: dir.path(),
            home: None,
            scope: Scope::Project,
        };
        ClaudeCode.install(&ctx).unwrap();
        ClaudeCode.install(&ctx).unwrap();

        let settings: Value =
            serde_json::from_str(&read_text(&dir.path().join(".claude/settings.json"))).unwrap();
        assert_eq!(settings["model"], "opus");
        let exe = paths::cairn_exe();
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
        // v0.8.0 Sprint 10 (C-2): registered unconditionally (the config gate is checked at
        // hook-run time, not at setup time) with a matcher covering both edit tools and Bash.
        let pre_tool_use = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(
            pre_tool_use
                .iter()
                .filter(|g| g["hooks"][0]["command"] == format!("{exe} hook PreToolUse"))
                .count(),
            1
        );
        assert!(pre_tool_use
            .iter()
            .any(|g| g["matcher"] == "Edit|Write|MultiEdit|NotebookEdit|Bash"));

        let mcp: Value = serde_json::from_str(&read_text(&dir.path().join(".mcp.json"))).unwrap();
        assert_eq!(mcp["mcpServers"]["cairn"]["command"], exe);
    }

    #[test]
    fn global_scope_writes_to_home_dot_claude_json() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let exe = paths::cairn_exe();

        let ctx = InstallCtx {
            project: project.path(),
            home: Some(home.path()),
            scope: Scope::Global,
        };
        ClaudeCode.install(&ctx).unwrap();

        let global_path = home.path().join(".claude.json");
        let v: Value = serde_json::from_str(&read_text(&global_path)).unwrap();
        assert_eq!(v["mcpServers"]["cairn"]["command"], exe);
        assert!(!project.path().join(".mcp.json").exists());
    }

    #[test]
    fn project_scope_writes_to_dot_mcp_json() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let exe = paths::cairn_exe();

        let ctx = InstallCtx {
            project: project.path(),
            home: Some(home.path()),
            scope: Scope::Project,
        };
        ClaudeCode.install(&ctx).unwrap();

        let v: Value = serde_json::from_str(&read_text(&project.path().join(".mcp.json"))).unwrap();
        assert_eq!(v["mcpServers"]["cairn"]["command"], exe);
        assert!(!home.path().join(".claude.json").exists());
    }

    #[test]
    fn install_de_tokenizes_an_env_block_left_by_an_older_binary() {
        let dir = tempfile::tempdir().unwrap();
        // Simulate a pre-v0.8.0 binary's `.mcp.json`, which used to embed
        // server/token directly into the agent's own config file.
        std::fs::write(
            dir.path().join(".mcp.json"),
            json!({
                "mcpServers": {
                    "cairn": {
                        "command": "cairn",
                        "args": ["mcp"],
                        "env": { "CAIRN_TOKEN": "old-token" },
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let ctx = InstallCtx {
            project: dir.path(),
            home: None,
            scope: Scope::Project,
        };
        ClaudeCode.install(&ctx).unwrap();
        let after: Value = serde_json::from_str(&read_text(&dir.path().join(".mcp.json"))).unwrap();
        assert!(
            after["mcpServers"]["cairn"].get("env").is_none(),
            "install must not write (or keep) an env block"
        );
    }

    #[test]
    fn removal_plan_strips_hooks_but_preserves_foreign_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[
                {"hooks":[{"type":"command","command":"echo foreign-tool"}]},
                {"hooks":[{"type":"command","command":"cairn hook SessionStart"}]}
            ]}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"cairn":{"command":"cairn"},"other":{"command":"foo"}}}"#,
        )
        .unwrap();

        for action in ClaudeCode.removal_plan(dir.path(), None) {
            action.apply().unwrap();
        }

        let settings: Value =
            serde_json::from_str(&read_text(&dir.path().join(".claude/settings.json"))).unwrap();
        let remaining = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(remaining.len(), 1, "the foreign hook must survive reset");
        assert_eq!(remaining[0]["hooks"][0]["command"], "echo foreign-tool");

        let mcp: Value = serde_json::from_str(&read_text(&dir.path().join(".mcp.json"))).unwrap();
        assert!(mcp["mcpServers"].get("cairn").is_none());
        assert_eq!(mcp["mcpServers"]["other"]["command"], "foo");
    }

    #[test]
    fn health_flags_a_missing_skill_only_when_claude_code_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            ClaudeCode.health(dir.path(), None).is_empty(),
            "no .claude marker at all - not in use, nothing to flag"
        );

        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        let issues = ClaudeCode.health(dir.path(), None);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("no Cairn skill installed"));
    }

    #[test]
    fn install_writes_a_skill_file_at_the_same_scope_as_the_mcp_entry() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();

        let project_ctx = InstallCtx {
            project: project.path(),
            home: Some(home.path()),
            scope: Scope::Project,
        };
        ClaudeCode.install(&project_ctx).unwrap();
        let project_skill = project.path().join(".claude/skills/cairn/SKILL.md");
        assert!(project_skill.exists());
        assert!(!home.path().join(".claude/skills/cairn/SKILL.md").exists());
        let content = read_text(&project_skill);
        assert!(content.starts_with("---\n"));
        assert!(content.contains("name: cairn"));
        assert!(content.contains("Using Cairn"));

        let global_ctx = InstallCtx {
            scope: Scope::Global,
            ..project_ctx
        };
        ClaudeCode.install(&global_ctx).unwrap();
        assert!(home.path().join(".claude/skills/cairn/SKILL.md").exists());
    }

    #[test]
    fn health_flags_a_skill_file_with_a_stale_guidance_revision() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            ClaudeCode.health(dir.path(), None).is_empty(),
            "no skill file yet - nothing to flag"
        );

        let skill_dir = dir.path().join(".claude/skills/cairn");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: cairn\n---\n<!-- cairn-guidance-rev: 0 -->\n\nstale content",
        )
        .unwrap();
        let issues = ClaudeCode.health(dir.path(), None);
        assert_eq!(
            issues.len(),
            1,
            "rev 0 must never match the real current rev"
        );
        assert!(issues[0].contains("stale"));

        // install() writes the current rev - health should go clean afterward.
        let ctx = InstallCtx {
            project: dir.path(),
            home: None,
            scope: Scope::Project,
        };
        ClaudeCode.install(&ctx).unwrap();
        assert!(ClaudeCode.health(dir.path(), None).is_empty());
    }

    #[test]
    fn removal_plan_deletes_the_skill_file() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let ctx = InstallCtx {
            project: project.path(),
            home: Some(home.path()),
            scope: Scope::Project,
        };
        ClaudeCode.install(&ctx).unwrap();
        assert!(project
            .path()
            .join(".claude/skills/cairn/SKILL.md")
            .exists());

        for action in ClaudeCode.removal_plan(project.path(), Some(home.path())) {
            action.apply().unwrap();
        }
        assert!(!project
            .path()
            .join(".claude/skills/cairn/SKILL.md")
            .exists());
    }
}

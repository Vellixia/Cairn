//! `cairn reset` - remove Cairn-managed entries from all agent config files.
//!
//! Per-agent cleanup is delegated to `agents::AGENTS[*].removal_plan()`, plus two
//! cross-agent steps (CLAUDE.md/AGENTS.md managed blocks, written by `cairn rules`
//! for whichever agent was set up) that aren't owned by any single agent.
//!
//! `--dry-run` and real execution walk the exact same `Vec<RemovalAction>` and both
//! go through `RemovalAction::compute()` - the only difference is whether the
//! computed effect gets written. Dry-run can never report something that doesn't
//! actually happen.

use crate::agents::{self, RemovalAction};
use crate::paths;
use anyhow::Result;

pub fn run(dry_run: bool) -> Result<()> {
    let project = std::env::current_dir()?;
    let home = paths::home_dir();

    let mut actions = vec![
        RemovalAction::StripManagedBlock {
            file: project.join("CLAUDE.md"),
        },
        RemovalAction::StripManagedBlock {
            file: project.join("AGENTS.md"),
        },
    ];
    for a in agents::AGENTS {
        actions.extend(a.removal_plan(&project, home.as_deref()));
    }

    // Each action is independent - a corrupt or unreadable file (hand-edited,
    // truncated write) must not abort cleanup of every *other* file. Report and
    // skip instead of propagating, so `cairn reset` stays best-effort exactly
    // like the settings it's trying to clean up.
    let mut removed = 0usize;
    for action in &actions {
        let outcome = if dry_run {
            action.would_change()
        } else {
            action.apply()
        };
        match outcome {
            Ok(true) => {
                if dry_run {
                    println!("{}", action.describe());
                }
                removed += 1;
            }
            Ok(false) => {}
            Err(e) => eprintln!("cairn reset: skipping {}: {e}", action.target().display()),
        }
    }

    if removed == 0 {
        println!("No Cairn-managed entries found.");
    } else if dry_run {
        println!("\nRun `cairn reset` without --dry-run to apply.");
    } else {
        println!("\nRemoved {removed} Cairn entries.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{Agent, ClaudeCode, Codex, InstallCtx, Scope};
    use serde_json::Value;
    use std::fs;

    /// Build the same set of actions `run()` would, against an explicit
    /// project/home pair - lets tests exercise the exact dry-run/apply
    /// symmetry `run()` relies on without touching the real cwd/home.
    ///
    /// Codex and OpenCode both resolve their config paths from
    /// `XDG_CONFIG_HOME` in *preference to* the `home` argument (see
    /// `paths::codex_config_path`, `paths::opencode_config_path`) - so a
    /// naive call here would either disagree with a test's own `home`-scoped
    /// `install()` calls (if some ambient `XDG_CONFIG_HOME` were set) or,
    /// worse, fall through to the *real* machine's OpenCode config when
    /// `home` is `None`. Pin `XDG_CONFIG_HOME` to `home` itself when one is
    /// given (so every agent agrees on where "home" is, matching how the
    /// test's own `install()` calls resolved paths), or to a throwaway
    /// sandbox otherwise. The returned actions carry already-resolved
    /// `PathBuf`s, so this is the only place that needs the pin.
    fn plan(project: &std::path::Path, home: Option<&std::path::Path>) -> Vec<RemovalAction> {
        let mut actions = vec![
            RemovalAction::StripManagedBlock {
                file: project.join("CLAUDE.md"),
            },
            RemovalAction::StripManagedBlock {
                file: project.join("AGENTS.md"),
            },
        ];
        let _sandbox_guard;
        let xdg_pin = match home {
            Some(h) => h.to_string_lossy().into_owned(),
            None => {
                _sandbox_guard = tempfile::tempdir().unwrap();
                _sandbox_guard.path().to_string_lossy().into_owned()
            }
        };
        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&xdg_pin))], || {
            for a in agents::AGENTS {
                actions.extend(a.removal_plan(project, home));
            }
        });
        actions
    }

    fn read(path: &std::path::Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn dry_run_reports_without_writing_anything() {
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        fs::write(
            p.join("CLAUDE.md"),
            "# rules\n\n<!-- BEGIN CAIRN (managed by `cairn rules`) -->\nstuff\n<!-- END CAIRN -->\n",
        )
        .unwrap();
        fs::write(
            p.join(".mcp.json"),
            r#"{"mcpServers":{"cairn":{"command":"cairn"}}}"#,
        )
        .unwrap();
        let before_claude = read(&p.join("CLAUDE.md"));
        let before_mcp = read(&p.join(".mcp.json"));

        let actions = plan(p, None);
        let mut reported = 0usize;
        for action in &actions {
            if action.would_change().unwrap() {
                reported += 1;
            }
        }
        assert!(
            reported >= 2,
            "should find the CLAUDE.md block and the mcp.json entry"
        );

        // Nothing on disk moved.
        assert_eq!(read(&p.join("CLAUDE.md")), before_claude);
        assert_eq!(read(&p.join(".mcp.json")), before_mcp);
    }

    #[test]
    fn dry_run_count_matches_real_run_count() {
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        fs::create_dir_all(p.join(".claude")).unwrap();
        fs::write(
            p.join("CLAUDE.md"),
            "<!-- BEGIN CAIRN (managed by `cairn rules`) -->\nstuff\n<!-- END CAIRN -->\n",
        )
        .unwrap();
        let ctx = InstallCtx {
            project: p,
            home: None,
            scope: Scope::Project,
            server: None,
            token: None,
            embed_env: false,
        };
        ClaudeCode.install(&ctx).unwrap();

        let dry_count = plan(p, None)
            .iter()
            .filter(|a| a.would_change().unwrap())
            .count();
        let real_count = plan(p, None).iter().filter(|a| a.apply().unwrap()).count();
        assert_eq!(dry_count, real_count);
        assert!(real_count > 0);

        // Running again finds nothing left.
        let second_pass = plan(p, None)
            .iter()
            .filter(|a| a.would_change().unwrap())
            .count();
        assert_eq!(second_pass, 0);
    }

    #[test]
    fn foreign_hooks_and_other_mcp_servers_survive_a_full_reset() {
        let project = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let p = project.path();
        let h = home.path();

        // Claude Code: cairn + a foreign hook + a foreign MCP server.
        fs::create_dir_all(p.join(".claude")).unwrap();
        let ctx = InstallCtx {
            project: p,
            home: Some(h),
            scope: Scope::Project,
            server: None,
            token: None,
            embed_env: false,
        };
        ClaudeCode.install(&ctx).unwrap();
        {
            let mut settings: Value =
                serde_json::from_str(&read(&p.join(".claude/settings.json"))).unwrap();
            settings["hooks"]["SessionStart"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({ "hooks": [{ "type": "command", "command": "echo foreign" }] }));
            fs::write(
                p.join(".claude/settings.json"),
                serde_json::to_string_pretty(&settings).unwrap(),
            )
            .unwrap();

            let mut mcp: Value = serde_json::from_str(&read(&p.join(".mcp.json"))).unwrap();
            mcp["mcpServers"]["other"] = serde_json::json!({ "command": "foo" });
            fs::write(
                p.join(".mcp.json"),
                serde_json::to_string_pretty(&mcp).unwrap(),
            )
            .unwrap();
        }

        // Codex: cairn + a foreign hook + a foreign MCP server. `codex_config_path`
        // reads `XDG_CONFIG_HOME` in preference to `ctx.home`; pin it to `h` for
        // this one call so it resolves the same way `plan()` will later, and so
        // this can't race against another test's concurrent `with_env` mutation
        // of the same env var.
        let h_str = h.to_string_lossy().into_owned();
        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&h_str))], || {
            Codex.install(&ctx).unwrap();
        });
        {
            let codex_toml = h.join(".codex/config.toml");
            let mut text = read(&codex_toml);
            text.push_str("\n[mcp_servers.other]\ncommand = \"foo\"\n");
            fs::write(&codex_toml, text).unwrap();

            let hooks_path = h.join(".codex/hooks.json");
            let mut hooks: Value = serde_json::from_str(&read(&hooks_path)).unwrap();
            hooks["hooks"]["SessionStart"].as_array_mut().unwrap().push(
                serde_json::json!({ "hooks": [{ "type": "command", "command": "echo foreign" }] }),
            );
            fs::write(&hooks_path, serde_json::to_string_pretty(&hooks).unwrap()).unwrap();
        }

        for action in plan(p, Some(h)) {
            action.apply().unwrap();
        }

        let settings: Value =
            serde_json::from_str(&read(&p.join(".claude/settings.json"))).unwrap();
        let starts = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 1, "foreign Claude Code hook must survive");
        assert_eq!(starts[0]["hooks"][0]["command"], "echo foreign");

        let mcp: Value = serde_json::from_str(&read(&p.join(".mcp.json"))).unwrap();
        assert!(mcp["mcpServers"].get("cairn").is_none());
        assert_eq!(mcp["mcpServers"]["other"]["command"], "foo");

        let codex_toml = read(&h.join(".codex/config.toml"));
        assert!(!codex_toml.contains("mcp_servers.cairn"));
        assert!(codex_toml.contains("[mcp_servers.other]"));

        let hooks: Value = serde_json::from_str(&read(&h.join(".codex/hooks.json"))).unwrap();
        let codex_starts = hooks["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(codex_starts.len(), 1, "foreign Codex hook must survive");
        assert_eq!(codex_starts[0]["hooks"][0]["command"], "echo foreign");
    }

    #[test]
    fn opencode_plugin_path_honors_xdg_config_home() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();

        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&home_str))], || {
            let ctx = InstallCtx {
                project: home.path(),
                home: Some(home.path()),
                scope: Scope::Global,
                server: None,
                token: None,
                embed_env: false,
            };
            agents::OpenCode.install(&ctx).unwrap();
            let plugin_path = paths::opencode_plugin_path();
            assert!(
                plugin_path.starts_with(home.path()),
                "plugin path must resolve under XDG_CONFIG_HOME, not a hardcoded ~/.config"
            );
            assert!(plugin_path.exists());

            for action in agents::OpenCode.removal_plan(home.path(), Some(home.path())) {
                action.apply().unwrap();
            }
            assert!(
                !plugin_path.exists(),
                "reset must delete the XDG-resolved plugin file"
            );
        });
    }

    #[test]
    fn corrupt_config_file_is_skipped_without_blocking_cleanup_of_others() {
        let project = tempfile::tempdir().unwrap();
        let p = project.path();
        fs::create_dir_all(p.join(".claude")).unwrap();
        let ctx = InstallCtx {
            project: p,
            home: None,
            scope: Scope::Project,
            server: None,
            token: None,
            embed_env: false,
        };
        // Install validly first (so settings.json has a real cairn hook to
        // clean up), THEN corrupt .mcp.json (hand-edited, truncated write) -
        // corrupting it before install would just make install() itself fail.
        ClaudeCode.install(&ctx).unwrap();
        fs::write(p.join(".mcp.json"), "{ not json").unwrap();

        // Mirror `run()`'s per-action error handling: never propagate, never panic.
        let mut removed = 0usize;
        for action in plan(p, None) {
            match action.apply() {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(_) => {}
            }
        }

        assert!(
            removed > 0,
            "the valid settings.json hooks must still be cleaned"
        );
        let settings: Value =
            serde_json::from_str(&read(&p.join(".claude/settings.json"))).unwrap();
        assert!(settings["hooks"].get("SessionStart").is_none());
        // The corrupt file is left untouched rather than being overwritten with
        // a guess at its contents.
        assert_eq!(read(&p.join(".mcp.json")), "{ not json");
    }
}

//! Codex CLI: `~/.codex/config.toml` under `[mcp_servers.cairn]` (TOML via
//! `toml_edit`, dependency-free hand-rolled merge would have mishandled inline
//! comments and quoted specials) plus `~/.codex/hooks.json` lifecycle hooks.

use super::{Agent, InstallCtx, InstallReport, InstalledFile, RemovalAction};
use crate::jsonedit::{add_hook, read_object, write_json};
use crate::paths;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Codex;

impl Agent for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn label(&self) -> &'static str {
        "Codex CLI"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }

    fn detect(&self, project: &Path, home: Option<&Path>) -> bool {
        let home_has = |rel: &str| home.is_some_and(|h| h.join(rel).exists());
        paths::codex_config_path(home).exists()
            || project.join(".codex").join("config.toml").exists()
            || home_has(".codex/config.toml")
    }

    fn install(&self, ctx: &InstallCtx) -> Result<InstallReport> {
        let path = paths::codex_config_path(ctx.home);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let original = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
        } else {
            String::new()
        };
        let merged =
            upsert_codex_cairn(&original).with_context(|| format!("editing {}", path.display()))?;
        fs::write(&path, merged).with_context(|| format!("writing {}", path.display()))?;

        let mut files = vec![InstalledFile {
            path: path.clone(),
            note: "MCP server: cairn",
        }];
        if let Some(h) = ctx.home {
            let hooks_path = write_codex_hooks(h)?;
            files.push(InstalledFile {
                path: hooks_path,
                note: "hooks: SessionStart, UserPromptSubmit, PostToolUse, Stop",
            });
        }
        Ok(InstallReport { files, hint: None })
    }

    fn removal_plan(&self, _project: &Path, home: Option<&Path>) -> Vec<RemovalAction> {
        let Some(h) = home else {
            return Vec::new();
        };
        let mut actions = vec![RemovalAction::RemoveCodexTable {
            file: paths::codex_config_path(Some(h)),
        }];
        let hooks_path = paths::codex_hooks_path(h);
        for event in ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"] {
            actions.push(RemovalAction::StripCairnHooks {
                file: hooks_path.clone(),
                event,
            });
        }
        actions
    }

    fn health(&self, _project: &Path, home: Option<&Path>) -> Vec<String> {
        let mut issues = Vec::new();
        let Some(h) = home else {
            return issues;
        };
        let hooks_path = paths::codex_hooks_path(h);
        let Ok(text) = fs::read_to_string(&hooks_path) else {
            return issues;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            return issues;
        };
        let Some(obj) = v.get("hooks").and_then(|o| o.as_object()) else {
            return issues;
        };
        for (event, arr) in obj {
            let Some(arr) = arr.as_array() else { continue };
            let cairn = arr
                .iter()
                .filter(|g| {
                    g.get("hooks")
                        .and_then(|hs| hs.as_array())
                        .map(|hs| {
                            hs.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .map(|c| {
                                        let lower = c.to_ascii_lowercase();
                                        lower.contains("cairn") && lower.contains("hook")
                                    })
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                })
                .count();
            if cairn > 1 {
                issues.push(format!(
                    "{event}: {cairn} cairn hooks (dedup with `cairn setup codex`)"
                ));
            }
        }
        issues
    }
}

/// Write Codex lifecycle hooks to `~/.codex/hooks.json`, idempotently merging
/// with any existing hooks from other tools (e.g. lean-ctx).
fn write_codex_hooks(home: &Path) -> Result<PathBuf> {
    let hooks_path = paths::codex_hooks_path(home);
    let mut hooks_cfg = read_object(&hooks_path)?;
    let hooks_obj = hooks_cfg
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("{}: 'hooks' is not an object", hooks_path.display()))?;

    let exe = paths::cairn_exe();
    add_hook(
        hooks_obj,
        "SessionStart",
        &format!("{exe} hook SessionStart"),
        Some("startup|resume|clear|compact"),
    );
    add_hook(
        hooks_obj,
        "UserPromptSubmit",
        &format!("{exe} hook UserPromptSubmit"),
        None,
    );
    add_hook(
        hooks_obj,
        "PostToolUse",
        &format!("{exe} hook PostToolUse"),
        Some("apply_patch|Edit|Write"),
    );
    add_hook(hooks_obj, "Stop", &format!("{exe} hook SessionEnd"), None);

    write_json(&hooks_path, &Value::Object(hooks_cfg))?;
    Ok(hooks_path)
}

/// Upsert the `[mcp_servers.cairn]` table into a Codex `config.toml`,
/// preserving every other table, comment, and whitespace via `toml_edit`.
/// Idempotent: running twice yields byte-identical output.
fn upsert_codex_cairn(original: &str) -> Result<String> {
    use toml_edit::{value, Array, DocumentMut, Item, Table};

    let mut doc = original
        .parse::<DocumentMut>()
        .context("Codex config.toml is not valid TOML; refusing to overwrite it")?;

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

    cairn["command"] = value(paths::cairn_exe());
    let mut args = Array::new();
    args.push("mcp");
    cairn["args"] = value(args);

    // v0.8.0 client redesign: server/token live only in `~/.cairn/config.toml` - actively
    // drop any env block a previous (pre-config-file) `setup` run embedded here, rather
    // than preserving it.
    cairn.remove("env");

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_text(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn upsert_writes_cairn_table() {
        let exe = paths::cairn_exe();
        let out = upsert_codex_cairn("").unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some(exe.as_str())
        );
        assert_eq!(doc["mcp_servers"]["cairn"]["args"][0].as_str(), Some("mcp"));
        assert!(
            doc["mcp_servers"]["cairn"].get("env").is_none(),
            "server/token live only in ~/.cairn/config.toml, never embedded here"
        );
    }

    #[test]
    fn upsert_replaces_stale_cairn_but_keeps_other_servers() {
        let exe = paths::cairn_exe();
        let original = "# head\n[mcp_servers.cairn]\ncommand = \"stale\"\nargs = [\"old\"]\n\n[mcp_servers.other]\ncommand = \"foo\"\n";
        let out = upsert_codex_cairn(original).unwrap();
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some(exe.as_str())
        );
        assert_ne!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some("stale")
        );
        assert_eq!(doc["mcp_servers"]["other"]["command"].as_str(), Some("foo"));
    }

    #[test]
    fn upsert_preserves_comments_other_tables_and_quoted_specials() {
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
        let out = upsert_codex_cairn(original).unwrap();
        assert!(out.contains("# user preferences — keep me!"));
        assert!(out.contains("model = \"opus\"  # inline comment"));
        assert!(out.contains("[tui]"));
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["other"]["env"]["WEIRD"].as_str(),
            Some("a=b#c")
        );
    }

    #[test]
    fn upsert_is_idempotent() {
        let first = upsert_codex_cairn("").unwrap();
        let second = upsert_codex_cairn(&first).unwrap();
        assert_eq!(first, second, "re-running upsert must be byte-identical");
    }

    #[test]
    fn upsert_de_tokenizes_an_env_block_left_by_an_older_binary() {
        // Simulate a pre-v0.8.0 binary's config.toml, which used to embed
        // server/token directly into `[mcp_servers.cairn.env]`.
        let embedded = "[mcp_servers.cairn]\ncommand = \"cairn\"\nargs = [\"mcp\"]\n\n[mcp_servers.cairn.env]\nCAIRN_TOKEN = \"old-tok\"\n";
        let bare = upsert_codex_cairn(embedded).unwrap();
        let doc = bare.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(
            doc["mcp_servers"]["cairn"].get("env").is_none(),
            "upsert must drop a previously-embedded env block, not merge into it"
        );
    }

    /// Test-only entry point that takes an explicit config path, skipping
    /// `codex_config_path`'s `XDG_CONFIG_HOME` lookup so tests don't race on
    /// the env var when run in parallel.
    fn install_at(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let original = if path.exists() {
            fs::read_to_string(path)?
        } else {
            String::new()
        };
        let merged = upsert_codex_cairn(&original)?;
        fs::write(path, merged)?;
        Ok(())
    }

    #[test]
    fn setup_writes_to_xdg_path_and_preserves_existing_keys() {
        let exe = paths::cairn_exe();
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        fs::write(&cfg, "# user prefs\ntui = { theme = \"dark\" }\n").unwrap();

        install_at(&cfg).unwrap();

        let out = read_text(&cfg);
        assert!(out.contains("# user prefs"));
        assert!(out.contains("tui = { theme = \"dark\" }"));
        let doc = out.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["cairn"]["command"].as_str(),
            Some(exe.as_str())
        );
        assert!(doc["mcp_servers"]["cairn"].get("env").is_none());
    }

    #[test]
    fn setup_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("config.toml");

        install_at(&cfg).unwrap();
        let first = read_text(&cfg);
        install_at(&cfg).unwrap();
        let second = read_text(&cfg);
        assert_eq!(first, second, "running setup twice must be idempotent");
    }

    #[test]
    fn removal_plan_removes_table_and_preserves_foreign_hooks() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        fs::create_dir_all(&codex_dir).unwrap();
        fs::write(
            codex_dir.join("config.toml"),
            "[mcp_servers.cairn]\ncommand = \"cairn\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n",
        )
        .unwrap();
        fs::write(
            codex_dir.join("hooks.json"),
            r#"{"hooks":{"SessionStart":[
                {"hooks":[{"type":"command","command":"echo foreign-tool"}]},
                {"matcher":"startup","hooks":[{"type":"command","command":"cairn hook SessionStart"}]}
            ]}}"#,
        )
        .unwrap();

        // `codex_config_path`/`codex_hooks_path` prefer `XDG_CONFIG_HOME` over the
        // `home` argument; pin it unset so this test deterministically targets
        // `dir.path()` regardless of what other tests touching the same env var
        // are doing concurrently.
        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", None)], || {
            for action in Codex.removal_plan(dir.path(), Some(dir.path())) {
                action.apply().unwrap();
            }
        });

        let toml_out = read_text(&codex_dir.join("config.toml"));
        assert!(!toml_out.contains("mcp_servers.cairn"));
        assert!(toml_out.contains("[mcp_servers.other]"));

        let hooks_out: Value =
            serde_json::from_str(&read_text(&codex_dir.join("hooks.json"))).unwrap();
        let remaining = hooks_out["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(remaining.len(), 1, "the foreign hook must survive reset");
        assert_eq!(remaining[0]["hooks"][0]["command"], "echo foreign-tool");
    }
}

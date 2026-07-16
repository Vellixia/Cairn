//! OpenCode: `$XDG_CONFIG_HOME/opencode/opencode.json` (or
//! `%USERPROFILE%\.config\opencode\opencode.json` on Windows) `mcp` key, plus a
//! generated plugin file that bridges lifecycle events to `cairn hook`.

use super::{Agent, InstallCtx, InstallReport, InstalledFile, RemovalAction};
use crate::jsonedit::{read_object, strip_cairn_plugin_entries, write_json};
use crate::paths;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// OpenCode lifecycle events this plugin bridges to Cairn hooks - documents what
/// `write_opencode_plugin`'s generated JS reacts to in one place rather than only inline in the
/// template string below (`session.created` -> SessionStart, `session.deleted`/`session.idle`
/// -> SessionEnd, `message.part.updated` -> PostToolUse when the part is a completed tool call).
const OPENCODE_EVENTS: &[&str] = &[
    "session.created",
    "session.deleted",
    "session.idle",
    "message.part.updated",
];

/// Bump whenever `write_opencode_plugin`'s generated JS changes in a way that makes a
/// previously-written plugin file stale (a new event mapping, a dispatch bug fix, ...).
/// `health()` compares this against the `cairn-plugin-rev: N` marker already in the file on
/// disk to decide whether `doctor --fix`/`cairn setup opencode` should rewrite it - the same
/// pattern `claude_code.rs` uses for its skill file's `cairn-guidance-rev`.
const PLUGIN_REV: u32 = 1;

pub struct OpenCode;

impl Agent for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }

    fn label(&self) -> &'static str {
        "OpenCode"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["oc"]
    }

    fn detect(&self, project: &Path, _home: Option<&Path>) -> bool {
        paths::opencode_config_path().exists() || project.join(".opencode").exists()
    }

    fn install(&self, _ctx: &InstallCtx) -> Result<InstallReport> {
        let path = paths::opencode_config_path();
        let mut cfg = read_object(&path)?;
        let mcp = cfg.entry("mcp").or_insert_with(|| json!({}));
        let mcp_obj = mcp
            .as_object_mut()
            .with_context(|| format!("{}: 'mcp' is not an object", path.display()))?;

        // v0.8.0 client redesign: server/token live only in `~/.cairn/config.toml` - any
        // `environment` block a pre-config-file `setup` run embedded here is dropped
        // (de-tokenized) rather than carried forward.
        let cli_exe = paths::cairn_exe();
        let entry = json!({
            "type": "local",
            "command": [cli_exe, "mcp"],
            "enabled": true
        });
        mcp_obj.insert("cairn".into(), entry);

        // Write the plugin file. OpenCode auto-loads every .js/.ts file in its
        // `plugins/` directory at startup, so it must NOT also be listed in the
        // `plugin` array (npm packages) - a local path there double-loads it,
        // firing every lifecycle hook twice. Strip any such entry an older
        // cairn version may have written so upgrades self-heal.
        let plugin_path = write_opencode_plugin()?;
        strip_cairn_plugin_entries(&mut cfg);

        // Write the skill file so the model knows how to use Cairn.
        let skill_dir = paths::opencode_skills_dir();
        std::fs::create_dir_all(&skill_dir)
            .with_context(|| format!("creating {}", skill_dir.display()))?;
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, cairn_mcp::guidance::skill_md())
            .with_context(|| format!("writing {}", skill_path.display()))?;

        write_json(&path, &Value::Object(cfg))?;

        Ok(InstallReport {
            files: vec![
                InstalledFile {
                    path,
                    note: "MCP server: cairn",
                },
                InstalledFile {
                    path: plugin_path,
                    note: "plugin: session + tool hooks, auto-loaded",
                },
                InstalledFile {
                    path: skill_path,
                    note: "skill: cairn playbook (loads on-demand)",
                },
            ],
            hint: None,
        })
    }

    fn removal_plan(&self, _project: &Path, _home: Option<&Path>) -> Vec<RemovalAction> {
        let path = paths::opencode_config_path();
        vec![
            RemovalAction::RemoveJsonKey {
                file: path.clone(),
                top_key: "mcp",
                inner_key: "cairn",
            },
            RemovalAction::StripOpenCodePluginArray { file: path },
            RemovalAction::DeleteFile {
                file: paths::opencode_plugin_path(),
            },
            RemovalAction::DeleteFile {
                file: paths::opencode_skills_dir().join("SKILL.md"),
            },
            RemovalAction::DeleteFile {
                file: paths::opencode_global_agents_md(),
            },
        ]
    }

    fn health(&self, _project: &Path, _home: Option<&Path>) -> Vec<String> {
        let mut issues = Vec::new();
        let cfg = paths::opencode_config_path();

        let want = format!("cairn-guidance-rev: {}", cairn_mcp::guidance::GUIDANCE_REV);
        let skill_path = paths::opencode_skills_dir().join("SKILL.md");
        if skill_path.exists()
            && !std::fs::read_to_string(&skill_path)
                .unwrap_or_default()
                .contains(&want)
        {
            issues.push(format!(
                "{} has a stale guidance revision (`cairn setup opencode` to refresh)",
                skill_path.display()
            ));
        }

        let double_registered = cfg
            .exists()
            .then(|| fs::read_to_string(&cfg).ok())
            .flatten()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| {
                v.get("plugin").and_then(|p| p.as_array()).map(|arr| {
                    arr.iter().any(|p| {
                        p.as_str().is_some_and(|s| {
                            let n = s.replace('\\', "/").to_ascii_lowercase();
                            n.ends_with("/plugins/cairn.js") || n == "plugins/cairn.js"
                        })
                    })
                })
            })
            .unwrap_or(false);
        if double_registered {
            issues.push(
                "opencode.json lists the cairn plugin in its `plugin` array; it already \
                 auto-loads from plugins/ and will double-fire (`cairn setup opencode` to fix)"
                    .to_string(),
            );
        }

        let plugin_path = paths::opencode_plugin_path();
        if plugin_path.exists() {
            let want = format!("cairn-plugin-rev: {PLUGIN_REV}");
            if !fs::read_to_string(&plugin_path)
                .unwrap_or_default()
                .contains(&want)
            {
                issues.push(format!(
                    "{} has a stale plugin revision (`cairn setup opencode` to refresh)",
                    plugin_path.display()
                ));
            }
        }
        issues
    }
}

/// Write a minimal OpenCode plugin that bridges lifecycle events to `cairn hook`.
/// Returns the absolute path to the plugin file so the caller can register it
/// (or, since v0.8.0's client redesign, NOT register it — see `install` above).
fn write_opencode_plugin() -> Result<PathBuf> {
    let plugin_path = paths::opencode_plugin_path();
    if let Some(parent) = plugin_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Use the absolute path to the cairn binary so the plugin works regardless of
    // PATH resolution at OpenCode startup. serde_json gives us correct JSON
    // escaping (backslashes, quotes) on either Windows or Unix paths.
    let exe_json = serde_json::to_string(&paths::cairn_exe())?;

    let events_comment = OPENCODE_EVENTS.join(", ");
    let plugin_content = format!(
        r#"// Cairn lifecycle plugin. Bridges OpenCode session/tool events to `cairn hook`.
// Managed by `cairn setup` -- changes will be preserved across runs.
// Uses the OpenCode `Plugin` API (see `@opencode-ai/plugin`) so we can react
// to `chat.message` (UserPromptSubmit equivalent) in addition to session and
// tool events.
// Events handled: {events_comment}
// cairn-plugin-rev: {PLUGIN_REV}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{InstallCtx, Scope};

    fn read_text(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn install_writes_bare_command_with_no_environment() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();
        let exe = paths::cairn_exe();

        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&home_str))], || {
            let ctx = InstallCtx {
                project: home.path(),
                home: Some(home.path()),
                scope: Scope::Global,
            };
            OpenCode.install(&ctx).unwrap();

            let v: Value =
                serde_json::from_str(&read_text(&paths::opencode_config_path())).unwrap();
            assert_eq!(v["mcp"]["cairn"]["command"][0], exe);
            assert_eq!(v["mcp"]["cairn"]["command"][1], "mcp");
            assert_eq!(v["mcp"]["cairn"]["type"], "local");
            assert_eq!(v["mcp"]["cairn"]["enabled"], true);
            assert!(
                v["mcp"]["cairn"]["environment"].is_null(),
                "server/token live only in ~/.cairn/config.toml, never embedded here"
            );
        });
    }

    #[test]
    fn strips_stale_plugin_entry_and_never_adds_one() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();

        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&home_str))], || {
            let cfg_path = paths::opencode_config_path();
            if let Some(parent) = cfg_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(
                &cfg_path,
                r#"{"plugin":["plugins/cairn.js","./plugins/agentmemory-capture.ts"]}"#,
            )
            .unwrap();

            let ctx = InstallCtx {
                project: home.path(),
                home: Some(home.path()),
                scope: Scope::Global,
            };
            OpenCode.install(&ctx).unwrap();

            let v: Value = serde_json::from_str(&read_text(&cfg_path)).unwrap();
            assert_eq!(v["mcp"]["cairn"]["command"][0], paths::cairn_exe());
            let plugins = v["plugin"].as_array().unwrap();
            assert!(
                !plugins.iter().any(|p| p
                    .as_str()
                    .map(|s| s.to_ascii_lowercase().contains("cairn.js"))
                    .unwrap_or(false)),
                "setup must strip the local cairn plugin from the `plugin` array"
            );
            assert!(plugins
                .iter()
                .any(|p| p.as_str() == Some("./plugins/agentmemory-capture.ts")));
        });
    }

    #[test]
    fn install_and_removal_round_trip_via_xdg_env() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();

        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&home_str))], || {
            let ctx = InstallCtx {
                project: home.path(),
                home: Some(home.path()),
                scope: Scope::Global,
            };
            OpenCode.install(&ctx).unwrap();

            let cfg_path = paths::opencode_config_path();
            assert!(cfg_path.exists());
            let plugin_path = paths::opencode_plugin_path();
            assert!(plugin_path.exists(), "plugin file must be written");

            for action in OpenCode.removal_plan(home.path(), Some(home.path())) {
                action.apply().unwrap();
            }
            let v: Value = serde_json::from_str(&read_text(&cfg_path)).unwrap();
            assert!(v["mcp"].get("cairn").is_none());
            assert!(
                !plugin_path.exists(),
                "plugin file must be deleted by reset"
            );
        });
    }

    #[test]
    fn health_flags_a_stale_plugin_revision_and_clears_after_reinstall() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();

        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&home_str))], || {
            assert!(
                OpenCode.health(home.path(), Some(home.path())).is_empty(),
                "no plugin file yet - nothing to flag"
            );

            let plugin_path = paths::opencode_plugin_path();
            fs::create_dir_all(plugin_path.parent().unwrap()).unwrap();
            fs::write(&plugin_path, "// cairn-plugin-rev: 0\nstale content").unwrap();
            let issues = OpenCode.health(home.path(), Some(home.path()));
            assert_eq!(issues.len(), 1);
            assert!(issues[0].contains("stale plugin revision"));

            let ctx = InstallCtx {
                project: home.path(),
                home: Some(home.path()),
                scope: Scope::Global,
            };
            OpenCode.install(&ctx).unwrap();
            assert!(OpenCode.health(home.path(), Some(home.path())).is_empty());
        });
    }

    #[test]
    fn install_de_tokenizes_an_environment_block_left_by_an_older_binary() {
        let home = tempfile::tempdir().unwrap();
        let home_str = home.path().to_string_lossy().into_owned();

        crate::env_guard::with_env(&[("XDG_CONFIG_HOME", Some(&home_str))], || {
            let cfg_path = paths::opencode_config_path();
            if let Some(parent) = cfg_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            // Simulate a pre-v0.8.0 binary's opencode.json, which used to embed
            // server/token directly into `mcp.cairn.environment`.
            fs::write(
                &cfg_path,
                json!({
                    "mcp": {
                        "cairn": {
                            "type": "local",
                            "command": ["cairn", "mcp"],
                            "environment": { "CAIRN_TOKEN": "old-token" },
                            "enabled": true
                        }
                    }
                })
                .to_string(),
            )
            .unwrap();

            let ctx = InstallCtx {
                project: home.path(),
                home: Some(home.path()),
                scope: Scope::Global,
            };
            OpenCode.install(&ctx).unwrap();
            let after: Value = serde_json::from_str(&read_text(&cfg_path)).unwrap();
            assert!(
                after["mcp"]["cairn"]["environment"].is_null(),
                "install must not write (or keep) an environment block"
            );
        });
    }
}

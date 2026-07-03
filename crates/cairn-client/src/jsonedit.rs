//! Shared JSON / TOML / managed-block file editing helpers for agent config
//! files. Used by both `agents::*::install()` (write) and `agents::*::removal_plan()`
//! executors (remove) so the two can never drift into different ideas of what
//! "a cairn hook" or "a cairn entry" looks like.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

pub fn read_object(path: &Path) -> Result<Map<String, Value>> {
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

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(value)?;
    fs::write(path, format!("{text}\n")).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Normalize a hook command string to its canonical form for dedup comparison.
///
/// Returns just the suffix after the cairn binary path (e.g. `"hook SessionStart"`),
/// ignoring whether the original was `"cairn hook SessionStart"`,
/// `"D:\\code\\Cairn\\target\\debug\\cairn.exe hook SessionStart"`, or any other
/// absolute/bare path to the same binary. Used to coalesce duplicates left by
/// multiple `cairn setup` runs from different binary locations.
/// True for `cairn`/`cairn.exe`, and for hash- or version-suffixed variants
/// like `cairn-effd16d15b110943.exe` (what `std::env::current_exe()` returns
/// from *inside a `cargo test` binary*, since test executables get a
/// content-hashed name — `paths::cairn_exe()` calls `current_exe()` too, so
/// any test that exercises real `install()` + real hook-removal end to end
/// observes exactly this shape) or a future `cairn-v0.8.0.exe`-style release
/// asset name.
fn is_cairn_binary_name(basename: &str) -> bool {
    basename == "cairn" || basename == "cairn.exe" || basename.starts_with("cairn-")
}

#[cfg(test)] // only consumer is the test-only `is_cairn_hook` below
fn hook_suffix(command: &str) -> String {
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
    if !is_cairn_binary_name(basename) {
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
    let event_lower = event.to_ascii_lowercase();
    suffix == event_lower || suffix == format!("\"{event_lower}\"")
}

/// True when `command` is any `cairn hook <anything>` invocation (event-agnostic).
/// Used to strip every cairn-owned hook regardless of event name.
pub fn is_any_cairn_hook(command: &str) -> bool {
    let original = command.trim_start().to_ascii_lowercase();
    let first = original.split_whitespace().next().unwrap_or("");
    let basename = first.rsplit(['\\', '/']).next().unwrap_or(first);
    is_cairn_binary_name(basename)
}

/// Drop pre-existing cairn-owned entries for the given event so a re-run replaces
/// stale bare/path duplicates with the current absolute-path entry. Returns the
/// number of entries removed.
fn dedup_cairn_hooks(arr: &mut Vec<Value>) -> usize {
    let before = arr.len();
    arr.retain(|g| {
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

/// Add (or idempotently replace) a cairn-owned hook entry for `event` in a
/// Claude-Code-shaped `hooks` object (`{"<event>": [{"matcher"?, "hooks": [...]}]}`).
/// Shared by Claude Code and Codex — both use this exact shape. Strips any
/// stale cairn-owned entries for the event first, so re-running `cairn setup`
/// from a different binary path (bare `cairn`, debug build, release install)
/// coalesces into exactly one entry per event.
pub fn add_hook(hooks: &mut Map<String, Value>, event: &str, command: &str, matcher: Option<&str>) {
    let groups = hooks
        .entry(event)
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut();
    let Some(groups) = groups else { return };

    dedup_cairn_hooks(groups);

    // Guard against accidental manual duplicates without losing other tools' hooks.
    let already_exact = groups.iter().any(|g| {
        g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
        })
    });
    if !already_exact {
        let mut group = serde_json::json!({ "hooks": [{ "type": "command", "command": command }] });
        if let Some(m) = matcher {
            group["matcher"] = serde_json::json!(m);
        }
        groups.push(group);
    }
}

/// Remove every cairn-owned hook entry for `event` from a Claude-Code-shaped
/// `hooks` object, WITHOUT touching other tools' entries for the same event.
/// (`cairn reset` used to `hooks.remove(event)`, wiping the whole event key —
/// including any other tool's hooks registered for it.) Returns `true` if
/// anything was actually removed.
pub fn strip_cairn_hooks(hooks: &mut Map<String, Value>, event: &str) -> bool {
    let Some(arr) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
        return false;
    };
    let before = arr.len();
    arr.retain(|g| {
        !g.get("hooks")
            .and_then(Value::as_array)
            .map(|hs| {
                hs.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .is_some_and(is_any_cairn_hook)
                })
            })
            .unwrap_or(false)
    });
    let changed = arr.len() < before;
    if arr.is_empty() {
        hooks.remove(event);
    }
    changed
}

/// Remove any cairn plugin entries from an OpenCode `opencode.json`'s top-level
/// `plugin` array (npm-package list). OpenCode auto-loads every local plugin
/// file under `plugins/` at startup, so a local path *also* listed here makes
/// OpenCode load it twice (every lifecycle hook fires twice). Older cairn
/// versions wrote such an entry; stripping it — on both `setup` (self-heal) and
/// `reset` (cleanup) — never creates an empty `plugin` key when there wasn't
/// one. Returns `true` if anything changed.
pub fn strip_cairn_plugin_entries(cfg: &mut Map<String, Value>) -> bool {
    let Some(plugins) = cfg.get_mut("plugin").and_then(Value::as_array_mut) else {
        return false;
    };
    let before = plugins.len();
    plugins.retain(|p| {
        p.as_str()
            .map(|s| {
                let normalized = s.replace('\\', "/").to_ascii_lowercase();
                !normalized.ends_with("/plugins/cairn.js") && normalized != "plugins/cairn.js"
            })
            .unwrap_or(true)
    });
    plugins.len() < before
}

/// Remove the `[mcp_servers.cairn]` table (and its `.env` sub-table) from a
/// Codex `config.toml`, preserving all other tables, comments, and formatting
/// via `toml_edit`. If the file is not valid TOML, it is returned unchanged
/// rather than risking corruption.
pub fn remove_codex_cairn_block(toml: &str) -> String {
    let mut doc = match toml.parse::<toml_edit::DocumentMut>() {
        Ok(d) => d,
        Err(_) => return toml.to_string(),
    };
    if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_mut()) {
        servers.remove("cairn");
        if servers.is_empty() {
            doc.as_table_mut().remove("mcp_servers");
        }
    }
    doc.to_string()
}

/// Remove a `<!-- BEGIN CAIRN ... --> ... <!-- END CAIRN -->` managed block from
/// a shared instructions file (CLAUDE.md/AGENTS.md), preserving everything else.
/// Returns `None` when there's no such block, or the cleaned text is entirely
/// blank (so the caller can decide not to leave a whitespace-only file).
pub fn remove_managed_block(text: &str) -> Option<String> {
    let begin = "<!-- BEGIN CAIRN";
    let end = "<!-- END CAIRN -->";
    let start = text.find(begin)?;
    let end_pos = text[start..].find(end)?;
    let before = &text[..start];
    let after = &text[start + end_pos + end.len()..];
    let cleaned = format!("{}{}", before.trim_end(), after);
    if cleaned.trim().is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hook_suffix_strips_binary_paths_case_insensitively() {
        let fake_exe = "C:\\Users\\foo\\.local\\bin\\cairn.exe";
        assert_eq!(hook_suffix("cairn hook SessionStart"), "SessionStart");
        assert_eq!(hook_suffix("CAIRN HOOK SessionStart"), "SessionStart");
        assert_eq!(
            hook_suffix(&format!("{fake_exe} hook SessionStart")),
            "SessionStart"
        );
        assert_eq!(
            hook_suffix("cairn.exe hook UserPromptSubmit"),
            "UserPromptSubmit"
        );
        assert_eq!(
            hook_suffix("D:\\old\\path\\cairn.exe hook PostToolUse"),
            "PostToolUse"
        );
    }

    #[test]
    fn is_any_cairn_hook_recognizes_hash_suffixed_test_binary_names() {
        // `paths::cairn_exe()` calls `std::env::current_exe()`, which from
        // inside a `cargo test` binary returns a hash-suffixed name like
        // this - any test exercising real install() + real removal needs
        // this recognized as cairn-owned, not just literal "cairn.exe".
        assert!(is_any_cairn_hook(
            "D:\\code\\Cairn\\target\\debug\\deps\\cairn-effd16d15b110943.exe hook SessionStart"
        ));
        assert!(is_any_cairn_hook("cairn-v0.8.0.exe hook SessionStart"));
    }

    #[test]
    fn is_cairn_hook_matches_across_path_variants() {
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
    fn strip_cairn_hooks_removes_only_cairn_owned_groups() {
        let mut hooks = Map::new();
        hooks.insert(
            "SessionStart".into(),
            json!([
                { "hooks": [{ "type": "command", "command": "echo hi" }] },
                { "hooks": [{ "type": "command", "command": "cairn hook SessionStart" }] },
            ]),
        );
        let changed = strip_cairn_hooks(&mut hooks, "SessionStart");
        assert!(changed);
        let remaining = hooks["SessionStart"].as_array().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["hooks"][0]["command"], "echo hi");
    }

    #[test]
    fn strip_cairn_hooks_removes_key_entirely_when_empty() {
        let mut hooks = Map::new();
        hooks.insert(
            "SessionStart".into(),
            json!([{ "hooks": [{ "type": "command", "command": "cairn hook SessionStart" }] }]),
        );
        strip_cairn_hooks(&mut hooks, "SessionStart");
        assert!(!hooks.contains_key("SessionStart"));
    }

    #[test]
    fn strip_cairn_hooks_is_a_noop_on_second_run() {
        let mut hooks = Map::new();
        hooks.insert(
            "SessionStart".into(),
            json!([{ "hooks": [{ "type": "command", "command": "echo hi" }] }]),
        );
        assert!(!strip_cairn_hooks(&mut hooks, "SessionStart"));
    }

    #[test]
    fn strip_cairn_plugin_entries_removes_all_cairn_paths_keeps_foreign() {
        let mut root = Map::new();
        root.insert(
            "plugin".into(),
            json!([
                "C:\\Users\\foo\\.config\\opencode\\plugins\\cairn.js",
                "./plugins/agentmemory-capture.ts",
                "plugins/cairn.js"
            ]),
        );

        let changed = strip_cairn_plugin_entries(&mut root);
        assert!(changed);

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
        let mut root = Map::new();
        assert!(!strip_cairn_plugin_entries(&mut root));
        assert!(!root.contains_key("plugin"));
    }

    #[test]
    fn remove_codex_cairn_block_keeps_other_servers_and_formatting() {
        let original = "# head\n[mcp_servers.cairn]\ncommand = \"stale\"\n\n[mcp_servers.other]\ncommand = \"foo\"\n";
        let out = remove_codex_cairn_block(original);
        assert!(!out.contains("mcp_servers.cairn"));
        assert!(out.contains("[mcp_servers.other]"));
        assert!(out.contains("command = \"foo\""));
    }

    #[test]
    fn remove_codex_cairn_block_removes_empty_parent_table() {
        let original = "[mcp_servers.cairn]\ncommand = \"stale\"\n";
        let out = remove_codex_cairn_block(original);
        assert!(!out.contains("mcp_servers"));
    }

    #[test]
    fn remove_codex_cairn_block_returns_input_unchanged_on_invalid_toml() {
        let original = "not [ valid toml";
        assert_eq!(remove_codex_cairn_block(original), original);
    }

    #[test]
    fn remove_managed_block_strips_block_and_keeps_surrounding_text() {
        let text = "# My rules\n\nAlways write tests.\n\n<!-- BEGIN CAIRN (managed by `cairn rules`) -->\nCairn stuff\n<!-- END CAIRN -->\n";
        let cleaned = remove_managed_block(text).unwrap();
        assert!(cleaned.contains("Always write tests."));
        assert!(!cleaned.contains("BEGIN CAIRN"));
        assert!(!cleaned.contains("Cairn stuff"));
    }

    #[test]
    fn remove_managed_block_returns_none_when_absent() {
        assert!(remove_managed_block("just some text").is_none());
    }

    #[test]
    fn remove_managed_block_returns_none_when_result_is_blank() {
        let text = "<!-- BEGIN CAIRN (managed by `cairn rules`) -->\nstuff\n<!-- END CAIRN -->\n";
        assert!(remove_managed_block(text).is_none());
    }
}

//! Installing and removing the Claude Code integration (FR-043, D16).
//!
//! Writes the six lifecycle hooks and the MCP server entry into the
//! repository's own configuration, merging rather than clobbering — a
//! developer's existing hooks are not ours to remove.

use crate::client;
use crate::Output;
use cairn_core::wire::{Request, WireError};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// The events Claude Code actually emits, and that Cairn uses (FR-041).
const EVENTS: &[&str] = &[
    "SessionStart",
    "PostToolUse",
    "PostToolUseFailure",
    "PreCompact",
    "Stop",
    "SessionEnd",
];

/// Events that fire per tool call and therefore carry a matcher.
const TOOL_EVENTS: &[&str] = &["PostToolUse", "PostToolUseFailure"];

pub async fn connect(agent: &str) -> Result<Output, WireError> {
    if agent != "claude-code" {
        return Err(WireError::invalid(format!(
            "unknown agent `{agent}`; Feature 001 integrates claude-code. \
             Other MCP-compatible agents can use `cairn mcp` directly."
        )));
    }
    // Registering the project first means `cairn status` is meaningful the
    // moment the integration exists.
    let init = client::send(&Request::Init { cwd: crate::cwd() }).await?;
    let root = worktree_from(&init)?;

    let settings_path = root.join(".claude").join("settings.json");
    let mut settings = read_json(&settings_path);
    install_hooks(&mut settings);
    write_json(&settings_path, &settings)?;

    let mcp_path = root.join(".mcp.json");
    let mut mcp = read_json(&mcp_path);
    install_mcp(&mut mcp);
    write_json(&mcp_path, &mcp)?;

    Ok(Output::with(
        json!({
            "connected": "claude-code",
            "settings": settings_path.display().to_string(),
            "mcp": mcp_path.display().to_string(),
            "events": EVENTS,
        }),
        format!(
            "Connected Claude Code.\n  hooks: {}\n  mcp:   {}\nStart a session in this \
             repository and Cairn will capture it.\n",
            settings_path.display(),
            mcp_path.display()
        ),
    ))
}

pub async fn disconnect(agent: &str) -> Result<Output, WireError> {
    if agent != "claude-code" {
        return Err(WireError::invalid(format!("unknown agent `{agent}`")));
    }
    let init = client::send(&Request::Init { cwd: crate::cwd() }).await?;
    let root = worktree_from(&init)?;

    let settings_path = root.join(".claude").join("settings.json");
    if settings_path.exists() {
        let mut settings = read_json(&settings_path);
        remove_hooks(&mut settings);
        write_json(&settings_path, &settings)?;
    }
    let mcp_path = root.join(".mcp.json");
    if mcp_path.exists() {
        let mut mcp = read_json(&mcp_path);
        if let Some(servers) = mcp.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
            servers.remove("cairn");
        }
        write_json(&mcp_path, &mcp)?;
    }

    Ok(Output::with(
        json!({ "disconnected": "claude-code" }),
        "Disconnected Claude Code. Local memory is untouched.\n".to_string(),
    ))
}

fn worktree_from(init: &Value) -> Result<PathBuf, WireError> {
    init.get("worktree_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| WireError::invalid("could not determine the repository root"))
}

fn install_hooks(settings: &mut Value) {
    let root = settings.as_object_mut().expect("object");
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().expect("hooks object");

    for event in EVENTS {
        let entry = if TOOL_EVENTS.contains(event) {
            json!({
                "matcher": "*",
                "hooks": [{ "type": "command", "command": command_for(event) }]
            })
        } else {
            json!({ "hooks": [{ "type": "command", "command": command_for(event) }] })
        };

        let list = hooks.entry(*event).or_insert_with(|| json!([]));
        let Some(array) = list.as_array_mut() else {
            continue;
        };
        // Idempotent: connecting twice must not double-register.
        array.retain(|existing| !mentions_cairn(existing));
        array.push(entry);
    }
}

fn remove_hooks(settings: &mut Value) {
    let Some(root) = settings.as_object_mut() else {
        return;
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|v| v.as_object_mut()) else {
        return;
    };
    let mut empty: Vec<String> = Vec::new();
    for (event, list) in hooks.iter_mut() {
        if let Some(array) = list.as_array_mut() {
            array.retain(|existing| !mentions_cairn(existing));
            if array.is_empty() {
                empty.push(event.clone());
            }
        }
    }
    for event in empty {
        hooks.remove(&event);
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
}

fn command_for(event: &str) -> String {
    format!("cairn hook {event}")
}

/// True when this hook entry is one of ours.
fn mentions_cairn(entry: &Value) -> bool {
    entry.to_string().contains("cairn hook")
}

fn install_mcp(config: &mut Value) {
    let root = config.as_object_mut().expect("object");
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    if let Some(map) = servers.as_object_mut() {
        map.insert(
            "cairn".to_string(),
            json!({ "command": "cairn", "args": ["mcp"] }),
        );
    }
}

fn read_json(path: &Path) -> Value {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| Value::Object(Map::new())),
        Err(_) => Value::Object(Map::new()),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), WireError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| WireError::invalid(format!("{}: {e}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    std::fs::write(path, format!("{text}\n"))
        .map_err(|e| WireError::invalid(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_all_six_events_and_no_others() {
        let mut settings = json!({});
        install_hooks(&mut settings);
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), EVENTS.len());
        for e in EVENTS {
            assert!(hooks.contains_key(*e), "{e} missing");
        }
        // The failure event is real and must be registered (D16).
        assert!(hooks.contains_key("PostToolUseFailure"));
    }

    #[test]
    fn preserves_a_developers_existing_hooks() {
        let mut settings = json!({
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "make lint" }] }]
            },
            "model": "opus"
        });
        install_hooks(&mut settings);
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "existing hook was dropped");
        assert!(stop.iter().any(|h| h.to_string().contains("make lint")));
        assert_eq!(settings["model"], "opus");
    }

    #[test]
    fn connecting_twice_does_not_double_register() {
        let mut settings = json!({});
        install_hooks(&mut settings);
        install_hooks(&mut settings);
        for e in EVENTS {
            assert_eq!(
                settings["hooks"][*e].as_array().unwrap().len(),
                1,
                "{e} duplicated"
            );
        }
    }

    #[test]
    fn disconnect_removes_only_cairn_entries() {
        let mut settings = json!({
            "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "make lint" }] }] }
        });
        install_hooks(&mut settings);
        remove_hooks(&mut settings);
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 1);
        assert!(stop[0].to_string().contains("make lint"));
    }

    #[test]
    fn mcp_entry_is_a_single_stdio_server() {
        let mut config = json!({});
        install_mcp(&mut config);
        assert_eq!(config["mcpServers"]["cairn"]["command"], "cairn");
        assert_eq!(config["mcpServers"]["cairn"]["args"][0], "mcp");
    }
}

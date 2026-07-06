//! `cairn rules` --- write per-agent instruction files that tell the model to actually USE Cairn.
//!
//! Registering an MCP server is not enough: the agent has to be *told* to prefer Cairn's tools
//! over its defaults. This writes that guidance into each agent's native instructions file,
//! idempotently: shared files (CLAUDE.md, AGENTS.md) get a replaceable **managed block**.
//!
//! v0.8.0 Sprint 10 (B6, layer 3): the block content itself now renders from
//! [`cairn_mcp::guidance`], the single source of truth shared with the MCP `instructions` field
//! and the Claude Code skill - Claude Code gets [`cairn_mcp::guidance::claude_md_block`] (slim;
//! the skill carries the real playbook), Codex/OpenCode (no skill system) get the fuller
//! [`cairn_mcp::guidance::agents_md_block`].

use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

/// Agents we can write rules for (`agents` = a generic AGENTS.md).
const KNOWN: &[&str] = &["claude-code", "codex", "opencode", "agents"];

const BEGIN: &str = "<!-- BEGIN CAIRN (managed by `cairn rules`) -->";
const END: &str = "<!-- END CAIRN -->";

/// Write the Cairn rules into `id`'s native instruction file under `project`.
pub fn write_for(id: &str, project: &Path) -> Result<()> {
    let (path, block) = match id {
        "claude-code" => (
            project.join("CLAUDE.md"),
            cairn_mcp::guidance::claude_md_block(),
        ),
        // Codex CLI reads AGENTS.md from the project root (or `$CODEX_HOME/AGENTS.md` for
        // user-scope rules); OpenCode has no rules-file convention of its own and shares the
        // same generic AGENTS.md target. We use the project root to stay scoped.
        "codex" | "agents" | "opencode" => (
            project.join("AGENTS.md"),
            cairn_mcp::guidance::agents_md_block(),
        ),
        other => bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    };
    managed(&path, &block)?;
    println!("\u{2713} wrote Cairn rules: {}", path.display());
    Ok(())
}

/// Insert or replace the Cairn managed block in a (possibly shared) file, preserving the rest.
fn managed(path: &Path, body: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let block = format!("{BEGIN}\n{body}\n{END}");
    let updated = match (existing.find(BEGIN), existing.find(END)) {
        (Some(s), Some(e)) if e > s => {
            let mut out = String::with_capacity(existing.len());
            out.push_str(&existing[..s]);
            out.push_str(&block);
            out.push_str(&existing[e + END.len()..]);
            out
        }
        _ if existing.trim().is_empty() => format!("{block}\n"),
        _ => format!("{}\n\n{block}\n", existing.trim_end()),
    };
    write_file(path, &updated)
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_block_is_idempotent_and_non_destructive() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("CLAUDE.md");
        fs::write(&p, "# My project rules\n\nAlways write tests.\n").unwrap();

        write_for("claude-code", dir.path()).unwrap();
        let after_first = fs::read_to_string(&p).unwrap();
        write_for("claude-code", dir.path()).unwrap(); // twice
        let after_second = fs::read_to_string(&p).unwrap();

        assert_eq!(after_first, after_second);
        assert_eq!(after_first.matches(BEGIN).count(), 1);
        assert_eq!(after_first.matches(END).count(), 1);
        assert!(after_first.contains("Always write tests."));
        // Claude Code gets the SLIM pointer (layer 3 fallback) - the real playbook lives in
        // the installed skill (layer 2), not permanently in CLAUDE.md.
        assert!(after_first.contains("Cairn MCP is connected"));
        assert!(after_first.contains("cairn skill"));
    }

    #[test]
    fn codex_targets_agents_md_at_project_root_with_the_fuller_block() {
        let dir = tempfile::tempdir().unwrap();
        write_for("codex", dir.path()).unwrap();
        let p = dir.path().join("AGENTS.md");
        assert!(p.exists());
        let content = fs::read_to_string(&p).unwrap();
        assert!(content.contains(BEGIN));
        // Codex has no skill system to fall back on, so AGENTS.md carries the fuller block
        // (same content class as the pre-Sprint-10 CLAUDE.md block) rather than a slim pointer.
        assert!(content.contains("prefer these tools"));
        assert!(content.contains("`recall`"));
    }

    #[test]
    fn opencode_shares_the_same_agents_md_target_and_block_as_codex() {
        let dir = tempfile::tempdir().unwrap();
        write_for("opencode", dir.path()).unwrap();
        let content = fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(content.contains("prefer these tools"));
    }
}

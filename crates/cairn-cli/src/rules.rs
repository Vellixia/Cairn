//! `cairn rules` — write per-agent instruction files that tell the model to actually USE Cairn.
//!
//! Registering an MCP server is not enough: the agent has to be *told* to prefer Cairn's tools
//! (`read`/`recall`/`remember`/`sanitize`/…) over its defaults — exactly like a hand-written rules
//! file. This writes that guidance into each agent's native instructions file, idempotently:
//! shared files (CLAUDE.md, copilot-instructions, AGENTS.md) get a replaceable **managed block**;
//! dedicated files (Cursor `.mdc`, Windsurf rules) are owned outright.

use anyhow::{anyhow, bail, Result};
use std::fs;
use std::path::Path;

/// Agents we can write rules for (`agents` = a generic AGENTS.md).
const KNOWN: &[&str] = &["claude-code", "cursor", "vscode", "windsurf", "opencode", "agents"];

const BEGIN: &str = "<!-- BEGIN CAIRN (managed by `cairn rules`) -->";
const END: &str = "<!-- END CAIRN -->";

/// The instruction body — what every agent is told about using Cairn. Kept tool-name-accurate.
const BODY: &str = "\
## Cairn — prefer these tools

You have **Cairn** (MCP server `cairn`): persistent memory, lean context, and edit safety. Use it.

- **Reading code/files:** use `read` instead of your default file read — unchanged re-reads are
  nearly free, and `mode:\"signatures\"` returns a large file as just its structure (huge token
  saving). Recover any full original with `expand`.
- **Memory:** at the start of a task, `recall` (or `assemble`) relevant past decisions and context;
  `remember` decisions, gotchas, and rationale as you make them so the next session never starts
  cold. Record standing user preferences with `prefer`.
- **Before sharing, logging, or committing text:** run `sanitize` to redact secrets/PII.
- **Risky edits:** `checkpoint` before large changes; `verify` a proposed file against its retained
  original to catch silent corruption; `rollback` to undo damage.
- **Stay on task:** keep the current goal in `anchor`.

Everything Cairn shows is lossless — the full original is always one `expand` away.";

pub fn run(agent: Option<&str>, all: bool) -> Result<()> {
    let project = std::env::current_dir()?;
    if all {
        for id in KNOWN {
            write_for(id, &project)?;
        }
        return Ok(());
    }
    let id = canonical(agent.unwrap_or("agents")).ok_or_else(|| {
        anyhow!(
            "unknown agent '{}'. Supported: {}.",
            agent.unwrap_or(""),
            KNOWN.join(", ")
        )
    })?;
    write_for(id, &project)
}

/// Map an agent name (and aliases) to a canonical id.
pub fn canonical(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "claude-code" | "claude" | "claudecode" | "cc" => Some("claude-code"),
        "cursor" => Some("cursor"),
        "vscode" | "code" | "vs-code" | "copilot" => Some("vscode"),
        "windsurf" | "codeium" => Some("windsurf"),
        "opencode" | "oc" => Some("opencode"),
        "agents" | "generic" => Some("agents"),
        _ => None,
    }
}

/// Write the Cairn rules into `id`'s native instruction file under `project`.
pub fn write_for(id: &str, project: &Path) -> Result<()> {
    let path = match id {
        "claude-code" => {
            managed(&project.join("CLAUDE.md"))?;
            project.join("CLAUDE.md")
        }
        "vscode" => {
            let p = project.join(".github").join("copilot-instructions.md");
            managed(&p)?;
            p
        }
        "agents" | "opencode" => {
            managed(&project.join("AGENTS.md"))?;
            project.join("AGENTS.md")
        }
        "cursor" => {
            let p = project.join(".cursor").join("rules").join("cairn.mdc");
            dedicated(&p, &cursor_mdc())?;
            p
        }
        "windsurf" => {
            let p = project.join(".windsurf").join("rules").join("cairn.md");
            dedicated(&p, BODY)?;
            p
        }
        other => bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    };
    println!("\u{2713} wrote Cairn rules: {}", path.display());
    Ok(())
}

/// Cursor's `.mdc` wants YAML frontmatter; `alwaysApply: true` makes it always in context.
fn cursor_mdc() -> String {
    format!("---\ndescription: Use Cairn's tools for memory, lean reads, and edit safety.\nalwaysApply: true\n---\n\n{BODY}\n")
}

/// Insert or replace the Cairn managed block in a (possibly shared) file, preserving the rest.
fn managed(path: &Path) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let block = format!("{BEGIN}\n{BODY}\n{END}");
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

/// Write a file Cairn owns entirely (dedicated rules files).
fn dedicated(path: &Path, content: &str) -> Result<()> {
    let body = if content.ends_with('\n') {
        content.to_string()
    } else {
        format!("{content}\n")
    };
    write_file(path, &body)
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

        // Idempotent: exactly one managed block, identical content both runs.
        assert_eq!(after_first, after_second);
        assert_eq!(after_first.matches(BEGIN).count(), 1);
        assert_eq!(after_first.matches(END).count(), 1);
        // Non-destructive: the user's own content survives.
        assert!(after_first.contains("Always write tests."));
        // And it actually tells the model to use Cairn.
        assert!(after_first.contains("prefer these tools"));
        assert!(after_first.contains("`recall`"));
    }

    #[test]
    fn cursor_and_windsurf_get_dedicated_files() {
        let dir = tempfile::tempdir().unwrap();
        write_for("cursor", dir.path()).unwrap();
        let mdc = fs::read_to_string(dir.path().join(".cursor/rules/cairn.mdc")).unwrap();
        assert!(mdc.starts_with("---")); // frontmatter
        assert!(mdc.contains("alwaysApply: true"));
        assert!(mdc.contains("Cairn"));

        write_for("windsurf", dir.path()).unwrap();
        assert!(dir.path().join(".windsurf/rules/cairn.md").exists());
    }

    #[test]
    fn vscode_targets_copilot_instructions() {
        let dir = tempfile::tempdir().unwrap();
        write_for("vscode", dir.path()).unwrap();
        let p = dir.path().join(".github/copilot-instructions.md");
        assert!(p.exists());
        assert!(fs::read_to_string(&p).unwrap().contains("Cairn"));
    }

    #[test]
    fn canonical_resolves_aliases() {
        assert_eq!(canonical("copilot"), Some("vscode"));
        assert_eq!(canonical("claude"), Some("claude-code"));
        assert_eq!(canonical("generic"), Some("agents"));
        assert!(canonical("nope").is_none());
    }
}

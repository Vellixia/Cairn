//! `cairn rules` --- write per-agent instruction files that tell the model to actually USE Cairn.
//!
//! Registering an MCP server is not enough: the agent has to be *told* to prefer Cairn's tools
//! (`read`/`recall`/`remember`/`sanitize`/...) over its defaults --- exactly like a hand-written rules
//! file. This writes that guidance into each agent's native instructions file, idempotently:
//! shared files (CLAUDE.md, AGENTS.md) get a replaceable **managed block**.

use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

/// Agents we can write rules for (`agents` = a generic AGENTS.md).
const KNOWN: &[&str] = &["claude-code", "codex", "opencode", "agents"];

const BEGIN: &str = "<!-- BEGIN CAIRN (managed by `cairn rules`) -->";
const END: &str = "<!-- END CAIRN -->";

/// The instruction body --- what every agent is told about using Cairn. Kept tool-name-accurate.
const BODY: &str = "\
## Cairn --- prefer these tools

You have **Cairn** (MCP server `cairn`): persistent memory, lean context, and edit safety. Use it.

- **Reading code/files:** use `read` instead of your default file read - unchanged re-reads are
  nearly free, and `mode:\"signatures\"` returns a large file as just its structure (huge token
  saving). Recover any full original with `expand`.
- **Verbose tool output:** run `compress` to shrink cargo/build/log output into a compact view,
  retaining the exact original (recover with `expand`).
- **Memory:** at the start of a task, `wakeup` auto-injects your highest-priority memories so
  you never start cold. Use `recall` (quick) or `search` (hybrid BM25+semantic) to find relevant
  past decisions and context; `remember` decisions, gotchas, and rationale as you make them.
  Record standing user preferences with `prefer`. Call `proactive_recall` at the start of each
  turn to get context automatically injected. Use `assemble` to build a context block under a
  token budget.
- **Before sharing, logging, or committing text:** run `sanitize` to redact secrets/PII.
- **Risky edits:** `checkpoint` before large changes; `verify` a proposed file against its retained
  original to catch silent corruption; `rollback` to undo damage.
- **Stay on task:** keep the current goal in `anchor`.
- **End of session:** run `memory_crystallize` then `consolidate` to promote working notes into
  durable knowledge. Curate with `memory_pin` (keep), `memory_reinforce` (bump confidence),
  `memory_delete` (remove stale). On self-hosted servers use `registry_search` to browse
  the local pack registry.
- **Dashboard is observability-only:** the web UI shows what exists and progress --- you are the one
  who writes, curates, and maintains; humans watch.

Everything Cairn shows is lossless --- the full original is always one `expand` away.";

/// Write the Cairn rules into `id`'s native instruction file under `project`.
pub fn write_for(id: &str, project: &Path) -> Result<()> {
    let path = match id {
        "claude-code" => {
            managed(&project.join("CLAUDE.md"))?;
            project.join("CLAUDE.md")
        }
        "codex" => {
            // Codex CLI reads AGENTS.md from the project root (or `$CODEX_HOME/AGENTS.md`
            // for user-scope rules). We use the project root to stay scoped.
            managed(&project.join("AGENTS.md"))?;
            project.join("AGENTS.md")
        }
        "agents" | "opencode" => {
            managed(&project.join("AGENTS.md"))?;
            project.join("AGENTS.md")
        }
        other => bail!("unknown agent '{other}'. Supported: {}.", KNOWN.join(", ")),
    };
    println!("\u{2713} wrote Cairn rules: {}", path.display());
    Ok(())
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
        assert!(after_first.contains("prefer these tools"));
        assert!(after_first.contains("`recall`"));
    }

    #[test]
    fn codex_targets_agents_md_at_project_root() {
        let dir = tempfile::tempdir().unwrap();
        write_for("codex", dir.path()).unwrap();
        let p = dir.path().join("AGENTS.md");
        assert!(p.exists());
        assert!(fs::read_to_string(&p).unwrap().contains("Cairn"));
        assert!(fs::read_to_string(&p).unwrap().contains(BEGIN));
    }
}

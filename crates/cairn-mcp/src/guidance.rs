//! Single source of truth for "how to use Cairn," rendered into every surface an agent might
//! read it from (v0.8.0 Sprint 10, B6):
//!
//! - **MCP `instructions`** (primary): [`GUIDANCE_COMPACT`] goes straight into the `initialize`
//!   response's `instructions` field - every MCP-speaking agent surfaces this to the model at
//!   connect time automatically, with zero files written anywhere.
//! - **Claude Code skill** (rich, on-demand): [`skill_md`] renders `.claude/skills/cairn/SKILL.md`
//!   from [`GUIDANCE_FULL`] - loads only when the model's task looks relevant, so it's near-free
//!   ambient cost for a much richer playbook than a permanently-injected block could justify.
//! - **Managed blocks** (fallback + non-skill agents): [`claude_md_block`] is a ~4-line pointer
//!   at Cairn (Claude Code already gets the richer skill); [`agents_md_block`] is a fuller
//!   block for Codex/OpenCode, which have no skill system to fall back on.
//!
//! [`GUIDANCE_REV`] is stamped into every rendered artifact so `doctor`/`setup` can detect stale
//! content (written by an older binary) and rewrite it. A unit test (see the bottom of this
//! file) extracts every backticked tool-shaped token out of all four variants and asserts it's a
//! real name in [`crate::tool_defs`] - guidance text cannot silently drift from the tool
//! registry without turning that test red.

/// Bump whenever [`GUIDANCE_COMPACT`], [`GUIDANCE_FULL`], or a renderer's structure changes in a
/// way that makes previously-written artifacts stale. `doctor`/`setup` compare this against the
/// `cairn-guidance-rev: N` marker already on disk to decide whether to rewrite.
pub const GUIDANCE_REV: u32 = 1;

/// ~150-word tool playbook for the MCP `initialize` response's `instructions` field. Every
/// MCP-speaking client (Claude Code, Codex, OpenCode, anything future) surfaces this to the
/// model at connect time - this is the primary teaching mechanism; everything else is a
/// fallback for agents that don't relay `instructions`, or a richer on-demand supplement.
pub const GUIDANCE_COMPACT: &str = "\
You have Cairn: persistent memory, lean context, and edit safety - prefer it over your \
defaults. Reading: `read` a file instead of your built-in reader; unchanged re-reads are \
nearly free and large files can be read as just their structure. Recover any original with \
`expand`. Memory: `wakeup` at the start of a session to load prior context; `recall` or \
`search` to look something up; `remember` decisions, gotchas, and rationale as you go; \
`prefer` records standing user preferences. Before sharing or committing text, `sanitize` it \
to redact secrets. Before a risky edit, `checkpoint`; after, `verify` the result; `rollback` \
if something broke. Keep the current goal in `anchor`. At the end of a session, \
`memory_crystallize` then `consolidate` to promote working notes into durable memory.";

/// Full workflow playbook for the Claude Code skill (`SKILL.md` body) - richer than
/// [`GUIDANCE_COMPACT`] since a skill loads on-demand rather than sitting permanently in
/// context. Structured around the four things an agent actually needs to decide moment to
/// moment: what to do at session start/end, how to edit safely, how to keep token spend down,
/// and how scope isolation/promotion works.
pub const GUIDANCE_FULL: &str = "\
# Using Cairn

Cairn gives you persistent memory, lean context, and edit safety across sessions. Prefer its \
tools over your built-in equivalents wherever one exists.

## Session lifecycle

1. **Start:** call `wakeup` to load the highest-priority memories for this context - decisions, \
   gotchas, and standing preferences you'd otherwise have to rediscover. If your client injects \
   this automatically, treat it as already loaded rather than re-fetching it.
2. **During work:** call `recall` (fast, exact-ish) or `search` (hybrid BM25 + vector + graph) \
   whenever you need something from earlier - a decision, a convention, a past fix. Call \
   `proactive_recall` at the start of a turn to get automatically-surfaced related context.
3. **As you go:** `remember` decisions, gotchas, and rationale the moment you make them, not at \
   the end of the session - a memory written in the moment is more accurate than one \
   reconstructed from a summary. Use `prefer` for standing user preferences (coding style, \
   communication tone, tool choices) that should apply beyond this one session.
4. **End:** call `memory_crystallize` to fold working-tier notes from this session into a single \
   durable summary, then `consolidate` to promote memories across tiers based on how much \
   they've been reinforced.

## Edit safety loop

Before a risky or wide-reaching edit, call `checkpoint` to snapshot the current state. After \
making the change, call `verify` against that checkpoint to catch silent corruption or an edit \
that didn't do what you intended. If something broke, `rollback` restores the checkpointed \
state. `checkpoints` lists what's available to roll back to.

## Token hygiene

- `read` a file instead of your default file-reading tool. An unchanged re-read is nearly free; \
  after an edit, you get a diff instead of the whole file. For a large or unfamiliar file, read \
  it in signatures mode first (AST outline, bodies elided) before paying for the full text.
- `expand` recovers the full original content behind any handle Cairn returned in place of raw \
  text - nothing is ever permanently lost to compression.
- `compress` shrinks noisy tool output (build logs, test runs, command output) into a compact \
  view, keeping the original recoverable the same way.
- `assemble` builds a single context block under an explicit token budget when you need several \
  pieces of context at once without blowing past what you can afford to spend.

## Scope model

Memories live in one of three scopes: **Global** (visible everywhere), **Project** (visible only \
within the current project), or **Session** (visible only within the current session). Write at \
the narrowest scope that's actually correct - a fact specific to this repo belongs at Project \
scope, not Global. A memory that turns out to be broadly useful can be promoted to a wider scope \
via `memory_promote`; `memory_reinforce` bumps confidence on something that keeps proving \
useful, `memory_pin` keeps a memory from decaying out regardless of use, and `memory_delete` \
removes one that's actively wrong. `memory_timeline` and `memory_graph` show how memories \
relate to and derive from each other.

## Before you share or commit

Run `sanitize` on any text before it leaves your control (a commit message, a shared export, a \
support ticket) - it redacts secrets and PII while leaving the rest intact.

Everything Cairn shows you is lossless: whatever `read`/`compress`/`recall` hands back, the full \
original is always one `expand` away.";

/// Slim ~4-line pointer for `CLAUDE.md`'s managed block. Claude Code gets the full playbook via
/// the installed skill ([`skill_md`]), so the always-in-context managed block only needs to say
/// that Cairn exists and where the real instructions live - keeping its permanent token cost low.
pub fn claude_md_block() -> String {
    format!(
        "## Cairn\n\n\
         Cairn MCP is connected: persistent memory, lean context, and edit safety. The cairn \
         skill has the full playbook (session lifecycle, edit safety, token hygiene, scope \
         model) and loads automatically when relevant - see `.claude/skills/cairn/SKILL.md`.\n\n\
         <!-- cairn-guidance-rev: {GUIDANCE_REV} -->"
    )
}

/// Fuller managed block for `AGENTS.md` (Codex, OpenCode - neither has a skill system to fall
/// back on, so this carries more of [`GUIDANCE_FULL`]'s substance than [`claude_md_block`] does).
pub fn agents_md_block() -> String {
    format!(
        "## Cairn --- prefer these tools\n\n\
         You have **Cairn** (MCP server named cairn): persistent memory, lean context, and edit \
         safety. Use it.\n\n\
         - **Reading code/files:** use `read` instead of your default file read - unchanged \
         re-reads are nearly free, and signatures mode returns a large file as just its \
         structure (huge token saving). Recover any full original with `expand`.\n\
         - **Verbose tool output:** run `compress` to shrink cargo/build/log output into a \
         compact view, retaining the exact original (recover with `expand`).\n\
         - **Memory:** at the start of a task, `wakeup` auto-injects your highest-priority \
         memories so you never start cold. Use `recall` (quick) or `search` (hybrid \
         BM25+semantic) to find relevant past decisions and context; `remember` decisions, \
         gotchas, and rationale as you make them. Record standing user preferences with \
         `prefer`. Call `proactive_recall` at the start of each turn to get context \
         automatically injected. Use `assemble` to build a context block under a token budget.\n\
         - **Before sharing, logging, or committing text:** run `sanitize` to redact \
         secrets/PII.\n\
         - **Risky edits:** `checkpoint` before large changes; `verify` a proposed file against \
         its retained original to catch silent corruption; `rollback` to undo damage.\n\
         - **Stay on task:** keep the current goal in `anchor`.\n\
         - **End of session:** run `memory_crystallize` then `consolidate` to promote working \
         notes into durable knowledge. Curate with `memory_pin` (keep), `memory_reinforce` \
         (bump confidence), `memory_delete` (remove stale). On self-hosted servers use \
         `registry_search` to browse the local pack registry.\n\
         - **Dashboard is observability-only:** the web UI shows what exists and progress --- \
         you are the one who writes, curates, and maintains; humans watch.\n\n\
         Everything Cairn shows is lossless --- the full original is always one `expand` away.\n\n\
         <!-- cairn-guidance-rev: {GUIDANCE_REV} -->"
    )
}

/// Full `.claude/skills/cairn/SKILL.md` content, frontmatter included. `description` carries
/// the trigger phrasing Claude Code's skill loader matches a task against, so it lists the
/// concrete words a relevant task is likely to contain rather than describing the skill
/// abstractly.
pub fn skill_md() -> String {
    format!(
        "---\n\
         name: cairn\n\
         description: Cairn gives this agent persistent memory, lean context, and edit safety \
         across sessions. Use for tasks involving memory, recall, remembering decisions or \
         gotchas, checkpoints and rollback, context compression, reducing token usage, or \
         continuing work on a project across multiple sessions.\n\
         ---\n\
         <!-- cairn-guidance-rev: {GUIDANCE_REV} -->\n\n\
         {GUIDANCE_FULL}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn known_tool_names() -> HashSet<String> {
        crate::tool_defs()
            .as_array()
            .expect("tool_defs() returns a JSON array")
            .iter()
            .map(|t| {
                t["name"]
                    .as_str()
                    .expect("every tool def has a string name")
                    .to_string()
            })
            .collect()
    }

    /// Every substring between an odd/even pair of backticks in well-formed markdown-style
    /// text (no nested or escaped backticks, which none of our own guidance text uses),
    /// filtered to the shape a real tool name takes (lowercase + underscores only) - this
    /// naturally excludes backticked file names, header names, and code snippets without
    /// needing a manually-maintained skip-list.
    fn backticked_tool_like_tokens(text: &str) -> Vec<String> {
        text.split('`')
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, s)| s)
            .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn every_guidance_variant_only_names_real_tools() {
        let known = known_tool_names();
        let variants: [(&str, String); 5] = [
            ("GUIDANCE_COMPACT", GUIDANCE_COMPACT.to_string()),
            ("GUIDANCE_FULL", GUIDANCE_FULL.to_string()),
            ("claude_md_block", claude_md_block()),
            ("agents_md_block", agents_md_block()),
            ("skill_md", skill_md()),
        ];
        for (label, text) in variants {
            for tok in backticked_tool_like_tokens(&text) {
                assert!(
                    known.contains(&tok),
                    "{label} references `{tok}` via backticks, but that isn't a real tool name \
                     in tool_defs() - guidance text has drifted from the tool registry"
                );
            }
        }
    }

    #[test]
    fn rendered_artifacts_carry_the_current_rev_marker() {
        let marker = format!("cairn-guidance-rev: {GUIDANCE_REV}");
        assert!(claude_md_block().contains(&marker));
        assert!(agents_md_block().contains(&marker));
        assert!(skill_md().contains(&marker));
    }

    #[test]
    fn skill_md_frontmatter_is_well_formed() {
        let md = skill_md();
        assert!(md.starts_with("---\n"));
        let mut parts = md.splitn(3, "---\n");
        let _before = parts.next().unwrap();
        let frontmatter = parts.next().expect("closing --- delimiter present");
        assert!(frontmatter.contains("name: cairn"));
        assert!(frontmatter.contains("description:"));
        // Trigger phrasing a real task is likely to contain, per the description field's own
        // job of getting the skill to load when relevant.
        for trigger in ["memory", "recall", "remember", "checkpoint", "token"] {
            assert!(
                frontmatter.contains(trigger),
                "description should mention {trigger:?} as trigger phrasing"
            );
        }
    }

    #[test]
    fn claude_md_block_is_meaningfully_slimmer_than_the_full_agents_block() {
        // The whole point of the 3-layer split is that Claude Code's permanently-injected
        // block is much cheaper than the fallback block non-skill agents carry.
        assert!(claude_md_block().len() < agents_md_block().len() / 2);
    }
}

---
name: cairn
description: Cairn gives this agent persistent memory, lean context, and edit safety across sessions. Use for tasks involving memory, recall, remembering decisions or gotchas, checkpoints and rollback, context compression, reducing token usage, or continuing work on a project across multiple sessions.
---
<!-- cairn-guidance-rev: 1 -->

# Using Cairn

Cairn gives you persistent memory, lean context, and edit safety across sessions. Prefer its tools over your built-in equivalents wherever one exists.

## Session lifecycle

1. **Start:** call `wakeup` to load the highest-priority memories for this context - decisions, gotchas, and standing preferences you'd otherwise have to rediscover. If your client injects this automatically, treat it as already loaded rather than re-fetching it.
2. **During work:** call `recall` (fast, exact-ish) or `search` (hybrid BM25 + vector + graph) whenever you need something from earlier - a decision, a convention, a past fix. Call `proactive_recall` at the start of a turn to get automatically-surfaced related context.
3. **As you go:** `remember` decisions, gotchas, and rationale the moment you make them, not at the end of the session - a memory written in the moment is more accurate than one reconstructed from a summary. Pass a short title and the reasoning behind non-obvious memories - both show up in the dashboard's Memory Browser and make the difference between a scannable record and an opaque blob of content. Use `prefer` for standing user preferences (coding style, communication tone, tool choices) that should apply beyond this one session.
4. **End:** call `memory_crystallize` to fold working-tier notes from this session into a single durable summary, then `consolidate` to promote memories across tiers based on how much they've been reinforced.

## Edit safety loop

Before a risky or wide-reaching edit, call `checkpoint` to snapshot the current state. After making the change, call `verify` against that checkpoint to catch silent corruption or an edit that didn't do what you intended. If something broke, `rollback` restores the checkpointed state. `checkpoints` lists what's available to roll back to.

## Token hygiene

- `read` a file instead of your default file-reading tool. An unchanged re-read is nearly free; after an edit, you get a diff instead of the whole file. For a large or unfamiliar file, read it in signatures mode first (AST outline, bodies elided) before paying for the full text.
- `expand` recovers the full original content behind any handle Cairn returned in place of raw text - nothing is ever permanently lost to compression.
- `compress` shrinks noisy tool output (build logs, test runs, command output) into a compact view, keeping the original recoverable the same way.
- `assemble` builds a single context block under an explicit token budget when you need several pieces of context at once without blowing past what you can afford to spend.

## Scope model

Memories live in one of three scopes: **Global** (visible everywhere), **Project** (visible only within the current project), or **Session** (visible only within the current session). Write at the narrowest scope that's actually correct - a fact specific to this repo belongs at Project scope, not Global. A memory that turns out to be broadly useful can be promoted to a wider scope via `memory_promote`; `memory_reinforce` bumps confidence on something that keeps proving useful, `memory_pin` keeps a memory from decaying out regardless of use, and `memory_delete` removes one that's actively wrong. `memory_timeline` and `memory_graph` show how memories relate to and derive from each other.

## Before you share or commit

Run `sanitize` on any text before it leaves your control (a commit message, a shared export, a support ticket) - it redacts secrets and PII while leaving the rest intact.

Everything Cairn shows you is lossless: whatever `read`/`compress`/`recall` hands back, the full original is always one `expand` away.

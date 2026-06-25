For project conventions, dev commands, and architecture, see [AGENTS.md](AGENTS.md).

<!-- BEGIN CAIRN (managed by `cairn rules`) -->
## Cairn - prefer these tools

You have **Cairn** (MCP server `cairn`): persistent memory, lean context, and edit safety. Use it.

- **Reading code/files:** use `read` instead of your default file read - unchanged re-reads are
  nearly free, and `mode:"signatures"` returns a large file as just its structure (huge token
  saving). Recover any full original with `expand`.
- **Memory:** at the start of a task, `recall` (or `assemble`) relevant past decisions and context;
  `remember` decisions, gotchas, and rationale as you make them so the next session never starts
  cold. Record standing user preferences with `prefer`.
- **Before sharing, logging, or committing text:** run `sanitize` to redact secrets/PII.
- **Risky edits:** `checkpoint` before large changes; `verify` a proposed file against its retained
  original to catch silent corruption; `rollback` to undo damage.
- **Stay on task:** keep the current goal in `anchor`.
- **End of session:** run `consolidate` then `memory_crystallize` to promote working notes into
  durable knowledge. Curate with `memory_pin` (keep), `memory_reinforce` (bump confidence),
  `memory_delete` (remove stale). On self-hosted servers use `registry_search` / `registry_revoke`
  to manage the local pack registry.
- **Dashboard is observability-only:** the web UI shows what exists and progress - you are the one
  who writes, curates, and maintains; humans watch.

Everything Cairn shows is lossless - the full original is always one `expand` away.
<!-- END CAIRN -->

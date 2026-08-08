# Contract: CLI and Claude Code Hooks

**Feature**: `001-cairn-mvp`

The `cairn` binary is the developer's interface and the hook runtime. `cairnd` is the
daemon it talks to over a local socket (Unix domain socket; named pipe on Windows).

## CLI surface

| Command | Purpose |
|---|---|
| `cairn init` | Register the repository as a project; idempotent; starts the daemon |
| `cairn connect claude-code` | Install MCP server + hook configuration for this repository |
| `cairn disconnect claude-code` | Remove that configuration (FR-043) |
| `cairn status` | Project, branch, commit, working-tree state, **all** active sessions, integration mode, daemon state |
| `cairn session list\|start\|show\|end [--session <id>]` | Manual session control for agents without hooks. Several sessions may be active in one worktree, so `show` and `end` take `--session`; without it they resolve the single active session and return `ambiguous_session` naming the candidates when there is more than one (FR-010) |
| `cairn task list\|show\|new\|set-status` | Task management |
| `cairn memory add\|search\|show\|forget` | Memory management from the terminal (FR-021) |
| `cairn handoff show [--session <id>]` | Read a handoff (FR-035) |
| `cairn context [--budget N]` | Print the briefing a session would receive |
| `cairn privacy exclude\|list\|unexclude` | Path and command exclusions (FR-050) |
| `cairn delete observation\|memory\|session\|handoff <id> [--with-memories]` | Deletion with the per-entity semantics in `data-model.md`; `--with-memories` is the only way a session delete removes the memories it produced (FR-052) |
| `cairn link [--project <id>] [--create]\|unlink` | Opt a project into or out of server sync; `--create` makes a new shared project, `--project` joins an existing one, bare `cairn link` offers remote-based candidates to confirm (FR-053, FR-064) |
| `cairn auth login\|token set\|logout` | Server credentials |
| `cairn sync status\|now` | Outbox state, last success, permanent failures (FR-058) |
| `cairn daemon start\|stop\|status` | Daemon lifecycle |
| `cairn mcp` | Run the MCP server over stdio |
| `cairn hook <event>` | Hook entry point; reads the event payload on stdin |

Every command supports `--json` and emits a stable envelope:

```json
{ "ok": true, "data": { … } }
{ "ok": false, "error": { "code": "not_a_repository", "message": "…" } }
```

Exit codes: `0` success, `1` user error (bad input, not a repository), `2` Cairn
unavailable (daemon or storage). `cairn hook` is the exception — it always exits 0, see
below.

## Hook contract

`cairn connect claude-code` writes hook entries invoking `cairn hook <event>` for the six
events in FR-041. The event payload arrives on stdin as JSON and carries `session_id`,
`transcript_path`, `cwd`, and per-event metadata (D16). It carries no process identity and
no liveness signal.

| Event | Class | Deadline | What Cairn does |
|---|---|---|---|
| `SessionStart` | context | 1,500 ms | Resolve repository → ensure project → start or resume the session (`source` distinguishes startup / resume / compact) → return the briefing as context |
| `PostToolUse` | capture | 250 ms | Successful tool execution → the typed success observation (`file_read`, `file_changed`, `command_run`, `test_run`, `discovery`) |
| `PostToolUseFailure` | capture | 250 ms | Failed tool execution → an `error` observation built from the payload's failure data |
| `PreCompact` | boundary | 1,500 ms | Durable handoff, trigger `pre_compact`; session stays active |
| `Stop` | capture | 250 ms | **Turn checkpoint** — the main agent finished responding. Flush pending capture, set `last_turn_ended_at`. Session stays `active`; no handoff |
| `SessionEnd` | boundary | 1,500 ms | Session lifecycle end → finalize the session (`completed`), record the payload's `reason`, produce the `session_end` handoff |

Two distinctions the integration makes and Cairn honors (D16):

- **Success and failure are separate events.** `PostToolUse` is success only;
  `PostToolUseFailure` carries the failure data. Cairn never infers a failure from a
  `PostToolUse` payload.
- **`Stop` is not session end.** It fires when the agent finishes a turn; the developer can
  send another prompt and the same session continues. Only `SessionEnd` completes a
  session, and only `pre_compact`, `session_end`, and `recovered` produce durable handoffs.

Both deadlines are configuration, not constants; these are the defaults.

### Capture class — fire and forget (FR-015, SC-007)

`PostToolUse`, `PostToolUseFailure` and `Stop` need no answer from Cairn, so they never
wait for one. The hook parses the payload, writes it to the daemon socket, and returns
without reading a reply — it does not wait for storage. Exceeding 250 ms means the
observation is dropped, which is not a failure.

Because capture is fire-and-forget, an observation can still be in flight when a boundary
arrives. Handoff synthesis therefore waits, briefly and boundedly, for accepted captures to
land, so a handoff reports the whole session rather than most of it.

### Boundary class — bounded wait

`PreCompact` and `SessionEnd` produce a durable handoff. They happen once per compaction or
once per session, not once per tool call, so they are worth waiting for: dropping one on a
250 ms deadline would mean no handoff at the boundary at all (FR-032). They share the
1,500 ms deadline with `SessionStart`.

### Context class — bounded wait with a clean fallback (FR-027, FR-041, FR-046)

`SessionStart` is the one path that must return an answer, and it legitimately has work to
do: start the daemon if it is not running, open SQLite, shell out to Git, assemble and
budget a briefing. It gets 1,500 ms.

If the deadline passes, **the agent session still starts.** The hook returns no context,
or whatever partial briefing was ready, and reports the reduced-context state so the
developer and the agent both know memory was not delivered. Cairn never holds the session
open waiting.

### Fail-soft rule (both classes)

`cairn hook` **always exits 0**. If the daemon is unreachable, storage is locked, or the
payload is unrecognized, the hook drops the work, writes a line to Cairn's own log, and
returns success. It never writes to stderr in a way the agent surfaces as an error.

### Session liveness

Cairn cannot tell whether an agent is still running, because nothing in the integration says
so (D16). A session leaves `active` only on `SessionEnd`, an explicit end command, or daemon
start — never on `Stop` — where every still-`active` session is reconciled to `interrupted` with a handoff and
resumed if a later event arrives for it. `cairn status` reports idle time; idle time never
reclassifies a session.

### Capture filtering order

For every candidate observation, in this order, before anything is written:

1. Path or command matches a configured exclusion → drop entirely (FR-050).
2. Redact values matching known secret patterns (FR-049).
3. Extract structured fields; summarize content above the payload cap (FR-013).
4. Write the observation with the repository state at capture (FR-014) — **locally, and
   only locally**. No step of this pipeline can produce an outbox row (FR-055).

Steps 1 and 2 run in the daemon before the write, so nothing sensitive is ever persisted
even briefly.

## Manual mode

An MCP-compatible agent with no lifecycle hooks uses `cairn_session`, `cairn_context`,
`cairn_remember`, and `cairn_handoff` directly (FR-042). Capture is then limited to what
the agent reports: memory, decisions, and handoff generation still work; automatic
`file_read`/`command_run` observation does not. `cairn status` states which mode a
repository is in.

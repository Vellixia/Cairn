# Contract: Canonical Lifecycle

**Feature**: `002-agent-integration-platform`

Seven canonical events. Cairn's session, capture, and handoff behavior depends only on these;
no vendor event name, payload shape, or ordering assumption reaches the daemon (FR-112).

```
vendor event  ──▶  AgentAdapter::normalize  ──▶  CanonicalLifecycleEvent  ──▶  daemon request
```

Adapters run inside `cairn hook <event>`, which keeps Feature 001's two deadline classes and
its always-exit-0 rule.

## The events

| Canonical | Daemon request | Session after | Durable handoff |
|---|---|---|---|
| `session_opened` | `SessionStart` + `Context` | `active` | no |
| `tool_succeeded` | `Observe` (typed success) | unchanged | no |
| `tool_failed` | `Observe` (`error`) | unchanged | no |
| `agent_quiesced` | `TurnCheckpoint` | **`active`** | **no** |
| `context_compacting` | `HandoffGenerate(pre_compact)` | `active` | yes |
| `context_compacted` | `Context` (redeliver, optional) | `active` | **no** |
| `session_closed` | `SessionEnd` (sealed, see below) | `completed` | yes, durably |

`session_opened`, `context_compacting`, and `session_closed` are boundary/context class
(1,500 ms default). `tool_succeeded`, `tool_failed`, `agent_quiesced`, and
`context_compacted` are capture class (250 ms, fire-and-forget, a missed deadline is a dropped
event and not a failure).

Where an agent imposes a shorter deadline than Cairn's default, the agent's limit wins and is
never exceeded (FR-194).

## `agent_quiesced`

The agent has stopped working and is waiting. That is all it asserts.

- It does **not** mean the turn succeeded (FR-230).
- It does **not** mean the session ended (FR-116).
- It never carries an observation, and no success or failure observation may be synthesized
  from it (FR-231).
- Behavior is Feature 001's `Stop` handling unchanged: flush pending capture, record the
  checkpoint, session stays `active`, no handoff (FR-032).

The name is deliberately weaker than "turn completed" because OpenCode's `session.idle` — one
of the three signals mapped here — fires after an error as readily as after an answer (D21).

## `session_closed` — the sealed close

Two phases inside the daemon (D22):

1. **Seal**, synchronously, before the reply: one transaction sets the terminal status, the
   end reason, `ended_at`, and `handoff_pending`. No Git, no capture quiesce, no synthesis.
   The daemon answers here.
2. **Synthesize**, immediately after: quiesce in-flight captures, build the handoff, write it,
   clear `handoff_pending`.

**Progress is guaranteed while the daemon runs** (FR-240), not only across a restart:

- The synthesis task retries on failure with a bounded backoff.
- The daemon's existing maintenance tick — the one that already reaps idle sessions — sweeps
  any session whose `handoff_pending` is older than a few seconds and synthesizes it. No new
  scheduler.
- After a bounded number of attempts the session is reported as `handoff synthesis failed`
  with a redacted reason in `cairn status` and doctor's core section, and retried slowly. A
  terminal session never sits silently owing a handoff.
- Daemon-start reconciliation remains the backstop for the process dying between the phases —
  not the only retry path.

Two outcomes, both bounded (FR-240):

- **Recoverable**: the durable handoff exists within the documented interval — target under
  5 seconds at p99 on a running daemon.
- **Not currently recoverable**: a *named* failure condition surfaces within that same interval
  rather than silence. It stays retryable and actionable; it is not a terminal outcome.

Until the handoff exists, the completion guarantee is **not** reported as satisfied for that
boundary (FR-240 clause 4) — an acknowledged-but-owed boundary reads as owed, not complete.

**SC-128** measures the acknowledgment against the vendor's budget; **SC-136** separately
measures that the handoff lands without a restart and that a permanently failing synthesis
becomes a reported condition.

`cairn session end` from the command line keeps the Feature 001 behavior and waits for the
handoff — nothing holds a deadline over it. The request carries `wait_for_handoff`, true for
the CLI and false for hooks.

**Why**: Codex's session-end handler has a 1-second default and 3-second maximum budget
(D31); the Feature 001 path can exceed that, which would make the completion guarantee
unprovable rather than merely slow.

## Recovery is not completion

Cairn's inactivity timeout (`recover.rs`, two hours) and daemon-start reconciliation both
close sessions and write `recovered` handoffs. They are safety nets:

- they may backstop a `session_closed` that was missed or that failed;
- they **never** satisfy the completion guarantee and never contribute to a FULL
  classification (FR-229);
- a session closed by them is reported as closed by inactivity, not completed.

## Adapter mapping

### Claude Code

| Vendor event | Canonical | Notes |
|---|---|---|
| `SessionStart` | `session_opened` | `source` carried through; context returned via `hookSpecificOutput.additionalContext` |
| `PostToolUse` | `tool_succeeded` | |
| `PostToolUseFailure` | `tool_failed` | Built from the payload's `error` |
| `Stop` | `agent_quiesced` | `last_assistant_message` and `tool_calls` are **read for nothing and never persisted** (D35) |
| `PreCompact` | `context_compacting` | |
| `PostCompact` | `context_compacted` | New in Feature 002 |
| `SessionEnd` | `session_closed` | `reason` carried through |

Not registered: `PreToolUse`, `UserPromptSubmit`, `UserPromptExpansion`, `StopFailure`,
`PostToolBatch`, `PermissionRequest`, `PermissionDenied`, `SubagentStart`, `SubagentStop`,
`Setup`, `Notification`, `MessageDisplay`, and the file, config, task, team, workspace, and
MCP-elicitation events. Cairn registers only what its lifecycle needs (US2 #6).

### Codex

| Vendor event | Canonical | Notes |
|---|---|---|
| `SessionStart` | `session_opened` | `source` carried through |
| `PostToolUse` | `tool_succeeded` **or** `tool_failed` | Classified from `tool_response` (D23) |
| `Stop` | `agent_quiesced` | `last_assistant_message` never persisted |
| `PreCompact` | `context_compacting` | |
| `PostCompact` | `context_compacted` | |
| `SessionEnd` | `session_closed` | Sealed close; `reason` is currently the constant `"other"` |

Not registered: `PreToolUse`, `PermissionRequest`, `UserPromptSubmit`, `SubagentStart`,
`SubagentStop`.

**Failure classification order** (D23): explicit non-zero `exit_code` → explicit
`success: false` or `error` → otherwise success. An uninterpretable response yields the
success-shaped observation, never a fabricated error (FR-117).

**Trust**: none of these run until the user trusts them inside Codex. Until then the adapter
reports `installed_not_activated` and the level reflects what works (FR-209, D24).

### OpenCode

Delivered by a Cairn-owned plugin. Session lifecycle arrives on the `event` bus; tool
lifecycle on plugin hooks.

| Vendor signal | Canonical | Notes |
|---|---|---|
| `session.created`, or first activity for an unseen `sessionID` | `session_opened` | Context is delivered at the earliest supported point — the first `chat.message` of the session — and the report says so |
| `tool.execute.after` | `tool_succeeded` | Output carries no outcome flag |
| `tool.execute.after` whose output unambiguously establishes failure | `tool_failed` | **Conditional capability.** Emitted only where the output establishes it; an ambiguous output emits nothing rather than a fabricated failure (FR-117, FR-241) |
| `session.idle` | `agent_quiesced` | **Never** `session_closed` |
| `experimental.session.compacting` | `context_compacting` | Experimental hook; if absent, the capability is reported absent |
| `session.compacted` | `context_compacted` | |
| — | `session_closed` | **Not emitted — the one genuine absence.** OpenCode signals no session end at all, which is different from the conditional failure row above |

Not mapped: `session.deleted` (deleting a record is not completing work), `session.updated`,
`session.status`, `session.error`, `session.diff`, `message.*`, `file.*`, `permission.*`,
`todo.*`, `lsp.*`, `installation.*`, `server.*`.

Consequence: OpenCode sessions leave `active` only through Cairn's deterministic boundaries —
an explicit end, daemon-start reconciliation, or the inactivity timeout — which are recovery,
not completion. OpenCode is therefore below FULL (FR-210, SC-131).

**Conditional, not absent**: OpenCode's `lifecycle_tool_failure` is reported `conditional`
rather than `absent`, because provable failures do exist and discarding them would be as
dishonest as inventing them. SC-110 tests both halves: a payload that establishes failure
produces `tool_failed`; an ambiguous payload produces nothing.

### Generic MCP

Emits no lifecycle events. The capability profile reports every lifecycle capability absent
and the level is `MCP_ONLY` (FR-128 story, SC-107).

## Session identity

Every canonical event carries `agent_session_key`, taken from the vendor's own session
identifier: Claude `session_id`, Codex `session_id` (thread id), OpenCode `sessionID`. Feature
001's rule is unchanged — identity is the key, never the worktree — so two agents in one
checkout get two sessions and every event routes to the one that produced it (FR-010,
FR-118).

An adapter that cannot supply a stable identifier reports
`stable_session_identifier: absent` rather than sharing one session (US10 #6). None of the
three native adapters is in that position.

## Tool normalization

Feature 001's `classify_tool` and `is_test_command` are reused, extended with the vendor names
the new adapters see (FR-120, D36):

| Vendor name | Observation type |
|---|---|
| Claude `Read`, `NotebookRead` · OpenCode `read` | `file_read` |
| Claude `Edit`/`Write`/`MultiEdit`/`NotebookEdit` · Codex `apply_patch` · OpenCode `edit`/`write` | `file_changed` |
| Claude `Bash`/`BashOutput` · Codex `shell` · OpenCode `bash` | `command_run`, or `test_run` when the command matches a test marker |
| Anything else | `discovery` |

The raw vendor name is retained as bounded provenance: normalized to `[A-Za-z0-9_.-]`,
truncated to 64 characters, redacted like every other field, and never consulted by ranking,
handoff synthesis, or context assembly (FR-121, FR-122).

## Payload allow-list

Fields not listed here are read for routing and discarded. Nothing else is persisted
(FR-198, FR-199, D35).

| Retained | Never retained |
|---|---|
| session identifier, `cwd`, `source`, `trigger`, `reason` | `transcript_path` |
| `tool_name` (bounded provenance) | `last_assistant_message` |
| `tool_input.file_path`, `tool_input.command` (through exclusion → redaction → bound) | `tool_calls` |
| derived outcome and exit code from `tool_response` | `prompt` / `user_prompt` |
| bounded, redacted failure summary from `error` | `model`, `permission_mode`, `turn_id`, `agent_id`, `agent_type` |
| | OpenCode tool `output` text and `metadata` |

## Failure behavior

Unchanged from Feature 001 (FR-193): `cairn hook` always exits 0, a missed capture deadline
drops the event, an unreachable daemon logs locally and returns success, and nothing is
written to a surface the agent renders as an error. `session_opened` keeps the bounded
fallback — the agent session starts with reduced context rather than waiting (FR-195).

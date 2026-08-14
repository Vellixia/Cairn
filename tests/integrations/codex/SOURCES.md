# Sources

`developers.openai.com` is blocked by this environment's egress policy, so these shapes were
recorded **2026-08-11** from the `openai/codex` repository at `main`, as cited in
`specs/002-agent-integration-platform/research.md` §D31. Values are realistic but invented.

Each file records one vendor event, the payload, and what the adapter must make of it:
`expect` is the canonical event name, or `null` where the adapter must decline.

| File | Vendor event | Recorded from |
|---|---|---|
| `session_start.json`, `session_start_thread_id.json` | `SessionStart` | `codex-rs/hooks/src/schema.rs` — `session_id` is a stable thread identifier; `source` |
| `post_tool_use_success.json` | `PostToolUse` | `PostToolUseCommandInput` — `tool_name`, `tool_input`, `tool_response`, `tool_use_id` |
| `post_tool_use_failure.json` | `PostToolUse` | same shape; **there is no `PostToolUseFailure`**, so failure is established from `tool_response` |
| `post_tool_use_ambiguous.json`, `post_tool_use_no_response.json` | `PostToolUse` | same shape with no outcome evidence — the success-shaped observation, never an asserted failure |
| `stop.json` | `Stop` | `HookEventName` — turn-scoped events add `turn_id` |
| `pre_compact.json`, `post_compact.json` | `PreCompact`, `PostCompact` | `HookEventName` |
| `session_end.json` | `SessionEnd` | `codex-rs/hooks/src/events/session_end.rs` — `SESSION_END_REASON` is the constant `"other"` |
| `declined_*.json` | `PreToolUse`, `PermissionRequest`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop` | `HookEventName` — events Cairn does not register |

The session-end budget recorded in the same file — `SESSION_END_DEFAULT_TIMEOUT_SEC = 1`,
`SESSION_END_MAX_TIMEOUT_SEC = 3` — is what `tests/tests/perf_session_close.rs` measures
against, not something a payload can express.

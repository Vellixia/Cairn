# Sources

Payload shapes recorded **2026-08-11** from `code.claude.com/docs` (hooks, mcp, skills,
memory), as cited in `specs/002-agent-integration-platform/research.md` §D30. Values are
realistic but invented; no payload here came from a real session.

Each file records one vendor event, the payload, and what the adapter must make of it:
`expect` is the canonical event name, or `null` where the adapter must decline. A vendor
change shows up as a diff here rather than as a silent behavioral regression.

| File | Vendor event | Recorded from |
|---|---|---|
| `session_start.json`, `session_start_resume.json` | `SessionStart` | hooks reference — common fields plus `source` (startup \| resume \| clear \| compact \| fork) |
| `post_tool_use.json`, `post_tool_use_bash.json` | `PostToolUse` | hooks reference — `tool_name`, `tool_input`, `tool_use_id`, `tool_response` |
| `post_tool_use_failure.json` | `PostToolUseFailure` | hooks reference — the failure event carries `error` |
| `stop.json` | `Stop` | hooks reference — now carries `last_assistant_message` and `tool_calls` (D34) |
| `pre_compact.json` | `PreCompact` | hooks reference — `trigger` is manual \| auto |
| `post_compact.json` | `PostCompact` | hooks reference — added since Feature 001 |
| `session_end.json` | `SessionEnd` | hooks reference — `reason` |
| `declined_*.json` | `PreToolUse`, `UserPromptSubmit`, `SubagentStop`, `Notification`, `Setup` | hooks reference — events Cairn does not register (US2 #6) |

`declined_no_session_identity.json` is not a vendor shape: it is the same `SessionStart`
with the identifier removed, recording that an event Cairn cannot route is declined rather
than guessed (FR-118).

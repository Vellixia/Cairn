# Sources

`opencode.ai` is blocked by this environment's egress policy, so these shapes were recorded
**2026-08-11** from the `sst/opencode` repository at `dev`, as cited in
`specs/002-agent-integration-platform/research.md` §D32. Values are realistic but invented.

Each file records one vendor event, the payload, and what the adapter must make of it:
`expect` is the canonical event name, or `null` where the adapter must decline.

| File | Vendor event | Recorded from |
|---|---|---|
| `session_created.json` | `session.created` | `packages/sdk/js/src/gen/types.gen.ts` — session events reach plugins through the event bus; there is no session-start hook |
| `tool_execute_after_success.json` | `tool.execute.after` | `packages/plugin/src/index.ts` — `{tool, sessionID, callID, args}` and `{title, output, metadata}`, **no outcome flag** |
| `tool_execute_after_failure.json` | `tool.execute.after` | same shape, with output that unambiguously establishes failure |
| `tool_execute_after_ambiguous.json` | `tool.execute.after` | same shape, with output that only mentions the word "error" — the conditional capability's other half |
| `session_idle.json` | `session.idle` | event bus — carries only `{sessionID}` |
| `session_compacting.json` | `experimental.session.compacting` | `packages/plugin/src/index.ts` — experimental, hence a conditional capability |
| `session_compacted.json` | `session.compacted` | event bus |
| `declined_*.json` | `session.deleted`, `session.error`, `session.updated`, `session.status`, `session.diff`, `tool.execute.before`, `chat.message` | event bus and plugin hooks — **there is no session-ended event at all** (FR-115) |

`session.deleted` is the one most likely to be mistaken for a session end. Deleting a record
is not completing work, and mapping it would produce a handoff declaring finished work that
was in fact discarded.

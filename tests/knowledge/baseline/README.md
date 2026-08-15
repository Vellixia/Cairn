The **pre-feature** baseline, captured before migration 0005 exists.

Contents:

- `briefing.json` — the `cairn context --json` briefing object, `estimated_tokens`, `truncated` and
  `omitted_sections` for a project with no task, no warnings, no pins and no checkpoint
- `mcp_calls.json` — a recorded corpus of Feature 001/002 MCP `tools/call` requests and responses

These are what later tasks compare against for no-regression: `us10_min_safe_context::no_regression`
(metric 13, SC-308) and `mcp_backward_compatibility` (metric 36, SC-323).

**Do not regenerate these against a Feature 003 build.** A baseline recaptured after the change it
exists to detect proves nothing.

---
title: "Live IDE Verification Prompts"
type: guide
status: living
updated: 2026-07-01
---

# Live IDE Verification Prompts

Run these inside OpenCode and Claude Code in `D:\code\Cairn` to validate the live integration. Each prompt + expected result.

---

## OpenCode (in `D:\code\Cairn`)

### Prompt 1 - list MCP tools
> Use the cairn tool to list every MCP tool you have access to. Return the count.

**Expected:** Model returns `30` (matches our `tools/list` enumeration). The OpenCode tool palette shows `cairn` as a provider.

**What it proves:** MCP stdio handshake works inside the IDE; the 30 tools advertised match the 30 we exercised in Phase 2.

### Prompt 2 - round-trip
> Use the cairn MCP tools to: (1) remember the fact 'OpenCode is wired to cairn 0.6.2', (2) recall it back, (3) print the id and the recall score.

**Expected:**
- Step 1 returns a UUID (e.g. `a3fa25a8-...`).
- Step 2 returns that same memory with score ~1.0.
- The dashboard at `https://cairn.andresholivin.dev/` shows the new memory in the graph within a few seconds (poll `/api/memory/timeline` or refresh the UI).

**What it proves:** `cairn_remember` and `cairn_recall` work end-to-end through OpenCode's MCP transport.

### Prompt 3 - hook injection (session bootstrap)
> What is my current task anchor and what preferences does cairn know about me?

**Expected:** Without any prior prompt in this session, the model answers from `additionalContext` that `cairn hook SessionStart` injected at session open. Anchor = whatever was last set (empty after `phase2` anchor was overwritten by next session); preferences include `always use lean-ctx for reads` from `cairn profile`.

**What it proves:** Hook fires automatically when OpenCode opens the session; `cairn_session_start` content reaches the model's context.

### Prompt 4 - hook on every turn
> List the most relevant memories for the question: "should we use lean-ctx or cairn MCP for reading files?"

**Expected:** `cairn hook UserPromptSubmit` injects ranked memories; the model can name them by content and kind. Score 0.05+ on at least one item.

**What it proves:** `UserPromptSubmit` hook fires on every prompt; ranking works.

---

## Claude Code (in `D:\code\Cairn`)

### Prompt 1 - list MCP tools
> Same as OpenCode #1.

**Expected:** 30 tools. `cairn` appears under Claude's MCP tools in the session info.

### Prompt 2 - round-trip
> Same as OpenCode #2.

**Expected:** Identical behavior. UUID returned; dashboard updates within seconds.

### Prompt 3 - hook trace
> List the Cairn hook events that ran since this session started. Quote any `additionalContext` blocks you received.

**Expected:** Model can quote the `additionalContext` from `SessionStart` (preferences + wakeup) and from the first `UserPromptSubmit` for this prompt. If model cannot, Claude Code hook logs are at `~/.claude/logs/` - check there for raw stdout from `cairn hook`.

**What it proves:** All 4 hook events are firing (`SessionStart`, `UserPromptSubmit`, `PostToolUse`, `SessionEnd`); `SessionEnd` is silent by design.

### Prompt 4 - edit + verify
> Edit `README.md` to add the line "## Cairn verified" at the end. Then ask cairn to verify the edit.

**Expected:** `cairn hook PostToolUse` ran silently in background (no visible output); `cairn_verify` MCP tool returns a report showing the diff applied (or silent if no baseline to compare against in remote mode).

**What it proves:** PostToolUse fires on Edit/Write/MultiEdit; verify works.

---

## What "success" looks like across all 8 prompts

| # | Agent | Outcome |
|---|-------|---------|
| O1 | OpenCode | `30` tools |
| O2 | OpenCode | UUID + score |
| O3 | OpenCode | Anchor/preferences from injected context |
| O4 | OpenCode | Ranked memories from UserPromptSubmit hook |
| C1 | Claude Code | `30` tools |
| C2 | Claude Code | UUID + score |
| C3 | Claude Code | 2+ quoted `additionalContext` blocks |
| C4 | Claude Code | Edit applied + verify report |

If any of O2/O3/C2/C3 fail, the most likely cause is the v0.6.2 setup regression we found: **`cairn setup opencode` without `--token` strips `CAIRN_TOKEN` from `opencode.json`**. Workaround: pass `--token <jwt>` on every setup, OR ensure `CAIRN_TOKEN` is in the User env so `cairn mcp` inherits it.

---

## Manual checks if prompts fail

```powershell
# Verify env vars are visible to a fresh shell
$env:CAIRN_SERVER; $env:CAIRN_TOKEN.Substring(0,30)

# Verify the MCP config has the token (after a fresh setup)
Get-Content C:\Users\andre\.config\opencode\opencode.json |
  Select-String -Pattern 'CAIRN_TOKEN'

# Re-run setup with explicit token to force the field back in
& "$env:USERPROFILE\.local\bin\cairn.exe" setup opencode `
  --server https://cairn.andresholivin.dev `
  --token "<your-token-here>"

# Sanity-test the MCP stdio path yourself
$env:CAIRN_SERVER = "https://cairn.andresholivin.dev"
$env:CAIRN_TOKEN = "<your-token-here>"
& "$env:USERPROFILE\.local\bin\cairn.exe" mcp
# then on stdin: {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"manual","version":"0"}}}
# then: {"jsonrpc":"2.0","method":"notifications/initialized"}
# then: {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"profile","arguments":{}}}
```

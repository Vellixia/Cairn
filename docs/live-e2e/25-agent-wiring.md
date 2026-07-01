# 25 — Agent Wiring: Claude Code, Codex, OpenCode File Writes

> **Walked 2026-07-01. Result: 0/8 EXECUTED — configuration description only. Wiring setup exercised by `cairn setup` (doc 23); outside this walk's REST API + browser scope.**

## Objective
Verify the multi-agent config writes performed by `cairn setup [agent]`. Three agents, one row per agent. Cover: Claude Code (`mcpServers.cairn` in `~/.claude.json` global or `.mcp.json` project; hooks in `<scope>/.claude/settings.json` for `SessionStart` / `UserPromptSubmit` / `PostToolUse` with matcher `Edit|Write|MultiEdit|NotebookEdit|StrReplace` / `SessionEnd`), Codex (`[mcp_servers.cairn]` TOML in `~/.codex/config.toml` with `CAIRN_SERVER` / `CAIRN_TOKEN` env; `~/.codex/hooks.json` with matchers `startup|resume|clear|compact` / `apply_patch|Edit|Write` / `Stop`), OpenCode (`mcp.cairn` + `plugin` array entry in `~/.config/opencode/opencode.json`; the plugin at `~/.config/opencode/plugins/cairn.js` translates OpenCode events to hook events: `event({event})` for `session.created` -> `SessionStart`, `session.deleted|idle` -> `SessionEnd`, `message.part.updated` tool completed -> `PostToolUse`, `chat.message` -> `UserPromptSubmit`). The dedup logic in `crates/cairn-client/src/setup.rs:107-123` strips prior cairn entries before writing, so re-runs are safe. Cover: re-run `setup` -> no duplicate entries; `reset` -> clean removal.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] A valid `CAIRN_TOKEN` exported in the shell
- [ ] `cairn` binary on PATH
- [ ] Backup `~/.claude.json`, `~/.codex/config.toml`, `~/.codex/hooks.json`, `~/.config/opencode/opencode.json`, and the project `.mcp.json` so the writes are reversible
- [ ] At least one project `.mcp.json` exists at the walk's `cwd` for the Step 3 `--project` test
- [ ] `cairn setup --all` has been run once successfully (doc 23 Step 6) so the baseline is established; this doc exercises each agent individually + the dedup + the reset path

## Surface
CLI (filesystem side effects)

## Steps

### Step 1: `cairn setup claude-code` — Claude Code config write
**Do**: per `crates/cairn-client/src/setup.rs:820-889`, the writer produces a `mcpServers.cairn` block in `~/.claude.json` (or project `.mcp.json`) and a `hooks` block in `<scope>/.claude/settings.json`. The mcp entry is `command: "cairn"`, `args: ["mcp"]`, plus optional `env` for `CAIRN_SERVER` / `CAIRN_TOKEN`.
**Request**:
```bash
$env:CAIRN_SERVER = "http://127.0.0.1:7777"
$env:CAIRN_TOKEN = "<admin-bearer>"
cairn setup claude-code --server http://127.0.0.1:7777 --token <admin-bearer>
$ec = $LASTEXITCODE
# inspect
$claude = "$env:USERPROFILE\.claude.json"
Get-Content -Raw -LiteralPath $claude | ConvertFrom-Json | ConvertTo-Json -Depth 10 | Select-String -Pattern "mcpServers|cairn"
```
**Expected**:
- Exit code 0
- `~/.claude.json` contains `mcpServers.cairn` with `command: "cairn"` and `args: ["mcp"]`
- The `env` block (if present) carries `CAIRN_SERVER` and `CAIRN_TOKEN`
- `~/.claude/settings.json` (or project equivalent) contains four `hooks` entries: `SessionStart`, `UserPromptSubmit`, `PostToolUse` (matcher `Edit|Write|MultiEdit|NotebookEdit|StrReplace`), `SessionEnd`
- Each hook's `command` is `cairn hook <event>`
**Observed**:
- Exit code: ___
- mcpServers.cairn.command: ___
- mcpServers.cairn.args: ___
- hooks.SessionStart command: ___
- hooks.PostToolUse matcher: ___
**Result**: PASS / FAIL

### Step 2: `cairn setup claude-code` re-run — dedup
**Do**: re-run the same command. The dedup logic must not duplicate the entry.
**Request**:
```bash
cairn setup claude-code --server http://127.0.0.1:7777 --token <admin-bearer>
# count occurrences of the cairn mcp entry
$json = Get-Content -Raw -LiteralPath "$env:USERPROFILE\.claude.json" | ConvertFrom-Json
$count = ($json.mcpServers.PSObject.Properties.Name | Where-Object { $_ -eq "cairn" }).Count
Write-Output "cairn mcp entries: $count"
```
**Expected**:
- Exit code 0
- Exactly 1 `mcpServers.cairn` entry, not 2
- Exactly 1 hook entry per event (no duplicate `SessionStart` keys)
**Observed**:
- Exit code: ___
- mcp entries: ___
- SessionStart hook count: ___
**Result**: PASS / FAIL

### Step 3: `cairn setup claude-code --project` — project-scope write
**Do**: per the `--project` flag, the writer targets the project `.mcp.json` instead of `~/.claude.json`.
**Request**:
```bash
$env:CAIRN_PROJECT_ROOT = "D:\code\Cairn"
cairn setup claude-code --project --server http://127.0.0.1:7777 --token <admin-bearer>
$ec = $LASTEXITCODE
# inspect
Get-Content -Raw -LiteralPath "D:\code\Cairn\.mcp.json" | Select-String -Pattern "mcpServers|cairn"
```
**Expected**:
- Exit code 0
- `D:\code\Cairn\.mcp.json` contains the cairn mcp entry
- The global `~/.claude.json` is unchanged
**Observed**:
- Exit code: ___
- project .mcp.json contains cairn: ___
- global unchanged: ___
**Result**: PASS / FAIL

### Step 4: `cairn setup codex` — Codex TOML + hooks.json write
**Do**: per `crates/cairn-client/src/setup.rs:542-635`, the writer produces a `[mcp_servers.cairn]` block in `~/.codex/config.toml` (TOML) with `command`, `args`, and `env = { CAIRN_SERVER, CAIRN_TOKEN }`. It also produces a `~/.codex/hooks.json` with matchers `startup|resume|clear|compact` (SessionStart), no matcher (UserPromptSubmit), `apply_patch|Edit|Write` (PostToolUse), and `Stop` -> `SessionEnd`.
**Request**:
```bash
cairn setup codex --server http://127.0.0.1:7777 --token <admin-bearer>
$ec = $LASTEXITCODE
# inspect TOML
$toml = Get-Content -Raw -LiteralPath "$env:USERPROFILE\.codex\config.toml"
$toml | Select-String -Pattern "\[mcp_servers.cairn\]|command|args|CAIRN_SERVER|CAIRN_TOKEN"
# inspect hooks.json
$hooks = Get-Content -Raw -LiteralPath "$env:USERPROFILE\.codex\hooks.json" | ConvertFrom-Json
Write-Output ("SessionStart matcher: " + ($hooks.hooks | Where-Object { $_.event -eq "SessionStart" }).matcher)
Write-Output ("PostToolUse matcher: " + ($hooks.hooks | Where-Object { $_.event -eq "PostToolUse" }).matcher)
Write-Output ("Stop -> SessionEnd: " + (($hooks.hooks | Where-Object { $_.event -eq "Stop" }).command -match "SessionEnd"))
```
**Expected**:
- Exit code 0
- `~/.codex/config.toml` has `[mcp_servers.cairn]` with `command = "cairn"`, `args = ["mcp"]`, and the `env` block carrying `CAIRN_SERVER` and `CAIRN_TOKEN`
- `~/.codex/hooks.json` has 4 hook entries with the matchers described above
- The `Stop` hook's `command` ends in `cairn hook SessionEnd`
**Observed**:
- Exit code: ___
- [mcp_servers.cairn] present: ___
- CAIRN_SERVER env: ___
- hooks.json events: ___
- PostToolUse matcher: ___
**Result**: PASS / FAIL

### Step 5: `cairn setup codex` re-run — dedup
**Do**: re-run; no duplicate `[mcp_servers.cairn]` or hook entries.
**Request**:
```bash
cairn setup codex --server http://127.0.0.1:7777 --token <admin-bearer>
$toml = Get-Content -Raw -LiteralPath "$env:USERPROFILE\.codex\config.toml"
$count = ([regex]::Matches($toml, "\[mcp_servers\.cairn\]")).Count
Write-Output "[mcp_servers.cairn] count: $count"
```
**Expected**:
- Exit code 0
- `[mcp_servers.cairn]` count is exactly 1, not 2
- `hooks.json` event count is unchanged from Step 4
**Observed**:
- Exit code: ___
- [mcp_servers.cairn] count: ___
- hooks.json event count delta: ___
**Result**: PASS / FAIL

### Step 6: `cairn setup opencode` — OpenCode config + plugin write
**Do**: per `crates/cairn-client/src/setup.rs:317-523`, the writer produces a `mcp.cairn` block in `~/.config/opencode/opencode.json` with `command`, `args`, plus a `plugin` array entry pointing at `plugins/cairn.js`. The plugin JS file is generated by `write_opencode_plugin` (`setup.rs:445-523`) and registered via `register_opencode_plugin` (`setup.rs:417-440`).
**Request**:
```bash
cairn setup opencode --server http://127.0.0.1:7777 --token <admin-bearer>
$ec = $LASTEXITCODE
# inspect
$oc = "$env:USERPROFILE\.config\opencode\opencode.json"
Get-Content -Raw -LiteralPath $oc | Select-String -Pattern "\"cairn\"|\"plugin\""
$plugin = "$env:USERPROFILE\.config\opencode\plugins\cairn.js"
Write-Output ("plugin exists: " + (Test-Path -LiteralPath $plugin))
```
**Expected**:
- Exit code 0
- `opencode.json` has a `mcp.cairn` block with `command: "cairn"` and `args: ["mcp"]`
- `opencode.json.plugin` is a non-empty array; one entry points at `plugins/cairn.js`
- `plugins/cairn.js` exists and is a syntactically valid JS file
**Observed**:
- Exit code: ___
- mcp.cairn.command: ___
- plugin array length: ___
- plugin file exists: ___
**Result**: PASS / FAIL

### Step 7: OpenCode plugin — event translation
**Do**: per `setup.rs:487-504`, the plugin uses the OpenCode `Plugin` API to map:
- `event({event})` for `session.created` -> `SessionStart`; `session.deleted` / `session.idle` -> `SessionEnd`; `message.part.updated` with `part.type == "tool"` AND `state.status == "completed"` -> `PostToolUse`
- `chat.message(input, output)` for `UserPromptSubmit` (captures `output.parts[].text`)
**Request**:
```bash
$plugin = "$env:USERPROFILE\.config\opencode\plugins\cairn.js"
$content = Get-Content -Raw -LiteralPath $plugin
$checks = @{
  "imports @opencode-ai/plugin" = ($content -match "@opencode-ai/plugin")
  "event({event}) handler" = ($content -match "event\(\s*\{\s*event\s*\}\s*\)")
  "session.created -> SessionStart" = ($content -match "session\.created.*SessionStart")
  "session.deleted/idle -> SessionEnd" = ($content -match "session\.(deleted|idle).*SessionEnd")
  "message.part.updated tool -> PostToolUse" = ($content -match "PostToolUse")
  "chat.message handler" = ($content -match "chat\.message")
  "fires SessionStart via fireHook" = ($content -match "fireHook.*SessionStart")
}
$checks.GetEnumerator() | ForEach-Object { Write-Output ("{0}: {1}" -f $_.Key, $_.Value) }
```
**Expected**:
- Exit code 0 (this step does not invoke cairn, just inspects the file)
- All seven greps return `True`
**Observed**:
- @opencode-ai/plugin: ___
- event({event}): ___
- session.created -> SessionStart: ___
- session.deleted/idle -> SessionEnd: ___
- PostToolUse: ___
- chat.message: ___
- fireHook: ___
**Result**: PASS / FAIL

### Step 8: `cairn setup opencode` re-run — dedup
**Do**: re-run; the `plugin` array still has exactly one cairn entry, and the mcp.cairn block is unchanged.
**Request**:
```bash
cairn setup opencode --server http://127.0.0.1:7777 --token <admin-bearer>
$oc = "$env:USERPROFILE\.config\opencode\opencode.json"
$json = Get-Content -Raw -LiteralPath $oc | ConvertFrom-Json
$cairnPluginCount = ($json.plugin | Where-Object { $_ -match "cairn" }).Count
Write-Output "cairn plugin entries: $cairnPluginCount"
```
**Expected**:
- Exit code 0
- `cairn` mcp entry count is exactly 1
- `plugin` array has exactly 1 entry pointing at cairn.js (not 2)
- The plugin file's mtime did not change (idempotent write)
**Observed**:
- Exit code: ___
- mcp.cairn entries: ___
- cairn plugin entries: ___
- plugin mtime delta: ___
**Result**: PASS / FAIL

### Step 9: `cairn setup --all` re-run — global dedup across all three agents
**Do**: re-run with `--all` and confirm no agent's file gained a duplicate.
**Request**:
```bash
cairn setup --all --server http://127.0.0.1:7777 --token <admin-bearer>
# count across all three
$ca = (Get-Content -Raw -LiteralPath "$env:USERPROFILE\.claude.json" | ConvertFrom-Json)
$co = (Get-Content -Raw -LiteralPath "$env:USERPROFILE\.codex\config.toml")
$oc = (Get-Content -Raw -LiteralPath "$env:USERPROFILE\.config\opencode\opencode.json" | ConvertFrom-Json)
$caCount = ($ca.mcpServers.PSObject.Properties.Name | Where-Object { $_ -eq "cairn" }).Count
$coCount = ([regex]::Matches($co, "\[mcp_servers\.cairn\]")).Count
$ocCount = ($oc.mcp.PSObject.Properties.Name | Where-Object { $_ -eq "cairn" }).Count
Write-Output "claude=$caCount codex=$coCount opencode=$ocCount"
```
**Expected**:
- Exit code 0
- All three counts are exactly 1
**Observed**:
- Exit code: ___
- claude: ___
- codex: ___
- opencode: ___
**Result**: PASS / FAIL

### Step 10: `cairn reset --dry-run` — names every file to be cleaned
**Do**: per `crates/cairn-client/src/reset.rs:10-234`, the writer names every file it would touch. Run with `--dry-run` so nothing is actually removed.
**Request**:
```bash
cairn reset --dry-run
```
**Expected**:
- Exit code 0
- The output lists: `CLAUDE.md` / `AGENTS.md` (rules block), the project `.mcp.json`, `~/.claude.json`, `~/.codex/config.toml`, `~/.codex/hooks.json`, `~/.config/opencode/opencode.json`, and `~/.config/opencode/plugins/cairn.js`
- No file is actually removed (verify with `Test-Path`)
**Observed**:
- Exit code: ___
- Files named: ___
- All four config files still present: ___
- plugin still present: ___
**Result**: PASS / FAIL

### Step 11: `cairn reset` — actual removal
**Do**: the real reset. This is destructive; the precondition says you have backups.
**Request**:
```bash
cairn reset
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
$ca = (Get-Content -Raw -LiteralPath "$env:USERPROFILE\.claude.json" | ConvertFrom-Json)
$caHasCairn = $ca.mcpServers.PSObject.Properties.Name -contains "cairn"
$coHasCairn = (Test-Path -LiteralPath "$env:USERPROFILE\.codex\config.toml") -and ((Get-Content -Raw -LiteralPath "$env:USERPROFILE\.codex\config.toml") -match "\[mcp_servers\.cairn\]")
$ocHasCairn = (Test-Path -LiteralPath "$env:USERPROFILE\.config\opencode\opencode.json") -and ((Get-Content -Raw -LiteralPath "$env:USERPROFILE\.config\opencode\opencode.json") -match "\"cairn\"")
$pluginExists = Test-Path -LiteralPath "$env:USERPROFILE\.config\opencode\plugins\cairn.js"
Write-Output "claude.cairn=$caHasCairn codex.cairn=$coHasCairn opencode.cairn=$ocHasCairn plugin=$pluginExists"
```
**Expected**:
- Exit code 0
- All four cairn entries are gone
- The plugin file is deleted
- Foreign config (other agents, unrelated hooks) is preserved
**Observed**:
- Exit code: ___
- claude.cairn present (must be false): ___
- codex.cairn present (must be false): ___
- opencode.cairn present (must be false): ___
- plugin exists (must be false): ___
**Result**: PASS / FAIL

### Step 12: `cairn setup --all` after reset — full restore
**Do**: prove the round-trip is clean. After reset, re-run setup --all and confirm all four artifacts are back.
**Request**:
```bash
cairn setup --all --server http://127.0.0.1:7777 --token <admin-bearer>
$ca = (Get-Content -Raw -LiteralPath "$env:USERPROFILE\.claude.json" | ConvertFrom-Json)
$caHasCairn = $ca.mcpServers.PSObject.Properties.Name -contains "cairn"
$coHasCairn = ((Get-Content -Raw -LiteralPath "$env:USERPROFILE\.codex\config.toml") -match "\[mcp_servers\.cairn\]")
$ocHasCairn = ((Get-Content -Raw -LiteralPath "$env:USERPROFILE\.config\opencode\opencode.json") -match "\"cairn\"")
$pluginExists = Test-Path -LiteralPath "$env:USERPROFILE\.config\opencode\plugins\cairn.js"
Write-Output "claude.cairn=$caHasCairn codex.cairn=$coHasCairn opencode.cairn=$ocHasCairn plugin=$pluginExists"
```
**Expected**:
- Exit code 0
- All four artifacts are back
- No duplicates introduced (the dedup logic keeps it at exactly 1 each)
**Observed**:
- Exit code: ___
- claude.cairn restored: ___
- codex.cairn restored: ___
- opencode.cairn restored: ___
- plugin restored: ___
**Result**: PASS / FAIL

## DB Verification
- Not directly applicable. The agent-wiring writes are filesystem-only; they do not touch HelixDB.
- For a secondary check, after Step 9: `GET /api/stats` should report the same `memories` and `checkpoints` counts as before — the wiring writes files, not data.

## UI Verification
- N/A. The CLI is a host-side tool. The only browser consequence is the dashboard's health pill; it should remain `ok` because the server is untouched. Confirm at `/?nocache=25-12` that `list_console_messages types=["error"]` is empty.

## Evidence
- Output captures of Steps 1, 4, 6, 10, 11, 12
- File hashes of the four config files before and after `reset` (proves the cleanup)
- The plugin-file grep results from Step 7
- Screenshot: `docs/live-e2e/screenshots/25-agent-wiring/dashboard.png` (proves the server is still healthy after the wiring churn)

## Known gaps
- The OpenCode plugin (Step 7) is generated by the CLI on every setup; it is the only agent where the binary is invoked through a JS shim rather than a direct `cairn hook <event>` call. This is by design (OpenCode's plugin API is JS-only) but worth noting when debugging the install.

## Findings
(none — not executed)

## Walked result
- **Steps walked:** 0/8 — all steps catalogued, none executed (wiring description only)
- **Screenshots:** none
- **Note:** Wiring files are exercised by `cairn setup` (doc 23). This doc documents the expected file shapes for manual inspection.

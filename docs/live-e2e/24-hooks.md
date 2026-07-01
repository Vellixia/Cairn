# 24 — Hooks: SessionStart, UserPromptSubmit, SessionEnd, PostToolUse

> **Walked 2026-07-01. Result: 0/8 EXECUTED — hook walk deferred (requires CLI binary + agent session, outside this walk's REST API + browser scope).**

## Objective
Verify the lifecycle hook layer invoked as `cairn hook <event>` (`crates/cairn-client/src/hook.rs:14-175`). Cover the four event types the agent surface translates to: `SessionStart` (reads `/api/guard/anchor` + `/api/profile` + `/api/memory/wakeup?limit=12`; emits one `hookSpecificOutput.additionalContext` block on stdout), `UserPromptSubmit` (records the prompt via `POST /api/memory` with `kind=note`, `tier=episodic`, `importance=0.3`; opt-in context injection via `CAIRN_INJECT_CONTEXT=true|1|yes|on` enables `POST /api/context/assemble?q=&budget=1200`), `SessionEnd` (`POST /api/memory/consolidate`), `PostToolUse` (no remote action). The hook never breaks the agent: any error path prints to stderr and exits 0 (`hook.rs:14-19`). Wire-protocol details for each agent (Claude / Codex / OpenCode) are in doc 25.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] A valid `CAIRN_TOKEN` exported in the shell (see doc 23 for the mint flow)
- [ ] `cairn` binary on PATH (`cairn --version` succeeds)
- [ ] No leftover `HOOK-2026-07-01-*` memories in HelixDB from prior walks (or capture baseline)
- [ ] For Steps 4-6, an active `cairn setup --all` install exists (doc 23 Step 6) so the agent's native config points at this binary; for the stdin-only tests (Steps 1-3, 7-8) the env-vars alone are sufficient

## Surface
CLI (stdio JSON-RPC)

## Steps

### Step 1: `cairn hook SessionStart` — happy path
**Do**: pipe a minimal Claude-style payload to stdin. Per `crates/cairn-client/src/hook.rs:65-107` the handler reads anchor + profile + wakeup?limit=12 and emits one `hookSpecificOutput.additionalContext` block.
**Request**:
```bash
$env:CAIRN_SERVER = "http://127.0.0.1:7777"
$env:CAIRN_TOKEN = "<admin-bearer>"
echo '{"session_id":"HOOK-2026-07-01-session-a1b2c3","cwd":"D:\\code\\Cairn","hook_event_name":"SessionStart"}' | cairn hook SessionStart
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
```
**Expected**:
- Exit code 0
- stdout is a single JSON object whose top-level key is `hookSpecificOutput.additionalContext` and whose value is a string containing the rendered context (anchor + preferences + wakeup memories)
- stderr is empty (no errors)
**Observed**:
- Exit code: ___
- additionalContext present: ___
- additionalContext length: ___
- stderr: ___
**Result**: PASS / FAIL

### Step 2: `cairn hook SessionStart` — no env vars (still exit 0)
**Do**: per `hook.rs:26-30`, if `CAIRN_SERVER` or `CAIRN_TOKEN` is unset, the hook prints a notice to stderr and exits 0 — the agent is never blocked.
**Request**:
```bash
Remove-Item Env:CAIRN_SERVER -ErrorAction SilentlyContinue
Remove-Item Env:CAIRN_TOKEN -ErrorAction SilentlyContinue
echo '{"session_id":"HOOK-2026-07-01-empty-env"}' | cairn hook SessionStart
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
```
**Expected**:
- Exit code 0 (the agent runs)
- stdout is `{}` or empty
- stderr contains a one-line notice that env vars are missing
**Observed**:
- Exit code: ___
- stdout: ___
- stderr: ___
**Result**: PASS / FAIL

### Step 3: `cairn hook SessionStart` — wakeup includes non-preference memories only
**Do**: the wakeup call inside SessionStart filters to non-preference memories. A `preference`-kind memory that already exists in the store must not appear in the additionalContext block.
**Request**:
```bash
# pre-seed: create a preference and a fact
$env:CAIRN_SERVER = "http://127.0.0.1:7777"
$env:CAIRN_TOKEN = "<admin-bearer>"
Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:7777/api/profile" -Headers @{ Cookie = "cairn_session=..." } -ContentType "application/json" -Body '{"rule":"HOOK-2026-07-01-pref-test: do the test"}'
Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:7777/api/memory" -Headers @{ Cookie = "cairn_session=..." } -ContentType "application/json" -Body '{"content":"HOOK-2026-07-01-fact-test","kind":"fact","tier":"episodic","importance":0.9}'
echo '{"session_id":"HOOK-2026-07-01-filter"}' | cairn hook SessionStart | Tee-Object -FilePath /tmp/opencode/hook-step3.json
```
**Expected**:
- Exit code 0
- The fact (`HOOK-2026-07-01-fact-test`) appears in the `additionalContext` block
- The preference (`HOOK-2026-07-01-pref-test`) does NOT appear in the wakeup list (it is surfaced separately in the preferences block from `/api/profile`, not the wakeup)
**Observed**:
- fact in additionalContext: ___
- preference in additionalContext (must be no): ___
**Result**: PASS / FAIL

### Step 4: `cairn hook UserPromptSubmit` — records the prompt to memory
**Do**: per `hook.rs:108-144`, the handler POSTs to `/api/memory` with `kind=note`, `tier=episodic`, `importance=0.3` for every prompt.
**Request**:
```bash
echo '{"session_id":"HOOK-2026-07-01-prompt","prompt":"HOOK-2026-07-01-prompt: build the live-e2e doc"}' | cairn hook UserPromptSubmit
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
# confirm via recall
Invoke-RestMethod -Uri "http://127.0.0.1:7777/api/memory/recall?q=HOOK-2026-07-01-prompt&limit=5" -Headers @{ Cookie = "cairn_session=..." }
```
**Expected**:
- Exit code 0
- stdout is `{}` (or a small JSON object with no `additionalContext`, since injection is off by default)
- stderr empty
- A new memory exists with `content: "HOOK-2026-07-01-prompt: build the live-e2e doc"`, `kind: "note"`, `tier: "episodic"`, `importance: 0.3`
**Observed**:
- Exit code: ___
- recall hit count: ___
- memory kind: ___
- memory tier: ___
- memory importance: ___
**Result**: PASS / FAIL

### Step 5: `cairn hook UserPromptSubmit` with `CAIRN_INJECT_CONTEXT=true` — opt-in injection
**Do**: per `hook.rs:170-175`, setting `CAIRN_INJECT_CONTEXT=true|1|yes|on` enables the `POST /api/context/assemble?q=&budget=1200` call and emits its `context` field as `additionalContext` if non-empty.
**Request**:
```bash
$env:CAIRN_INJECT_CONTEXT = "true"
echo '{"session_id":"HOOK-2026-07-01-inject","prompt":"HOOK-2026-07-01-inject: what is the cap on sync pull?"}' | cairn hook UserPromptSubmit | Tee-Object -FilePath /tmp/opencode/hook-step5.json
```
**Expected**:
- Exit code 0
- stdout is a JSON object whose `hookSpecificOutput.additionalContext` field is a non-empty string assembled from the prompt
- The string references relevant memories (the sync cap is 500 — see doc 21)
- The `CAIRN_INJECT_CONTEXT` env var is read fresh on every call (so the toggle is per-process)
**Observed**:
- Exit code: ___
- additionalContext present: ___
- additionalContext length: ___
**Result**: PASS / FAIL

### Step 6: `cairn hook SessionEnd` — calls consolidate
**Do**: per `hook.rs:145-147`, the handler POSTs `/api/memory/consolidate`.
**Request**:
```bash
$baseline = (Invoke-RestMethod -Uri "http://127.0.0.1:7777/api/stats" -Headers @{ Cookie = "cairn_session=..." }).promoted
echo '{"session_id":"HOOK-2026-07-01-end"}' | cairn hook SessionEnd
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
$after = (Invoke-RestMethod -Uri "http://127.0.0.1:7777/api/stats" -Headers @{ Cookie = "cairn_session=..." }).promoted
Write-Output "promoted before=$baseline after=$after"
```
**Expected**:
- Exit code 0
- stdout is a small JSON (the consolidate response: `{"promoted": N}`)
- stderr empty
- `promoted` is a non-negative integer (could be 0 if no memories were eligible for promotion; that's fine)
**Observed**:
- Exit code: ___
- stdout: ___
- promoted delta: ___
**Result**: PASS / FAIL

### Step 7: `cairn hook PostToolUse` — no remote action
**Do**: per `hook.rs:148-151`, the handler falls through to a no-op. It must still exit 0 and produce no network traffic.
**Request**:
```bash
echo '{"session_id":"HOOK-2026-07-01-posttool","tool_name":"Edit","tool_input":{"file_path":"docs/live-e2e/24-hooks.md","new_string":"x"}}' | cairn hook PostToolUse
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
```
**Expected**:
- Exit code 0
- stdout is `{}` (or empty)
- No POST to `/api/memory` (verify by checking the metrics counter `savings.calls` did not bump)
**Observed**:
- Exit code: ___
- stdout: ___
**Result**: PASS / FAIL

### Step 8: `cairn hook <unknown>` — unrecognized event is a no-op
**Do**: per `hook.rs:148-151`, the default arm is `_ => {}`. An unknown event name must still exit 0.
**Request**:
```bash
echo '{"session_id":"HOOK-2026-07-01-unknown"}' | cairn hook SomeOtherEvent
$ec = $LASTEXITCODE
Write-Output "exit=$ec"
```
**Expected**:
- Exit code 0
- stdout is `{}` (or empty)
- stderr may contain a debug-level note but no error
**Observed**:
- Exit code: ___
- stdout: ___
- stderr: ___
**Result**: PASS / FAIL

### Step 9: `cairn hook SessionStart` — anchor + preference text present
**Do**: pre-set an anchor and a preference; run SessionStart; confirm both appear in the additionalContext block.
**Request**:
```bash
# pre-seed: set anchor and a known preference
Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:7777/api/guard/anchor" -Headers @{ Cookie = "cairn_session=..." } -ContentType "application/json" -Body '{"goal":"HOOK-2026-07-01-anchor: walk the hook doc"}'
Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:7777/api/profile" -Headers @{ Cookie = "cairn_session=..." } -ContentType "application/json" -Body '{"rule":"HOOK-2026-07-01-style: terse, code-first"}'
echo '{"session_id":"HOOK-2026-07-01-mixed"}' | cairn hook SessionStart | Out-File -Encoding utf8 /tmp/opencode/hook-step9.json
```
**Expected**:
- Exit code 0
- The `additionalContext` block contains the anchor text (`HOOK-2026-07-01-anchor: walk the hook doc`) and the preference (`HOOK-2026-07-01-style: terse, code-first`)
**Observed**:
- anchor in additionalContext: ___
- preference in additionalContext: ___
**Result**: PASS / FAIL

### Step 10: `cairn hook SessionStart` — never blocks the agent
**Do**: with an unreachable server, the hook must still exit 0 (proving the best-effort contract).
**Request**:
```bash
$env:CAIRN_SERVER = "http://127.0.0.1:1"  # bad
$env:CAIRN_TOKEN = "<any>"
$start = Get-Date
echo '{"session_id":"HOOK-2026-07-01-timeout"}' | cairn hook SessionStart
$ec = $LASTEXITCODE
$elapsed = (Get-Date) - $start
Write-Output "exit=$ec elapsed=$($elapsed.TotalSeconds)s"
```
**Expected**:
- Exit code 0 (the agent is never blocked)
- The elapsed time is small (< 10s; the hook uses a short connect timeout)
- stderr contains an error line but the process exits 0
**Observed**:
- Exit code: ___
- elapsed seconds: ___
- stderr: ___
**Result**: PASS / FAIL

### Step 11: Plugin-bridge sanity (OpenCode)
**Do**: confirm the OpenCode plugin file at `~/.config/opencode/plugins/cairn.js` (written by `cairn setup opencode`, per `crates/cairn-client/src/setup.rs:445-523`) imports `@opencode-ai/plugin` and registers `event` and `chat.message` hooks. Read the file and check for those symbols.
**Request**:
```bash
$plugin = "$env:USERPROFILE\.config\opencode\plugins\cairn.js"
if (Test-Path -LiteralPath $plugin) {
  $content = Get-Content -Raw -LiteralPath $plugin
  Write-Output ("has @opencode-ai/plugin: " + ($content -match "@opencode-ai/plugin"))
  Write-Output ("has event({event}): " + ($content -match "event\(\s*\{\s*event\s*\}\s*\)"))
  Write-Output ("has chat.message: " + ($content -match "chat\.message"))
  Write-Output ("has session.created -> SessionStart: " + ($content -match "session\.created.*SessionStart"))
  Write-Output ("has session.deleted/idle -> SessionEnd: " + ($content -match "session\.(deleted|idle).*SessionEnd"))
  Write-Output ("has tool completed -> PostToolUse: " + ($content -match "PostToolUse"))
}
```
**Expected**:
- All five greps return `True`
- The plugin translates OpenCode events to the four hook events the binary handles
**Observed**:
- @opencode-ai/plugin: ___
- event({event}): ___
- chat.message: ___
- session.created -> SessionStart: ___
- session.deleted/idle -> SessionEnd: ___
- tool completed -> PostToolUse: ___
**Result**: PASS / FAIL

## DB Verification
- Not directly applicable. The hook is a client-side dispatcher that calls server APIs; it does not write to HelixDB itself.
- For Step 4: `GET /api/memory/recall?q=HOOK-2026-07-01-prompt&limit=5` confirms the prompt was recorded.
- For Step 6: `GET /api/memory/consolidate` (POST) followed by `GET /api/memory/timeline` shows any promotions.

## UI Verification
- N/A. The hook is stdio JSON-RPC, not browser. The only UI consequence is the dashboard's topbar; it should remain `ok` because the hook is best-effort. Confirm at `/?nocache=24-11` that `list_console_messages types=["error"]` is empty.

## Evidence
- Output captures of Steps 1, 4, 5, 6, 7, 8
- The `additionalContext` blocks from Steps 1, 5, 9
- The recall responses from Step 4
- The plugin-file grep results from Step 11
- Screenshot: `docs/live-e2e/screenshots/24-hooks/dashboard.png` (proves the server is still healthy after the hook storm)

## Known gaps
- The OpenCode plugin (Step 11) is a small JS shim generated by `setup.rs:445-523`. It is the only agent where the binary is invoked indirectly through a plugin; Claude and Codex invoke `cairn hook <event>` directly via their native hook config (see doc 25).

## Findings
(none — not executed)

## Walked result
- **Steps walked:** 0/8 — all steps catalogued, none executed (hook walk deferred)
- **Screenshots:** none
- **Note:** Hook walk requires `cairn hook` CLI binary and a live agent session; deferred to dedicated CLI run.

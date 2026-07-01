# 23 — CLI: `cairn` Subcommands (Doctor, Onboard, Setup, Status, Reset, Upgrade)

> **Walked 2026-07-01. Result: 0/15 EXECUTED — CLI walk deferred. All 15 steps catalogued but not executed. This walk focuses on REST API + browser surfaces; the CLI binary is exercised in a separate dedicated run.**

## Objective
Verify the `cairn` host CLI tarball binary (`crates/cairn-client/src/main.rs:40-172`). Cover 7 of the 8 subcommands (the 8th, `mcp`, is exercised in doc 24-hooks.md because the stdio MCP server is a special case): `doctor` (4 checks, exit 0/1), `onboard` (re-onboard detection, spawns `setup --all`), `setup [agent|--all] [--server|--token|--project]` (token validate against `/api/memory/wakeup?limit=1`, idempotent file writes to `~/.claude.json` / `.mcp.json` / `~/.codex/{config.toml,hooks.json}` / `~/.config/opencode/{opencode.json,plugins/cairn.js}`; aliases `claude-code|claude|claudecode|cc|codex|opencode|oc`), `status` (decode JWT, list agents), `reset --dry-run` (reports the writes it would make), `upgrade --check` (GitHub release probe). `hook` is covered in doc 24-hooks.md.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh (curl it: `curl -sS -c /tmp/opencode/walk-cookies.txt -d 'username=admin&password=AuditPass2026!' http://127.0.0.1:7777/api/auth/login`)
- [ ] The latest `cairn` tarball is installed at `~/.cargo/bin/cairn` (or `$env:USERPROFILE\.cargo\bin\cairn.exe` on Windows); run `cairn --version` to confirm
- [ ] Backup the existing `~/.claude.json`, `~/.codex/{config.toml,hooks.json}`, `~/.config/opencode/opencode.json`, and any project `.mcp.json` so `reset --dry-run` is reversible (`Copy-Item` them first)
- [ ] `CAIRN_SERVER=http://127.0.0.1:7777` and a valid `CAIRN_TOKEN=<admin-bearer>` exported in the shell for the duration of the doc (mint the token via `POST /api/devices/tokens` with `scope: admin`)

## Surface
CLI

## Steps

### Step 1: `cairn doctor`
**Do**: run the four health checks. Per `crates/cairn-client/src/doctor.rs:56-91`: data dir writable, remote `/api/memory/wakeup` reachable, agents detected, config health.
**Request**:
```bash
$env:CAIRN_SERVER = "http://127.0.0.1:7777"
$env:CAIRN_TOKEN = "<admin-bearer>"
cairn doctor
```
**Expected**:
- Exit code 0
- Human-readable output, one line per check, with `[ok]` / `[warn]` / `[fail]` markers
- `Remote /api/memory/wakeup reachable: ok` is the most important line
- `Agents detected: <count> (claude-code, codex, opencode)` — at least 0; lists which agent config files exist
**Observed**:
- Exit code: ___
- Output: ___
**Result**: PASS / FAIL

### Step 2: `cairn doctor --json`
**Do**: machine-readable variant. The `--json` flag emits the same checks as a JSON object.
**Request**:
```bash
cairn doctor --json
```
**Expected**:
- Exit code 0
- stdout is a single JSON object with one key per check (`data_dir`, `remote`, `agents`, `config`); each value has `status: "ok"|"warn"|"fail"` and a `detail` field
- The `agents` key lists the detected agent names
**Observed**:
- Exit code: ___
- JSON shape: ___
**Result**: PASS / FAIL

### Step 3: `cairn doctor --fix` (data dir missing)
**Do**: with `CAIRN_DATA_DIR` pointing at a missing path, `cairn doctor --fix` should create the directory and report `ok` on the data-dir check.
**Request**:
```bash
$env:CAIRN_DATA_DIR = "C:\Users\andre\AppData\Local\Temp\cairn-test-2026-07-01"
Remove-Item -LiteralPath $env:CAIRN_DATA_DIR -Recurse -Force -ErrorAction SilentlyContinue
cairn doctor --fix
Test-Path -LiteralPath $env:CAIRN_DATA_DIR
```
**Expected**:
- Exit code 0
- Data dir check transitions from `fail` to `ok`
- The directory exists after the call
**Observed**:
- Exit code: ___
- Data dir exists after: ___
**Result**: PASS / FAIL

### Step 4: `cairn status`
**Do**: decode the JWT, verify it against `/api/memory/wakeup?limit=1`, list the detected agents. Per `crates/cairn-client/src/status.rs:22-91`.
**Request**:
```bash
cairn status
```
**Expected**:
- Exit code 0
- Output shows: server URL, the JWT `sub` / `exp` / `scope` decoded from the payload, the agent list, and a final `Server: ok` line proving the wakeup round-trip succeeded
- Agent list mirrors what `doctor` detected
**Observed**:
- Exit code: ___
- Decoded sub: ___
- Decoded exp: ___
- Agents: ___
**Result**: PASS / FAIL

### Step 5: `cairn status --json`
**Do**: machine-readable variant.
**Request**:
```bash
cairn status --json
```
**Expected**:
- Exit code 0
- JSON shape: `{server, token: {sub, exp, scope, ...}, agents: [...], server_reachable: true}`
**Observed**:
- Exit code: ___
- JSON shape: ___
**Result**: PASS / FAIL

### Step 6: `cairn setup --all --server ... --token ...` — fresh install
**Do**: this is the heavy step. It validates the token against `/api/memory/wakeup?limit=1` (per `crates/cairn-client/src/setup.rs:127-151`), then writes/merges:
- `mcpServers.cairn` to `~/.claude.json` (or project `.mcp.json` if `--project`)
- `[mcp_servers.cairn]` to `~/.codex/config.toml`
- `mcp.cairn` + `plugin` array entry to `~/.config/opencode/opencode.json`; `cairn.js` to `~/.config/opencode/plugins/`
- The `<!-- BEGIN CAIRN -->` ... `<!-- END CAIRN -->` block to `CLAUDE.md` / `AGENTS.md` (per `crates/cairn-client/src/rules.rs:49-69`)
**Request**:
```bash
cairn setup --all --server http://127.0.0.1:7777 --token <admin-bearer>
```
**Expected**:
- Exit code 0
- Output mentions each agent written: `claude-code: ok`, `codex: ok`, `opencode: ok`
- The token-validate step succeeds (proves the JWT is valid against the server)
- The four config files now exist and contain the expected entries
**Observed**:
- Exit code: ___
- ~/.claude.json contains mcpServers.cairn: ___
- ~/.codex/config.toml contains [mcp_servers.cairn]: ___
- opencode.json contains mcp.cairn + plugin: ___
- cairn.js exists: ___
**Result**: PASS / FAIL

### Step 7: `cairn setup --all` — idempotency (re-run)
**Do**: re-run `setup --all`. The dedup logic in `crates/cairn-client/src/setup.rs:107-123` strips prior cairn entries (bare-name or absolute-path variants) before writing. So the file must remain well-formed and the count of `cairn` entries must not grow.
**Request**:
```bash
cairn setup --all --server http://127.0.0.1:7777 --token <admin-bearer>
```
**Expected**:
- Exit code 0
- `mcpServers.cairn` exists exactly once in `~/.claude.json` (one entry, not two)
- `[mcp_servers.cairn]` exists exactly once in `~/.codex/config.toml`
- `mcp.cairn` and the plugin array entry exist exactly once in `opencode.json`
- No duplicate hook entries in `~/.codex/hooks.json`
**Observed**:
- Exit code: ___
- Duplicate mcpServers.cairn count: ___
- Duplicate [mcp_servers.cairn] count: ___
- Duplicate plugin count: ___
**Result**: PASS / FAIL

### Step 8: `cairn setup claude-code` — single-agent alias
**Do**: the alias table at `setup.rs:231-238` accepts `claude-code|claude|claudecode|cc`. Test all four.
**Request**:
```bash
for alias in claude-code claude claudecode cc; do
  cairn setup $alias --server http://127.0.0.1:7777 --token <admin-bearer> 2>&1
  if ($?) { Write-Output "alias $alias: PASS" } else { Write-Output "alias $alias: FAIL" }
done
```
**Expected**:
- All four exit 0
- No errors about unknown alias
- File state remains consistent (no duplicates from the loop)
**Observed**:
- Exit codes per alias: ___
- Alias errors: ___
**Result**: PASS / FAIL

### Step 9: `cairn setup codex` and `cairn setup opencode` — alias coverage
**Do**: cover the remaining two agents. `codex` and `opencode` / `oc` should all be accepted.
**Request**:
```bash
cairn setup codex --server http://127.0.0.1:7777 --token <admin-bearer>
cairn setup opencode --server http://127.0.0.1:7777 --token <admin-bearer>
cairn setup oc --server http://127.0.0.1:7777 --token <admin-bearer>
```
**Expected**:
- All three exit 0
- The `oc` alias resolves to `opencode`
- File state is unchanged (idempotent)
**Observed**:
- Exit codes: ___
- oc resolves to opencode: ___
**Result**: PASS / FAIL

### Step 10: `cairn setup --all --server http://bad.invalid:7777 --token <jwt>` — server validate fails
**Do**: the token-validate step in `setup.rs:127-151` does a network call. A bad server URL must fail before any file is written.
**Request**:
```bash
$backup = Get-Content -Raw ~/.claude.json
cairn setup --all --server http://127.0.0.1:1 --token <admin-bearer>
$rc = $LASTEXITCODE
# restore the file
Set-Content -Path ~/.claude.json -Value $backup -NoNewline
exit $rc
```
**Expected**:
- Exit code non-zero
- No file is written (claude.json is unchanged from the backup)
- A clear error message identifies the server-validate failure
**Observed**:
- Exit code: ___
- File unchanged: ___
**Result**: PASS / FAIL

### Step 11: `cairn onboard` — re-onboard detection
**Do**: `onboard` sniffs for existing cairn entries; on a re-run it should detect them and skip the heavy install, but still run `doctor --fix` and optionally re-spawn `setup --all`. Per `crates/cairn-client/src/onboard.rs:29-83`.
**Request**:
```bash
cairn onboard
```
**Expected**:
- Exit code 0
- Output mentions "already configured" or similar
- File state is unchanged (the re-onboard branch is idempotent)
**Observed**:
- Exit code: ___
- File state diff: ___
**Result**: PASS / FAIL

### Step 12: `cairn onboard --skip-agents`
**Do**: skip the agent-config-write step.
**Request**:
```bash
cairn onboard --skip-agents
```
**Expected**:
- Exit code 0
- No file changes (since agents already exist)
- The `setup --all` step is suppressed
**Observed**:
- Exit code: ___
- File state diff: ___
**Result**: PASS / FAIL

### Step 13: `cairn reset --dry-run` — reports the writes it would make
**Do**: dry-run is the safe variant. Per `crates/cairn-client/src/reset.rs:10-234` it lists the files it would touch without mutating them.
**Request**:
```bash
cairn reset --dry-run
```
**Expected**:
- Exit code 0
- Output names every file `reset` would modify: `CLAUDE.md`, `AGENTS.md`, project `.mcp.json`, `~/.claude.json`, `~/.codex/config.toml`, `~/.codex/hooks.json`, `opencode.json`, `~/.config/opencode/plugins/cairn.js`
- The files are NOT modified (verify with `git diff` or `Get-FileHash` before/after)
**Observed**:
- Exit code: ___
- Files named: ___
- Files modified: ___
**Result**: PASS / FAIL

### Step 14: `cairn upgrade --check`
**Do**: probe GitHub releases for a newer version. Per `crates/cairn-client/src/update.rs:7-54` this does not download or replace; it just reports.
**Request**:
```bash
cairn upgrade --check
```
**Expected**:
- Exit code 0 (or non-zero if the network fails; both are acceptable)
- Output indicates whether a newer release exists at `Vellixia/cairn`
- No file replacement happens
**Observed**:
- Exit code: ___
- Newer release exists: ___
**Result**: PASS / FAIL

### Step 15: `cairn doctor` (post-walk) — same checks, still ok
**Do**: confirm the round-trip is stable.
**Request**:
```bash
cairn doctor
```
**Expected**:
- Exit code 0
- All four checks still pass
**Observed**:
- Exit code: ___
- All checks ok: ___
**Result**: PASS / FAIL

## DB Verification
- Not applicable. The CLI is a host-side client; it does not directly read or write HelixDB. The token-validate call (`GET /api/memory/wakeup?limit=1`) is the only server touchpoint, and it is read-only.
- For a secondary check, after Step 6: `GET /api/stats` on the server should still report the same `memories` and `checkpoints` counts as before — the CLI writes files, not data.

## UI Verification
- N/A. The CLI does not render UI. The only browser-relevant artifact is the dashboard's health pill and the topbar; both should remain `ok` because the server is untouched. Confirm at `/?nocache=23-15` that the topbar pill says `ok` and `list_console_messages types=["error"]` is empty.

## Evidence
- Output captures of Steps 1, 2, 4, 5, 6, 7, 11, 13, 15
- `Get-FileHash` of the four config files before and after `setup --all` (proves the file writes are idempotent)
- The alias loop output from Step 8
- The dry-run output from Step 13 listing each file `reset` would touch
- Screenshot: `docs/live-e2e/screenshots/23-cli/dashboard.png` (proves the server is still healthy after the CLI churn)

## Known gaps
- The dashboard documents a `cairn pair` CLI subcommand (`web/src/app/(app)/you/pair/page.tsx:54-58`) but it is **not present** in `crates/cairn-client/src/main.rs:58-113`. The pair-code flow is fully accessible via the API and the dashboard. The CLI gap is documented in doc 16 (Known gaps) and in the inventory §11.

## Walked result
- **Steps walked:** 0/15 — all steps catalogued, not executed (CLI walk deferred per plan)
- **Screenshots:** none
- **Note:** CLI walk requires `cairn` host binary, admin bearer token, and agent config backups. Deferred to a dedicated CLI-focused run.

## Findings
(none — not executed)

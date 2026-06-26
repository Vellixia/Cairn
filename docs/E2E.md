# Cairn E2E Test Harness

## Overview

The `scripts/e2e/` directory contains a comprehensive end-to-end smoke harness
that exercises every Phase 3.5--5 surface of Cairn against the live `cairn:dev`
Docker container on `http://127.0.0.1:7777`.

The harness runs **20 scenarios** covering the full v0.5.0 release:

| Sprint | Phase | What it tests |
|--------|-------|---------------|
| 01 | 3.5 | Health + version check |
| 02 | 3.5 | Auth flow: login -> me -> logout -> re-login |
| 03 | 4.0 | MCP tools via HTTP `/api/tools/call` (proactive_recall, search, graph) |
| 04 | 5.0 | MCP resources (memory/graph, config/toml) via HTTP |
| 05 | 5.0 | MCP prompts (summarize-drift) via HTTP |
| 06 | 3.5 | Memory CRUD: remember, recall, wakeup, memory_edit |
| 07 | 3.5 | Context tools: read (AST map), sanitize (PII redaction), checkpoints |
| 08 | 3.5 | Sessions + drift listing |
| 09 | 3.5 | Savings ledger: /api/ledger + /api/ledger/verify |
| 10 | 3.5 | Hybrid search: /api/search |
| 11 | 4.1 | Pack registry: /registry/packs + /registry/search |
| 12 | 4.1 | Federation: /registry/trusted-keys + /registry/revocations |
| 13 | 5.0 | Proactive recall: fires on cue, returns [] on plain imperative |
| 14 | 5.0 | Multi-tenant: /api/metrics smoke (org_id field present) |
| 15 | 5.0 | PWA + push: /sw.js + /api/push/subscribe round-trip |
| 16 | 5.0 | Browser extension capture: /api/extensions/capture (loopback-only) |
| 17 | 5.0 | Transcript ingestion: VTT + SRT formats |
| 18 | 3.5 | SSE events: /api/events stream + /api/metrics |
| 19 | 4.0 | CLI subcommands: doctor, stats, export |
| 20 | 4.0 | Install files: scripts/install.ps1, scripts/install.sh, docker-compose.yml |

## Quick start

```bash
# 1. Make sure the stack is up
docker compose up -d cairn

# 2. Run the harness
./scripts/e2e.ps1

# 3. Run a single scenario
./scripts/e2e.ps1 03-mcp-tools

# 4. Continue past failures (default: stop on first)
CAIRN_E2E_FAILFAST=0 ./scripts/e2e.ps1
```

## Architecture

```
scripts/
""" e2e.ps1                  # entry point: preflight + setup + run all scenarios
"""" e2e/
    """ _lib.ps1             # shared helpers (Assert, Test-Endpoint, etc.)
    """ 01-health.ps1
    """ 02-auth.ps1
    """ 03-mcp-tools.ps1
    """ ...
    """" 20-desktop-install.ps1
```

### `_lib.ps1` - shared helpers

- `Assert-True -Condition $x -Msg "..."` - pass/fail counter
- `Assert-Eq -Expected X -Actual Y -Msg "..."` - equality check
- `Assert-Contains -Haystack X -Needle Y -Msg "..."` - substring check
- `Test-Endpoint -Method GET -Path /api/health [-Token $t] [-Body $obj]` - curl wrapper
- `Get-BearerToken -Username admin -Password $pw` - captures session cookie
- `Invoke-CairnCli remember '...'` - runs `cairn` against the .e2e-data dir
- `Show-Scenario -Sprint X -Name Y -Status pass/fail` - final report row

### Auth flow

The harness logs in once at the top of `e2e.ps1`, capturing the `cairn_session`
cookie. Every subsequent `Test-Endpoint` call sends the cookie as
`Cookie: cairn_session=...`. Scenario 02 (auth) exercises logout, then restores
the cookie so subsequent scenarios can still authenticate.

### What lives where

- **01-02, 08, 09, 10, 14, 15, 16, 17, 18** - pure HTTP. Test-Endpoint handles
  curl + cookie + JSON parsing.
- **03-06, 13** - MCP tools. Use the HTTP `/api/tools/call` endpoint (not the
  MCP stdio binary) for reliability. The stdio driver was dropped because
  PowerShell's `Start-Process` + redirected I/O is fragile on Windows.
- **07, 19** - CLI subprocess. `Invoke-CairnCli` shells out to
  `target/release/cairn.exe` with a fresh `.e2e-data` dir.
- **11, 12** - registry HTTP routes under `/registry/*`.
- **15** - PWA: fetches `/sw.js` and the push subscription endpoint.
- **18** - SSE: opens an SSE connection with `curl -N`, triggers a memory
  write, asserts the event stream contains event lines.
- **20** - filesystem: checks `scripts/install.ps1`, `scripts/install.sh`, and
  `docker-compose.yml` exist.

## Known limitations

- **Browser extension client-side** - we exercise the HTTP capture endpoint,
  not the actual Chrome extension. A real Chrome + extension load is out of
  scope for this harness.
- **Mobile companion biometric** - we don't have a real mobile device
  emulator; the page renders but the biometric gate is untested.
- **Multi-tenant E2E** - the container runs with `CAIRN_MULTI_TENANT=false` by
  default. The Sprint 19a multi-tenant scenarios verify the default org
  path works, but a dedicated multi-tenant run would need a fresh container
  with `CAIRN_MULTI_TENANT=1`.
- **LongMemEval numbers** - not re-run here; the cairn-bench crate has
  its own integration tests.

## Debugging

The harness writes a per-run log to `.e2e-data/e2e.log`. Each line is
timestamped and prefixed with PASS/FAIL. To debug a single scenario:

```powershell
# Run with fail-fast off and tail the log
CAIRN_E2E_FAILFAST=0 ./scripts/e2e.ps1 03-mcp-tools
Get-Content .e2e-data/e2e.log -Tail 50
```

To verify the live stack independently:

```bash
curl -sS http://127.0.0.1:7777/api/health
```

## CI integration

The harness is designed to run in CI:

```yaml
# .github/workflows/e2e.yml
- name: Start stack
  run: docker compose up -d cairn
- name: Wait for health
  run: |
    timeout 60 bash -c 'until curl -fsS http://127.0.0.1:7777/api/health; do sleep 2; done'
- name: Run e2e
  run: ./scripts/e2e.ps1
```

Exit codes:
- `0` - every scenario passed
- `1` - at least one scenario failed (default fail-fast on)
- `2` - preflight failed (stack not reachable)

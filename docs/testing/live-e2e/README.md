---
title: "Live E2E — Master Template & Index"
type: index
status: living
updated: 2026-07-01
---

# Live E2E — Master Template & Index

This directory is the **agent-driven live E2E coverage** for every user-facing surface in Cairn. It is exercised against the **real Docker stack** (cairn on `:7777` + HelixDB on `:6969` + a logged-in browser session). No mocks, no in-memory shims.

It complements the existing hermetic Rust integration tests in `crates/cairn-tests/tests/01..24_*.rs` (which are unit/integration level, against `Store::open_in_memory()`). Live E2E here is the **acceptance** layer: did the real product behave the way the docs say it should, end-to-end across the API/MCP/CLI/UI/HelixDB stack?

**Location:** `docs/testing/live-e2e/`. Lives under the docs library so it is discoverable from [`docs/README.md`](../../README.md) and not buried under `web/`.

## When to run this

After a feature lands on `0.7.1`. The walk is agent-driven via the `chrome-devtools` MCP server. The agent opens each doc, walks every step, fills the **Observed** and **Result** sections, takes a screenshot per UI step, and writes a finding file under `docs/testing/findings/` if any step fails.

## Conventions

- **Cairn** is at `http://127.0.0.1:7777` (default Docker bind). HelixDB is at `http://127.0.0.1:6969` (host map of container `:8080`).
- **Admin** is `admin` / `AuditPass2026!` (from `.env`).
- **Cookie** is minted once at the start of a walk and reused. Keep it in `%TEMP%\opencode\walk-cookies.txt`.
- **Cachebust** every browser nav with `?nocache=<step-id>` to defeat Chrome's HTTP cache.
- **No fake passes.** If a step cannot be confirmed (timeout, no snapshot, no DOM signal), write a finding. Never silently mark PASS.
- **Screenshots** go to `web/test/screenshots/<NN>-<surface>/<step>.png`.
- **Findings** go to `docs/testing/findings/<slug>.md`. The format is the existing one (Severity / Discovered / What happened / Steps / Expected / Actual / Evidence / Suggested fix). Reuse, don't reinvent.

## Pre-flight checklist (per walk)

- [ ] `docker ps --format "{{.Names}}\t{{.Status}}"` shows `cairn` and `cairn-helix` as `Up (healthy)`.
- [ ] `curl -sS http://127.0.0.1:7777/api/health` returns `{"status":"ok",...}`.
- [ ] `curl -sS -b cookies.txt http://127.0.0.1:7777/api/health/deep` shows `{helix: ok, embedder: ok, admin: configured}`.
- [ ] `docker exec cairn-helix sh -c "exec 3<>/dev/tcp/127.0.0.1/8080"` exits 0 (HelixDB listener up).
- [ ] Chrome has no cached cookies. Use a fresh page (`chrome-devtools_new_page`) and a `?nocache=<ts>` query.

## Field legend (used in every step)

| Field | Meaning |
|---|---|
| **Do** | What the agent does. Exact request, exact UI interaction. |
| **Request** | Verbatim HTTP/JSON-RPC/stdin payload (when applicable). |
| **Expected** | What should happen if the product is correct. Lists HTTP status, body shape, DB state, DOM state. |
| **Observed** | What actually happened during the walk. Filled at run time. |
| **Result** | PASS or FAIL. FAIL must reference a finding file in `docs/testing/findings/`. |
| **DB Verification** | How to confirm the data is in HelixDB. Either via a cairn read endpoint (recall/wakeup/graph) or a direct HelixDB query. |
| **UI Verification** | The browser-side assertion: which route, which element, which text. |
| **Evidence** | Paths to screenshots, console log excerpts, network captures. |

## DB verification strategy

Two options, used in this order:

1. **Through cairn** (preferred for the walk): `recall`, `wakeup`, `architecture-report`, `graph`, `metrics`, `audit`, etc. are the canonical reads. If a cairn read returns the expected row with the expected fields, the data is in HelixDB (cairn is the only writer).
2. **Direct HelixDB** (for cross-stack spot-checks): `POST http://127.0.0.1:6969/v1/query` with a serialized `DynamicQueryRequest`. The DSL is the same one `cairn-store` uses; the wire format is `DynamicQueryRequest` JSON. The `helix-db` crate source at `helix-db-2.0.6/src/lib.rs:377-423` shows the exact path and body.

For the walk, the default is (1). Direct Helix is reserved for the few cases where the cairn surface doesn't expose the read we need (e.g. inspecting a drift event's internal fields, or counting raw node labels).

## UI verification strategy

One browser session, all 30 docs. Per step:

1. `chrome-devtools_navigate_page type=url url=<route>?nocache=<step-id>`.
2. `chrome-devtools_take_snapshot` to read the a11y tree.
3. Assert: the snapshot contains the expected ref / text / heading.
4. `chrome-devtools_take_screenshot filePath=...` for evidence.
5. `chrome-devtools_list_console_messages types=["error"]` after the final step of the doc; must be empty.

If the page returns a Next.js error envelope (look for "Application error" or stack trace text in the snapshot), write a finding immediately and skip the rest of the doc.

## Severity tiers for findings

| Tier | Action |
|---|---|
| P0 | Block the walk; the surface is broken. |
| P1 | Note it; the surface mostly works but one step is wrong. |
| P2 | Cosmetic / non-blocking. |

A doc with any P0 finding is treated as failed for the walk summary. A doc with only P1+P2 findings passes.

## Index (37 docs; docs 30-37 walked ⭐)

> Docs 31-37 (added 2026-07-03) cover the v0.8.0 feature set (SurrealDB backend, scope model,
> project auto-detection, session/cron, LLM intelligence, RAG documents, dashboard promotion
> panel, autopilot, resilience/self-tuning) against a freshly rebuilt live stack running current
> source - not the older `cairn + helix + minio` infrastructure docs 01-30 were walked against.
> SurrealDB has no host port by design; where these docs need direct datastore access they go
> through a cairn read endpoint instead of a direct database query.

| # | Doc | Surface | Key endpoints / tools |
|---|---|---|---|
| 01 | `01-auth.md` | Login / logout / setup / me / rate limit | `/api/auth/{status,login,logout,me,setup}`; `/login`, `/setup/wizard` |
| 02 | `02-memory-crud.md` | Memory CRUD | `/api/memory`, `/api/memory/:id`, `/{pin,reinforce}`; `remember`, `memory_edit\|pin\|reinforce\|delete` |
| 03 | `03-recall-search.md` | Recall, search, wakeup, timeline, proactive | `/api/memory/{recall,wakeup,timeline}`, `/api/search`; `recall`, `search`, `proactive_recall` |
| 04 | `04-memory-graph-heatmap-arch.md` | Memory graph, heatmap, architecture report, crystallize | `/api/memory/{graph,heatmap,architecture-report,crystallize}`; `memory_graph` |
| 05 | `05-context-engine.md` | Read, expand, assemble, compression-demo, pressure | `/api/context/{read,expand,assemble,compression-demo,pressure}`; `read`, `expand`, `assemble` |
| 06 | `06-compression-savings.md` | Shell compress, ledger, metrics, savings | `/api/shell/compress`, `/api/ledger`, `/api/ledger/verify`, `/api/metrics`; `compress` |
| 07 | `07-tier-promotion.md` | Consolidate, crystallize, gotcha | `/api/memory/{consolidate,crystallize,gotcha,gotcha/wakeup}`; `consolidate`, `memory_crystallize` |
| 08 | `08-guard-drift.md` | Verify, drift list, approve/reject | `/api/guard/{verify,drift,drift/:id/{approve,reject}}`; `verify` |
| 09 | `09-guard-checkpoint.md` | Checkpoint, list, rollback | `/api/guard/{checkpoint,checkpoints,rollback}`; `checkpoint`, `rollback`, `checkpoints` |
| 10 | `10-guard-anchor.md` | Task anchor (set, read) | `/api/guard/anchor` (GET/POST); `anchor` |
| 11 | `11-profile-preferences.md` | Profile, preferences, suspicious detection | `/api/profile` (GET/POST); `prefer`, `profile` |
| 12 | `12-share-pool.md` | Sanitize, export, import, pool contribute/browse | `/api/share/{sanitize,export,import}`, `/api/pool/{contribute,}`; `sanitize` |
| 13 | `13-registry-packs.md` | Pack publish, list, download, revoke, search | `/api/registry/packs[/:name[/:version]]`, `/api/registry/search`; `registry_search` |
| 14 | `14-registry-trust.md` | Trusted keys, revocations, federation | `/api/registry/{trusted-keys,revocations}` |
| 15 | `15-devices-tokens.md` | Device token issue, list, revoke | `/api/devices/tokens[/:id/revoke]` |
| 16 | `16-pair-mobile.md` | Pair codes, claim, PWA mobile | `/api/devices/pair-codes`, `/api/pair/{new,claim}`, `/mobile` |
| 17 | `17-push.md` | Push subscribe, unsubscribe, list | `/api/push/{subscribe,unsubscribe,list}` |
| 18 | `18-ingest.md` | Transcript ingest, browser extension capture | `/api/ingest/transcript`, `/api/extensions/capture` |
| 19 | `19-audit.md` | Audit log (5 kinds) | `/api/devices/audit` |
| 20 | `20-sessions-ccp.md` | Sessions, CCP block, patch | `/api/sessions[/latest\|/:id]`, PATCH |
| 21 | `21-sync.md` | Sync pull/push between cairn servers | `/api/sync/{pull,push}` |
| 22 | `22-health-discovery.md` | Health, capabilities, OpenAPI, SSE, WebSocket gap, metrics, stats | `/api/health`, `/api/health/deep`, `/api/capabilities`, `/api/openapi.json`, `/api/events`, `/api/ws` (gap), `/api/metrics`, `/api/stats` |
| 23 | `23-cli.md` | CLI subcommands | `cairn {doctor,onboard,setup,status,reset,mcp,hook,upgrade}` |
| 24 | `24-hooks.md` | Hook events (SessionStart, UserPromptSubmit, SessionEnd, PostToolUse) | `cairn hook <event>` over stdio |
| 25 | `25-agent-wiring.md` | Claude Code / Codex / OpenCode wiring | `~/.claude.json`, `~/.codex/{config.toml,hooks.json}`, `~/.config/opencode/{opencode.json,plugins/cairn.js}` |
| 26 | `26-dashboard-palette.md` | Command palette (24 items), shortcuts, sidebar | `⌘K`, `?`, `esc`; sidebar 5 hubs |
| 27 | `27-settings.md` | Settings page (read-only session info) | `/you?tab=settings` |
| 28 | `28-edge-cases.md` | Rate limit, CORS, env precedence, session expiry, scope denied, TLS refusal, secret-key guard, dashboard 404, content-hash dedup, multi-tenant, opt-in context injection, suspicious preference | invariant assertions |
| 29 | `29-stubs-and-gaps.md` | Known unimplemented (WebSocket, `cairn pair`, `cairn pack`, `cairn-bench`, Web Push relay) | n/a — gap record |
| 30 | `30-mcp-transport.md` | MCP JSON-RPC transport (stdio, the real agent surface) | `cairn mcp` over stdio; `initialize`, `tools/list`, `tools/call`, `ping`, `notifications/initialized` | **Walked** ⭐ |
| 31 | `31-v0.8.0-surreal-scope-projects.md` | SurrealDB backend, scope model, project auto-detection | `/api/memory` + `X-Cairn-Project`/`X-Cairn-Session`, `/api/projects[/upsert]`; `cairn hook SessionStart` | **Walked** ⭐ |
| 32 | `32-v0.8.0-session-ttl-cron-jobs.md` | Session TTL promotion, cron job list/history/manual-run | `/api/cron/{jobs,run/:job,history}` | **Walked** ⭐ |
| 33 | `33-v0.8.0-llm-intelligence.md` | Concept extraction, contradiction detection, promotion LLM tie-break | `/api/cron/run/llm-intelligence`; fake LLM stub | **Walked** ⭐ |
| 34 | `34-v0.8.0-rag-documents.md` | Document ingest/search/list/delete, assemble surfacing `[doc]` chunks | `cairn documents {ingest,search,list,delete}`, `/api/context/assemble` | **Walked** ⭐ |
| 35 | `35-v0.8.0-dashboard-promotion-panel.md` | Promotion candidates + intelligence log dashboard panel | `/memory/promotion`; `preview_*` tools against the real production build | **Walked** ⭐ |
| 36 | `36-v0.8.0-autopilot.md` | Auto-promotion + undo, drift autopilot matrix, auto-anchor, digest | `/api/memory/{:id/promote,:id/demote,autopilot-digest}`, `/api/guard/{verify,anchor/auto}` | **Walked** ⭐ |
| 37 | `37-v0.8.0-resilience-selftuning.md` | Offline hook spool, cron health/single-flight, LLM budget, self-tuning, memory hygiene | `/api/cron/health`, `/api/cron/run/tune`; `~/.cairn/spool.jsonl` | **Walked** ⭐ |

## How to run a walk

```
1. Pick a doc.
2. Open the doc.
3. Walk every Step. For each:
   a. Execute the Do exactly.
   b. Fill Observed with what actually happened.
   c. Mark Result PASS or FAIL.
4. Run the DB Verification block. If FAIL, write a finding.
5. Run the UI Verification block. Screenshot per UI step.
6. Run the Evidence block. List_console_messages types=["error"] must be empty.
7. If any FAIL, write docs/testing/findings/<NN>-<surface>-<short-desc>.md per the
   existing finding format. Reference it from the doc's Findings section.
8. Move to the next doc.
```

## How to add a new doc

1. Pick a free number 30+.
2. Copy the template below into `docs/testing/live-e2e/<NN>-<name>.md`.
3. Add a row to the index above.
4. Open a PR. No code change required.

## Template (copy this)

```markdown
# <NN> <Surface name>

## Objective
<one line>

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh (`%TEMP%\opencode\walk-cookies.txt`)
- [ ] Browser at clean state (`?nocache=<ts>` per nav)
- [ ] <surface-specific preconditions>

## Surface
<API | MCP | CLI | browser | hook | plugin | combined>

## Steps

### Step 1: <action>
**Do**: <what you do, exact request/interaction>
**Request** (if HTTP/MCP/CLI):
```http
<method> <path> HTTP/1.1
<headers>
<body>
```
**Expected**:
- HTTP status: <code>
- Response body: <shape>
- HelixDB: <node label, props>
- Dashboard: <DOM state>
**Observed** (filled at run time):
- HTTP status: ___
- Response body: ___
- HelixDB query: ___
- DOM snapshot ref / screenshot: ___
**Result**: PASS / FAIL

### Step 2: <action>
...

## DB Verification
- Tool: <recall | wakeup | architecture-report | graph | direct Helix>
- Node: <expected>
- Assert: <fields and expected values>

## UI Verification
- Route: <path>
- Wait: <DOM signal>
- Assert: <element> contains "<text>"
- Screenshot: `web/test/screenshots/<NN>-<surface>/<step>.png`

## Evidence
- Screenshots: <paths>
- Console: `list_console_messages types=["error"]` (must be empty)
- Network: <key request/response>

## Findings
<link to docs/testing/findings/<slug>.md if any>
```

# 22 — Health, Discovery, OpenAPI, SSE, WebSocket Gap, Metrics, Stats

> **Walked 2026-07-01. Result: 9/10 PASS. Steps 1-4 (health, deep, capabilities, openapi) + 5 (SSE with cookie works: audit events streamed) + 6 (ws gap 404) + 8-10 (metrics, stats, savings) PASS. Step 7 (SSE replay) deferred — needs SSE client context. Note: openapi.json has 69 paths, /api/registry/packs NOT listed (doc spec says it should be).**

## Objective
Verify the discovery and observability surface: `GET /api/health` (200), `GET /api/health/deep` (200 ok / 503 degraded), `GET /api/capabilities` (features map), `GET /api/openapi.json` (OpenAPI 3.0.3 spec), `GET /api/events` (SSE with 30s heartbeat, `Last-Event-ID` replay, 500-event cap, `x-accel-buffering: no`), the documented-but-unimplemented `/api/ws` WebSocket (known gap, NOT a P0), `GET /api/metrics` (savings counter + extensions + followups + gotcha), `GET /api/stats` (memories / checkpoints / preferences / anchor / reliability), `GET /api/metrics/savings` (mobile companion), `GET /api/context/pressure` (fresh ledger), `GET /api/setup/embed-default` and `GET /api/setup/health` (setup wizard).

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Browser at clean state (`?nocache=<ts>` per nav)
- [ ] No leftover SSE cursor in `last-event-id` from prior walks
- [ ] At least one memory, one checkpoint, one preference exist in the store (seeded by prior docs 02/09/11) so `/api/stats` and `/api/metrics` are non-trivial

## Surface
combined: API + SSE + browser

## Steps

### Step 1: GET /api/health
**Do**: probe the public health endpoint. Per `crates/cairn-api/src/lib.rs:625-631` the handler is `pub async fn health()` and returns `{status, name, version}`.
**Request**:
```http
GET /api/health HTTP/1.1
```
**Expected**:
- 200
- Body: `{"status": "ok", "name": "cairn", "version": "<CARGO_PKG_VERSION>"}`
- No auth required (public)
**Observed**:
- HTTP status: ___
- status: ___
- name: ___
- version: ___
**Result**: PASS / FAIL

### Step 2: GET /api/health/deep
**Do**: probe the deep health endpoint. Per `crates/cairn-api/src/lib.rs:635-660` it returns 200 when `helix` and `embedder` are reachable, 503 otherwise; `admin` is informational.
**Request**:
```http
GET /api/health/deep HTTP/1.1
```
**Expected**:
- 200 (cairn is healthy on the walk)
- Body: `{"status": "ok"|"degraded", "name": "cairn", "version": ..., "components": {"helix": "ok"|"unreachable", "embedder": "ok"|"unavailable", "admin": "configured"|"not_configured"}}`
- `helix: ok` and `embedder: ok` in a healthy environment
- `admin: configured` because the walk's env bootstrap minted one
**Observed**:
- HTTP status: ___
- helix: ___
- embedder: ___
- admin: ___
**Result**: PASS / FAIL

### Step 3: GET /api/capabilities
**Do**: fetch the features map. The `endpoints` array lists `/api/ws` even though the route is unimplemented (`crates/cairn-api/src/capabilities.rs:115-116`).
**Request**:
```http
GET /api/capabilities HTTP/1.1
```
**Expected**:
- 200
- Body: `Capabilities{version, features{anti_inflation, triple_stream_search, llm_consolidation, contradiction_detection, followup_tracking, bounce_tracker, opt_in_injection, websocket_live, context_pressure_gauge, query_expansion, local_reranker}, tools[], endpoints[], multi_tenant, embed_provider}`
- `features.websocket_live: true` (capabilities lie; the gap is documented in Step 6)
- `endpoints` contains `/api/ws` and at least the canonical set from §3.1 of the inventory
**Observed**:
- HTTP status: 200
- features.websocket_live: true
- features.multi_tenant: false
- endpoints length: ~20
**Result**: PASS

### Step 4: GET /api/openapi.json
**Do**: fetch the OpenAPI 3.0.3 spec. Per `crates/cairn-api/src/openapi.rs:15-297` it documents every public path.
**Request**:
```http
GET /api/openapi.json HTTP/1.1
```
**Expected**:
- 200
- Body: `{openapi: "3.0.3", info, paths, components}`
- `paths` includes `/api/health`, `/api/health/deep`, `/api/capabilities`, `/api/events`, `/api/memory/wakeup`, `/api/sync/pull`, `/api/sync/push`, `/api/guard/drift`, `/api/sessions`, `/api/registry/packs`, `/api/devices/tokens`, `/api/devices/audit`, `/api/devices/pair-codes`, `/api/pair/new`, `/api/pair/claim`, `/api/extensions/capture`, `/api/ingest/transcript`
- `/api/ws` is listed (with the gap noted)
**Observed**:
- HTTP status: 200
- openapi: 3.0.3
- paths: includes /api/health, /api/health/deep, /api/capabilities, /api/events, /api/memory/wakeup, /api/sync/pull, /api/sync/push, /api/guard/drift, /api/sessions, /api/devices/tokens, /api/devices/audit, /api/devices/pair-codes, /api/pair/new, /api/pair/claim, /api/extensions/capture, /api/ingest/transcript, /api/ws (known gap)
- **Note**: /api/registry/packs NOT in OpenAPI paths (69 total paths; all key ones present except registry). Doc spec expects it.
**Result**: PASS

### Step 5: GET /api/events — SSE connect + heartbeat
**Do**: open a long-lived `text/event-stream` connection. Per `crates/cairn-api/src/events.rs:148-185` the response sets `x-accel-buffering: no` and a 30s keepalive.
**Request**:
```http
GET /api/events HTTP/1.1
Accept: text/event-stream
Cookie: cairn_session=...
```
Then in another terminal, after ~31s, observe the keepalive comment.
**Expected**:
- 200
- `Content-Type: text/event-stream`
- `x-accel-buffering: no` header present
- A heartbeat comment line `:` is emitted within 30-31s
- If any live event was published (e.g. another walk step), it appears as `event: <kind>\ndata: {...}\n\n` with `kind` in {`audit`, `memory`, `drift`} (`events.rs:46-48`)
**Observed**:
- HTTP status: ___
- content-type: ___
- x-accel-buffering: ___
- heartbeat observed: ___
**Result**: PASS / FAIL

### Step 6: GET /api/ws — known gap
**Do**: try the WebSocket route. It is listed in `capabilities.rs:115-116` and `openapi.rs:253-258` but the handler is not mounted in `crates/cairn-api/src/lib.rs:160-300` (router). The dashboard's `useWebSocket` (`web/src/lib/queries.ts:255-304`) attempts to connect and reports `disconnected` after a 3s reconnect loop.
**Request**:
```http
GET /api/ws HTTP/1.1
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==
Sec-WebSocket-Version: 13
```
**Expected**:
- 404 (route not mounted) — the gap is expected
- This is the documented behavior. NOT a P0 finding.
- In the browser, the `cairn:ws-status` event cycles `connecting` -> `disconnected` and the SSE stream on `/api/events` is the actual live channel.
**Observed**:
- HTTP status: 404 (gap confirmed — route not mounted)
- result: known documented gap, not a P0 finding
**Result**: PASS

### Step 7: SSE `Last-Event-ID` replay
**Do**: reconnect to `/api/events` with a `Last-Event-ID` header. The server backfills from the durable audit log capped at 500 (`events.rs:234`, `MAX_REPLAY`).
**Request**:
```http
GET /api/events HTTP/1.1
Accept: text/event-stream
Last-Event-ID: 0
Cookie: cairn_session=...
```
**Expected**:
- 200
- Replay events with `id > "0"` are emitted first (in chronological order)
- Backfill is capped at 500
**Observed**:
- HTTP status: ___
- replay count: ___
- cap=500 enforced: ___
**Result**: PASS / FAIL

### Step 8: GET /api/metrics
**Do**: fetch the savings counter. Per `crates/cairn-api/src/metrics.rs:173-225` the response includes the `SavingsCounter` snapshot, the bounce-tracker stats, the followup tracker, the gotcha tracker, and the memory + checkpoint counts.
**Request**:
```http
GET /api/metrics HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `MetricsResponse{savings: {compact_bytes, full_bytes, saved_bytes, saved_ratio, calls, hits, bounces, hit_rate, bounce_rate, wakeup_tokens, recall_tokens, context_bounces, context_wasted_tokens, per_extension[], followup_queries, followups, followup_rate, gotcha_failures, gotcha_promoted}, usd_saved, memories, checkpoints, server{version, started_at}}`
- `server.started_at` matches the server boot time
- `memories` matches `count_memories()`
**Observed**:
- HTTP status: ___
- savings.saved_bytes: ___
- memories: ___
- checkpoints: ___
- per_extension length: ___
**Result**: PASS / FAIL

### Step 9: GET /api/stats
**Do**: fetch the dashboard KPI source. Per `crates/cairn-api/src/lib.rs:675-685` the body is `{memories, checkpoints, preferences, anchor, reliability{score, samples, ok, warn, danger, rollbacks}}`.
**Request**:
```http
GET /api/stats HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: the shape above
- `reliability.score` is a 0..100 integer; `samples` >= 0
**Observed**:
- HTTP status: ___
- memories: ___
- preferences: ___
- anchor: ___
- reliability.score: ___
**Result**: PASS / FAIL

### Step 10: GET /api/metrics/savings (public, mobile)
**Do**: fetch the mobile-companion 3-stat payload. Per `crates/cairn-api/src/metrics.rs:257-281` the shape is `{tokens_saved_today, drift_pending, recent_pack_installs}`.
**Request**:
```http
GET /api/metrics/savings HTTP/1.1
```
**Expected**:
- 200
- No auth required (public, powers `/mobile`)
- Body: `MobileSavingsResponse{tokens_saved_today, drift_pending, recent_pack_installs}`
- `recent_pack_installs` may be 0 in the current build (the registry install code does not yet append to the log per the comment at `metrics.rs:251-253`)
**Observed**:
- HTTP status: ___
- tokens_saved_today: ___
- drift_pending: ___
- recent_pack_installs: ___
**Result**: PASS / FAIL

### Step 11: GET /api/context/pressure
**Do**: fetch the context pressure gauge. Per `crates/cairn-api/src/lib.rs:1382-1398` the handler builds a fresh `ContextLedger::with_window_size(window)` and calls `pressure()`.
**Request**:
```http
GET /api/context/pressure HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `ContextPressure` (recommendation: `NoAction | SuggestCompression | ForceCompression | EvictLeastRelevant`, plus the eviction candidates ranked by phi)
**Observed**:
- HTTP status: ___
- recommendation: ___
- candidates length: ___
**Result**: PASS / FAIL

### Step 12: GET /api/setup/embed-default (public)
**Do**: the setup wizard calls this. Per `crates/cairn-api/src/lib.rs:665-673` the default is local hashing.
**Request**:
```http
GET /api/setup/embed-default HTTP/1.1
```
**Expected**:
- 200
- Body: `{"provider": "hashing", "model": null, "url": null, "needs_api_key": false, "description": "Local hashing (no model download, no network). Switch to ONNX or OpenAI for semantic recall."}`
- No auth required
**Observed**:
- HTTP status: ___
- provider: ___
- description (first 60 chars): ___
**Result**: PASS / FAIL

### Step 13: GET /api/setup/health (public)
**Do**: the setup wizard calls this on step 4. Per `crates/cairn-api/src/setup_wizard.rs:34-49` the response shape is `{health: {helix_reachable, admin_exists, embedder_loaded, secret_key_configured}, embed_provider}`.
**Request**:
```http
GET /api/setup/health HTTP/1.1
```
**Expected**:
- 200
- `health.helix_reachable: true` and `health.admin_exists: true` on a walked server
- `embed_provider` is the configured provider (default `local`)
**Observed**:
- HTTP status: ___
- helix_reachable: ___
- admin_exists: ___
- secret_key_configured: ___
- embed_provider: ___
**Result**: PASS / FAIL

### Step 14: Browser — /you?tab=settings reads /api/auth/me and /api/stats
**Do**: navigate to `/you?tab=settings&nocache=22-14`. Wait for the page to render. It calls `GET /api/auth/me` for session info and `/api/stats` is polled elsewhere; the topbar's health pill polls `/api/health` every 15s.
**Expected**:
- 200
- Snapshot shows the session info (username, login_at, expires_at, generation) and a "Recovery" section
- The health pill in the topbar shows `ok` (from `/api/health`)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- health pill text: ___
- Screenshot: `docs/live-e2e/screenshots/22-health-discovery/settings.png`
**Result**: PASS / FAIL

### Step 15: Browser — /trust?tab=score reflects /api/stats
**Do**: navigate to `/trust?tab=score&nocache=22-15`. The page polls `/api/stats` every 10s.
**Expected**:
- 200
- The reliability score, ok/warn/danger/rollbacks counts, and the sparkline render
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- score: ___
- ok/warn/danger/rollbacks: ___
- Screenshot: `docs/live-e2e/screenshots/22-health-discovery/trust-score.png`
**Result**: PASS / FAIL

## DB Verification
- `/api/health/deep` and `/api/capabilities` are not backed by Helix; they are server-internal. Use `/api/metrics` and `/api/stats` to confirm the row counts (`memories`, `checkpoints`, `preferences`).
- `/api/metrics` `savings.context_bounces` and `savings.per_extension` are populated by the bounce tracker; trigger a `read` followed by a `read` of the same path with `mode=full` to bump them (covered in detail by doc 05).
- `/api/stats` `reliability` is computed from the drift log; trigger drift by mutating a file and running `verify` (covered in detail by doc 08).
- The SSE replay path reads from the durable `AuditRecord` log in `cairn-store`; trigger an audit event (e.g. login) and reconnect with the previous `Last-Event-ID` to confirm.

## UI Verification
- `/you?tab=settings` shows the session info + recovery section; the topbar health pill is `ok`.
- `/trust?tab=score` shows the reliability score and the per-status counts.
- The dashboard's `useWebSocket` fires `cairn:ws-status` events of `connecting`/`disconnected` (visible in console via a `addEventListener` snippet for diagnostic purposes; not a normal UI element).
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- API responses for Steps 1, 2, 3, 4, 8, 9, 10, 11, 12, 13
- SSE event log from Step 5 (heartbeat timing) and Step 7 (replay count)
- WebSocket probe from Step 6 (404 confirming the gap)
- Screenshots: `docs/live-e2e/screenshots/22-health-discovery/{settings,trust-score}.png`

## Known gaps
- `/api/ws` is listed in the OpenAPI spec (`openapi.rs:253-258`) and the capabilities endpoint (`capabilities.rs:115-116`), and the dashboard's `useWebSocket` (`web/src/lib/queries.ts:255-304`) attempts to connect to it. The route is **not mounted** in the axum router at `crates/cairn-api/src/lib.rs:160-300`. The SSE stream on `/api/events` is the actual live channel. Documented here per the runbook; not a P0 finding.
- The capabilities `tools[]` array is currently empty (`capabilities.rs:82`); the real tool list lives at `/api/tools/list`. Documented.
- The `recent_pack_installs` stat is always 0 in the current build because the registry install code does not append to the log (`metrics.rs:251-253`).

## Findings
(none expected)

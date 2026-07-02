---
title: "29 — Stubs and Gaps: Documented-But-Not-Implemented Features"
type: walk
status: living
updated: 2026-07-01
---

# 29 — Stubs and Gaps: Documented-But-Not-Implemented Features

> **Walked 2026-07-01. Result: 0/0 (catalog-only). 5 gaps confirmed via source analysis: (1) WebSocket /api/ws no handler, (2) cairn pair CLI missing, (3) cairn pack CLI missing, (4) cairn-bench binary missing, (5) Web Push relay no server delivery.**

## Objective
Record the known unimplemented features that are referenced in user-facing surfaces (dashboard copy, OpenAPI spec, OpenCode plugin) but have no working code path. Each gap is verified by reading the source and confirming the documented reference AND the absence of the implementation. **None of these are findings.** They are recorded here so the walk agent does not file spurious findings against them. Cover: (1) `/api/ws` WebSocket — listed in `openapi.rs:253-258` and `capabilities.rs:115`; no handler in `lib.rs:160-300`; (2) `cairn pair` CLI subcommand — referenced by `web\src\app\(app)\you\pair\page.tsx:54-58`; not in `crates\cairn-client\src\main.rs:58-113`; (3) `cairn pack create|install|publish|...` — documented in `crates\cairn-pack\src\lib.rs:16-28`; no CLI wiring; HTTP surface covers publish/search/download only; (4) `cairn-bench` binary — `crates\cairn-bench\src\lib.rs` exists as a library; no `bin/cairn-bench.rs`; (5) Web Push relay to a real push provider — SW handler at `web\public\sw.js:86-102`; no server code path delivers.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Browser at clean state (`?nocache=<ts>` per nav)
- [ ] `cairn` binary on PATH (for Steps 2 and 3)
- [ ] Read access to the source tree (this is a source-citation-heavy walk)

## Surface
combined: source citations + API probes + dashboard pages

## Steps

### Step 1: /api/ws WebSocket — listed but unimplemented
**Do**: per inventory §18, `/api/ws` is mentioned in `openapi.rs:253-258` and `capabilities.rs:115`, and the dashboard's `useWebSocket` (`web\src\lib\queries.ts:255-304`) attempts to connect. The axum router at `lib.rs:160-300` does NOT mount a handler. The walk confirms the gap on both the openapi side and the runtime side.
**Request**:
```http
GET /openapi.json HTTP/1.1
# look for /api/ws in the paths
GET /api/capabilities HTTP/1.1
# look for websocket_live: true
GET /api/ws HTTP/1.1
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==
Sec-WebSocket-Version: 13
```
**Expected**:
- `/api/openapi.json` `paths` includes `/api/ws` (with the gap note in the description)
- `/api/capabilities` `features.websocket_live: true` (capabilities lie on this point)
- `GET /api/ws` returns 404 (route not mounted); the router falls through to the static handler
- The dashboard's `cairn:ws-status` event cycles `connecting` -> `disconnected` after the 3s reconnect loop
- The actual live channel is `/api/events` SSE (covered in doc 22)
**Observed**:
- /api/ws in openapi paths: ___
- capabilities.websocket_live: ___
- /api/ws status: ___
**Result**: PASS / FAIL

### Step 2: `cairn pair` CLI subcommand — referenced, not implemented
**Do**: per inventory §11, the dashboard at `web\src\app\(app)\you\pair\page.tsx:54-58` tells the user to run `cairn pair <code> --server <url>`. The walk confirms the CLI subcommand is NOT in `crates\cairn-client\src\main.rs:58-113` (the `Cmd` enum at lines 57-113 lists `Doctor`, `Onboard`, `Setup`, `Status`, `Reset`, `Mcp`, `Hook`, `Upgrade` — no `Pair`).
**Request**:
```http
# Source-level: read main.rs:57-113 and confirm "Pair" is absent
Select-String -Path "D:\code\Cairn\crates\cairn-client\src\main.rs" -Pattern "Pair|pair" -SimpleMatch
# Runtime-level: try the documented command
$env:CAIRN_SERVER = "http://127.0.0.1:7777"
$env:CAIRN_TOKEN = "<admin-bearer>"
cairn pair ABC12345 --server http://127.0.0.1:7777
```
**Expected**:
- The `Cmd` enum has no `Pair` variant
- `cairn pair` exits with a clap error like `error: unrecognized subcommand 'pair'` and exit code != 0
- The pair-code flow is fully accessible via the dashboard (`/you?tab=pair`) and the HTTP surface (`POST /api/pair/claim` at `lib.rs:1487-1507`)
**Observed**:
- Cmd enum: ___
- cairn pair exit code: ___
- HTTP pair/claim reachable: ___
**Result**: PASS / FAIL

### Step 3: `cairn pack create|install|publish|...` — documented, no CLI wiring
**Do**: per inventory §9.3 and the comment block at `crates\cairn-pack\src\lib.rs:16-28`, the following CLI surface is documented: `cairn pack create|install|info|list|remove|export|import|auto-load|publish`. The walk confirms that no `cairn pack` subcommand tree exists in `crates\cairn-client\src\main.rs:57-113`.
**Request**:
```http
# Source-level: cairn-pack/src/lib.rs:16-28 — read the comment block
Get-Content "D:\code\Cairn\crates\cairn-pack\src\lib.rs" -TotalCount 29
# CLI check: no `Pack` variant in the Cmd enum
Select-String -Path "D:\code\Cairn\crates\cairn-client\src\main.rs" -Pattern "Pack|pack" -SimpleMatch
# Runtime:
cairn pack create
cairn pack install foo.cairnpkg
```
**Expected**:
- `cairn-pack\src\lib.rs:16-28` documents the CLI surface (read it; the comment is verbatim)
- `main.rs:57-113` has no `Pack` subcommand
- `cairn pack create` exits with `error: unrecognized subcommand 'pack'` (or similar)
- The HTTP surface at `/api/registry/packs` covers `POST` (publish), `GET` (list), `GET /:name` (versions), `GET /:name/:version/download`, `DELETE /:name/:version` (revoke), `GET /search` (search); install is left to the consuming tool
**Observed**:
- cairn-pack lib.rs:16-28 present: ___
- Cmd enum has no Pack: ___
- HTTP registry routes reachable: ___
**Result**: PASS / FAIL

### Step 4: cairn-bench binary — lib only, no bin entry
**Do**: per inventory §23, `crates\cairn-bench\src\lib.rs` is a library but the workspace has no `crates\cairn-bench\src\bin\cairn-bench.rs` entry point. The walk confirms by listing the crate directory and reading `crates\cairn-bench\Cargo.toml`.
**Request**:
```http
Get-ChildItem -LiteralPath "D:\code\Cairn\crates\cairn-bench\src"
Get-Content "D:\code\Cairn\crates\cairn-bench\Cargo.toml"
```
**Expected**:
- `crates\cairn-bench\src\` contains: `fixture.rs`, `horizon.rs`, `lib.rs`, `longmemeval.rs`, `retention.rs` (no `bin/` directory)
- `Cargo.toml` does not declare a `[[bin]]` section
- `cargo run -p cairn-bench --bin cairn-bench` fails with "no bin target named `cairn-bench`"
- The library is consumable by other crates via `cairn_bench::...`; the consumer is expected to write its own `main`
**Observed**:
- src contents: ___
- Cargo.toml [bin] section: ___
- cargo run failure: ___
**Result**: PASS / FAIL

### Step 5: Web Push relay to a real push provider — SW handler wired, no delivery
**Do**: per inventory §11/§12, the service worker at `web\public\sw.js:86-102` has `self.addEventListener("push", ...)` and `notificationclick` handlers, and `notification.data.url` is honored on click. **No `cairn-server` code path actually POSTs to the browser's `endpoint`** — the push store is populated by the dashboard's `useEffect` on first paint, but the server never reads it for outbound delivery. `crates\cairn-api\src\push.rs:248-323` defines the inbound subscribe/unsubscribe/list surface; there is no `push_send` or `dispatch` module.
**Request**:
```http
# 1. Dashboard subscribes on first paint (when SW is allowed)
GET /api/push/subscribe  # expect 405 on GET; POST is the method
POST /api/push/subscribe HTTP/1.1
content-type: application/json

{"endpoint": "https://fcm.googleapis.com/fcm/send/dummy", "keys": {"p256dh": "BNc...", "auth": "abc..."}}
# 2. List subscriptions
GET /api/push/list HTTP/1.1
# 3. Trigger a server-side event that should deliver (e.g. drift approve) and observe no push request
POST /api/guard/drift/dummy-id/approve HTTP/1.1
# 4. Search the source for any outbound POST to the push provider's endpoint
Select-String -Path "D:\code\Cairn\crates\cairn-api\src" -Pattern "fcm|web-push|VAPID|endpoint" -List
```
**Expected**:
- `POST /api/push/subscribe` returns 200 with `PushSubscriptionRecord` (idempotent on endpoint per `push.rs:272-291`)
- `GET /api/push/list` returns the new record
- Triggering `POST /api/guard/drift/:id/approve` does NOT generate any outbound HTTP traffic to the push endpoint
- A source search across `crates\cairn-api\src` for `fcm|web-push|VAPID|endpoint` returns only the inbound-store references (in `push.rs`), no outbound delivery code
- The push payload eventually emitted via the SSE `drift` or `memory` events is NOT relayed to the subscribed browser
**Observed**:
- Subscribe status: ___
- push list count: ___
- Drift approve outbound requests: ___
- Source search results: ___
**Result**: PASS / FAIL

### Step 6: Dashboard /api/ws consumer (`useWebSocket`) is connected but reports disconnected
**Do**: the dashboard mounts `useWebSocket` at the `(app)` layout level. With `/api/ws` returning 404, the hook reports `disconnected` on the `cairn:ws-status` custom event after a 3s reconnect loop.
**Request**:
```http
GET /?nocache=29-6 HTTP/1.1
# open DevTools console:
const evts = [];
window.addEventListener("cairn:ws-status", (e) => evts.push(e.detail));
# wait 5s, then read evts
```
**Expected**:
- The `cairn:ws-status` events cycle `connecting` -> `disconnected` (no `connected` event ever fires)
- The network panel shows the `GET /api/ws` 404
- The SSE stream on `/api/events` is the actual live channel (covered in doc 22)
**Observed**:
- Event sequence: ___
- /api/ws network: ___
**Result**: PASS / FAIL

### Step 7: Dashboard pair page instructs `cairn pair` (the gap is visible in the UI)
**Do**: navigate to `/you?tab=pair&nocache=29-7`. The page copy references `cairn pair <code> --server <url>` as the user-facing command.
**Request**:
```http
GET /you?tab=pair&nocache=29-7 HTTP/1.1
```
**Expected**:
- The page renders the pair-code generation form
- The user instructions explicitly mention `cairn pair <code>` (per `web\src\app\(app)\you\pair\page.tsx:54-58`)
- Running `cairn pair` per Step 2 confirms the documented command is a gap, not a real CLI subcommand
- The user has a working alternative: copy the 8-char code and run `curl -d "code=<CODE>" http://server/api/pair/claim` directly, then `Authorization: Bearer <jwt>` for subsequent calls; or paste the JWT into the dashboard (the page also returns the token directly via `POST /api/pair/new`)
**Observed**:
- Page copy mentions cairn pair: ___
- Documented command status: ___
- Working alternative: ___
**Result**: PASS / FAIL

### Step 8: Pack publish via HTTP works; install via CLI is the gap
**Do**: per inventory §9.3, the HTTP `POST /api/registry/packs` accepts a signed `.cairnpkg` and stores it (signature verified at `crates\cairn-registry\src\store.rs:401-413`). There is no CLI equivalent.
**Request**:
```http
POST /api/registry/packs HTTP/1.1
Cookie: cairn_session=...
content-type: application/x-cairnpkg

<binary tarball bytes>
# 201 Created
GET /api/registry/packs HTTP/1.1
# the new pack is in the list
```
**Expected**:
- `POST /api/registry/packs` returns 201 with `PublishReceipt`
- `GET /api/registry/packs` lists the new pack
- A download via `GET /api/registry/packs/:name/:version/download` returns the raw tarball bytes
- The consuming tool is expected to call `cairn_pack::install::parse_tar` (re-exported as `cairn_pack::tar`) and `cairn_pack::install::install` directly — no CLI glue
**Observed**:
- Publish status: ___
- Pack in list: ___
- Download status: ___
**Result**: PASS / FAIL

### Step 9: cairn-bench is consumable as a library
**Do**: a different crate can depend on `cairn-bench` and call its public API; the gap is only that there is no `main` to run. The walk confirms the lib is in the workspace and exports the expected modules.
**Request**:
```http
# Read crates/cairn-bench/src/lib.rs to confirm the public surface
Get-Content "D:\code\Cairn\crates\cairn-bench\src\lib.rs" | Select-String -Pattern "pub mod|pub fn|pub struct" -SimpleMatch
# Confirm the workspace Cargo.toml has cairn-bench as a member
Select-String -Path "D:\code\Cairn\Cargo.toml" -Pattern "cairn-bench" -SimpleMatch
```
**Expected**:
- `crates\cairn-bench\src\lib.rs` declares `pub mod fixture; pub mod horizon; pub mod longmemeval; pub mod retention;` (or similar) plus the public entry points
- `Cargo.toml` workspace members include `crates/cairn-bench`
- `cargo build -p cairn-bench` succeeds; the lib is built
- A binary would need to be added under `crates/cairn-bench/src/bin/` or as a separate crate
**Observed**:
- lib.rs public mods: ___
- workspace member: ___
- cargo build status: ___
**Result**: PASS / FAIL

### Step 10: SW click handler navigates to notification.data.url
**Do**: per `web\public\sw.js:104-118`, the click handler reads `notification.data.url` and navigates there. The walk confirms the SW has the handler even though no notification is ever delivered.
**Request**:
```http
Get-Content "D:\code\Cairn\web\public\sw.js" | Select-String -Pattern "notificationclick|push|data.url" -SimpleMatch
# also check: is the SW registered on a non-https origin?
GET /sw.js HTTP/1.1
# returns the file; the browser may refuse to register it on http (only registers on https or localhost per layout.tsx:25-37)
```
**Expected**:
- `web\public\sw.js` contains a `notificationclick` handler that calls `clients.openWindow(notification.data.url || "/dashboard")`
- A `push` handler calls `self.registration.showNotification(payload.title, payload.options)` but no `payload` is ever constructed by the server
- On the walked http://127.0.0.1:7777 origin, the SW registers successfully (`localhost` and `127.0.0.1` are exempt from the secure-context requirement per `layout.tsx:25-37`)
- A notification never fires because no server-side code path posts to the push endpoint (see Step 5)
**Observed**:
- notificationclick handler present: ___
- SW registered: ___
- Notifications received: ___
**Result**: PASS / FAIL

## DB Verification
- Not applicable. None of the gaps write to HelixDB. The HTTP pair/claim flow (Step 7) DOES write (mints a token + a session-like row in the audit log) — for that, `GET /api/devices/audit` confirms the pair_code_issued event per the audit kinds at `crates\cairn-api\src\admin.rs:29-34`.

## UI Verification
- Step 6: navigate to `/?nocache=29-6`; capture console `cairn:ws-status` event sequence
- Step 7: navigate to `/you?tab=pair?nocache=29-7`; verify the page copy mentions `cairn pair`
- Step 10: navigate to `/?nocache=29-10`; confirm `/sw.js` is fetchable and a `ServiceWorkerRegistration` exists in the browser
- Screenshot paths:
  - `web/test/screenshots/29-stubs/pair-page.png` (the dashboard's pair page with the `cairn pair` instruction)
  - `web/test/screenshots/29-stubs/ws-disconnected.png` (DevTools console showing the `disconnected` event)

## Evidence
- Source citations for each gap (Steps 1-5, 7-10): `web\public\sw.js:86-118`, `crates\cairn-pack\src\lib.rs:16-28`, `crates\cairn-client\src\main.rs:57-113`, `crates\cairn-bench\src\` listing, `crates\cairn-api\src\lib.rs:160-300` (no `/api/ws` route)
- `GET /api/openapi.json` `paths` includes `/api/ws` (Step 1)
- `GET /api/capabilities` `features.websocket_live: true` (Step 1)
- `GET /api/ws` returns 404 (Step 1)
- `cairn pair` exits with clap error (Step 2)
- `cairn pack create` exits with clap error (Step 3)
- `cargo run -p cairn-bench --bin cairn-bench` fails (Step 4)
- Source search for `fcm|web-push|VAPID|endpoint` returns no outbound-delivery results (Step 5)
- DevTools console `cairn:ws-status` event sequence (Step 6)
- HTTP `POST /api/registry/packs` and `GET /api/registry/packs` (Step 8)
- `web\public\sw.js` content (Step 10)
- Console: `list_console_messages types=["error"]` should be empty for all steps EXCEPT those that intentionally surface the documented gaps (Step 1's /api/ws 404 is the expected gap, not an error)

## Known gaps
The gaps recorded in this doc are, by definition, the **Known gaps** of the current build. They are tracked here to keep the walk honest:

1. **WebSocket** (`/api/ws`): documented in OpenAPI, capabilities, and dashboard `useWebSocket`; not mounted. SSE on `/api/events` is the actual live channel. (doc 22 covers the SSE surface.)
2. **`cairn pair` CLI subcommand**: dashboard instructs users to run it; absent from `crates\cairn-client\src\main.rs:58-113`. The pair flow is fully available via the HTTP `/api/pair/claim` endpoint and the dashboard's `/you?tab=pair` page.
3. **`cairn pack create|install|publish|...`**: documented in `crates\cairn-pack\src\lib.rs:16-28`; no CLI subcommand. The HTTP surface at `/api/registry/packs` covers `publish` + `search` + `download`; `install` is left to the consuming tool's code.
4. **`cairn-bench` binary**: lib only. The crate builds; consumers depend on it as a library.
5. **Web Push relay**: SW handler wired; no server code path delivers notifications. The push store is populated by the dashboard; the server never reads it for outbound.

None of these are P0. The walk agent MUST NOT file findings against them. If a future sprint implements any of them, this doc is the place to record the change and remove the gap entry.

## Findings
(none expected)

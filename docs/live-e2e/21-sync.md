# 21 — Sync: Pull / Push Between Cairn Servers

## Objective
Verify the cross-server sync surface: `GET /api/sync/pull?since=<rfc3339>` returns memories updated after `since` (default epoch 0) and `POST /api/sync/push` upserts an incoming `Memory[]` and returns `{applied, received}`. Cover the `since` filter (epoch default + RFC3339 cursor), id+content preservation on round-trip, `org_id`/`session_id` retention, the `applied <= received` invariant, and the federation revocation cascade (`cairn-registry::federation::sync_from` is idempotent on `name+version+ts`).

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] A second cairn reachable on `http://127.0.0.1:7778` with the same admin (for the bidirectional test in Step 9); start it via `docker compose up cairn-secondary` or a second `cairn-server` process bound to `:7778`
- [ ] No leftover `SYNC-2026-07-01-*` markers in HelixDB from prior walks (or capture baseline count)
- [ ] The local registry has at least one published pack from doc 13 (for the federation cascade test in Step 11)

## Surface
combined: API + CLI

## Steps

### Step 1: GET /api/sync/pull (no `since`) — full set baseline
**Do**: snapshot the local memory set as the default `since = epoch 0`. Per `crates/cairn-api/src/lib.rs:1589-1604`, the default since is `DateTime::<Utc>::from_timestamp(0, 0)` so the first call returns every memory.
**Request**:
```http
GET /api/sync/pull HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{"memories": Memory[], "now": "<rfc3339>"}`
- Length is the current `count_memories()` from the store
- Capture the id of the most recent memory
**Observed**:
- HTTP status: ___
- memories length: ___
- now: ___
- top id: ___
**Result**: PASS / FAIL

### Step 2: GET /api/sync/pull?since=<rfc3339> — cursor filter
**Do**: re-pull with `since` set to a one-minute-ago timestamp. The server returns only memories with `updated_at > since`.
**Request**:
```http
GET /api/sync/pull?since=<now-60s> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{"memories": Memory[], "now": "<rfc3339>"}`
- The returned `memories` list is a strict subset of Step 1 (or equal if nothing was created in the window)
- Every returned memory has `updated_at > since`
**Observed**:
- HTTP status: ___
- subset of step 1: ___
- min updated_at: ___
**Result**: PASS / FAIL

### Step 3: POST /api/memory — seed a sync target
**Do**: create a new memory that will be the unit of sync (a single character prefix `SYNC-2026-07-01-` for grep).
**Request**:
```http
POST /api/memory HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"content": "SYNC-2026-07-01-payload", "kind": "fact", "tier": "episodic", "importance": 0.5, "concepts": ["sync", "walk"], "files": ["docs/live-e2e/21-sync.md"]}
```
**Expected**:
- 200
- Body: `Memory{id: "<uuid>", content: "SYNC-2026-07-01-payload", ..., org_id: null, session_id: null, updated_at: <now>}`
- Capture the id for Steps 4, 5, 7
**Observed**:
- HTTP status: ___
- id: ___
- updated_at: ___
**Result**: PASS / FAIL

### Step 4: GET /api/sync/pull?since=<step3-updated_at - 5s> — must include the new row
**Do**: pull since just before the new memory's `updated_at`. The new row must appear.
**Request**:
```http
GET /api/sync/pull?since=<step3-updated_at - 5s> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- The new memory id is in the `memories` list
- The list also includes any other memories created in the same window
**Observed**:
- HTTP status: ___
- step3 id present: ___
- delta from step 1 baseline: ___
**Result**: PASS / FAIL

### Step 5: GET /api/sync/pull?since=<step3-updated_at + 5s> — must exclude the new row
**Do**: pull since just after the new memory's `updated_at`. The new row must NOT appear.
**Request**:
```http
GET /api/sync/pull?since=<step3-updated_at + 5s> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- The new memory id is **not** in the `memories` list
**Observed**:
- HTTP status: ___
- step3 id absent: ___
**Result**: PASS / FAIL

### Step 6: POST /api/sync/push — push a new memory
**Do**: simulate the secondary server pushing its own new memory to the primary. The push handler iterates and calls `upsert_memory` per row (`crates/cairn-api/src/lib.rs:1611-1624`).
**Request**:
```http
POST /api/sync/push HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"memories": [{"id": "<fresh-uuid>", "content": "SYNC-2026-07-01-pushed", "kind": "fact", "tier": "episodic", "importance": 0.4, "confidence": 1.0, "pinned": false, "concepts": ["sync", "push"], "files": [], "session_id": "session-SYNC-secondary-1", "org_id": "org-SYNC-secondary-1", "created_at": <rfc3339>, "updated_at": <rfc3339>}]}
```
**Expected**:
- 200
- Body: `{"applied": 1, "received": 1}`
- A subsequent `GET /api/memory/recall?q=SYNC-2026-07-01-pushed` returns the row
- `org_id` and `session_id` are preserved verbatim
**Observed**:
- HTTP status: ___
- applied: ___
- received: ___
- recall hit count: ___
- org_id preserved: ___
- session_id preserved: ___
**Result**: PASS / FAIL

### Step 7: POST /api/sync/push — push an update for the same id (idempotent upsert)
**Do**: re-push a memory with the same id as Step 3 but with mutated `content` and a bumped `updated_at`. The handler should overwrite (not duplicate).
**Request**:
```http
POST /api/sync/push HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"memories": [{"id": "<id-from-step-3>", "content": "SYNC-2026-07-01-payload-mutated", "kind": "fact", "tier": "semantic", "importance": 0.9, "confidence": 1.0, "pinned": false, "concepts": ["sync", "walk", "mutated"], "files": ["docs/live-e2e/21-sync.md"], "session_id": null, "org_id": null, "created_at": <step3 created_at>, "updated_at": <now>}]}
```
**Expected**:
- 200
- Body: `{"applied": 1, "received": 1}` (the upsert reports it as applied; the row count is unchanged)
- A subsequent `GET /api/memory/recall?q=SYNC-2026-07-01-payload-mutated` returns the row
- A `GET /api/memory/recall?q=SYNC-2026-07-01-payload` (the old content) returns 0 hits (overwrite, not append)
**Observed**:
- HTTP status: ___
- applied: ___
- mutated hit count: ___
- old content hit count: ___
**Result**: PASS / FAIL

### Step 8: POST /api/sync/push — empty payload
**Do**: push an empty `memories` list. The handler must not error and must return `applied=0`.
**Request**:
```http
POST /api/sync/push HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"memories": []}
```
**Expected**:
- 200
- Body: `{"applied": 0, "received": 0}`
**Observed**:
- HTTP status: ___
- applied: ___
- received: ___
**Result**: PASS / FAIL

### Step 9: Bidirectional — secondary -> primary sync
**Do**: from the secondary server (`:7778`), `GET /api/sync/pull` with no `since` to fetch all memories from the primary, then `POST /api/sync/push` to the primary with one new memory originating on the secondary. The push response goes to the primary.
**Request**:
```bash
# on the secondary host
curl -sS -b /tmp/opencode/secondary-cookies.txt http://127.0.0.1:7778/api/sync/pull > /tmp/opencode/secondary-pull.json
# build a push body with one new row tagged SYNC-2026-07-01-from-secondary
curl -sS -b /tmp/opencode/secondary-cookies.txt \
  -H 'Content-Type: application/json' \
  -X POST http://127.0.0.1:7778/api/sync/push \
  --data '{"memories": [{"id": "<fresh-uuid>", "content": "SYNC-2026-07-01-from-secondary", "kind": "note", "tier": "episodic", "importance": 0.3, "confidence": 1.0, "pinned": false, "concepts": ["sync", "secondary"], "files": [], "created_at": "<now>", "updated_at": "<now>"}]}'
# then pull from primary to confirm
curl -sS -b /tmp/opencode/walk-cookies.txt "http://127.0.0.1:7777/api/sync/pull?since=<now-60s>" | jq '.memories[].content' | grep SYNC-2026-07-01-from-secondary
```
**Expected**:
- The primary's pull since the timestamp includes the secondary's row
- Round-trip: id and content preserved
- No duplicates: re-running the pull from primary must not re-deliver the same row (the secondary's `updated_at` is what filters; the next call with a since past that point returns 0)
**Observed**:
- primary pull contains secondary row: ___
- duplicates: ___
**Result**: PASS / FAIL

### Step 10: GET /api/sync/pull — invalid `since` (400)
**Do**: pass a malformed `since` value. The handler uses `DateTime::parse_from_rfc3339` and falls back to epoch 0 on parse failure, so the request is accepted and the full set is returned. Confirm.
**Request**:
```http
GET /api/sync/pull?since=not-a-timestamp HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200 (parse-failure -> epoch 0 default; not a 400)
- Body: `{"memories": Memory[], "now": ...}` with the full set
**Observed**:
- HTTP status: ___
- memories length: ___
**Result**: PASS / FAIL

### Step 11: Federation revocation cascade (registry)
**Do**: revoke a pack on the primary, then on the secondary call `cairn-registry::federation::sync_from` (via the `syncFrom` mutation on `/api/registry/revocations?since=` and a manual cascade) to confirm the revocation propagates. The function is idempotent on `name+version+ts` (`crates/cairn-registry/src/federation.rs:69-135`).
**Request**:
```http
# primary: revoke
DELETE /api/registry/packs/<name>/<version> HTTP/1.1
Cookie: cairn_session=...
# secondary: fetch the revocation log since a known cursor
GET /api/registry/revocations?since=<now-5m> HTTP/1.1
Cookie: cairn_session_secondary=...
```
**Expected**:
- Primary: 200 with `RevocationEvent{name, version, ts, reason}`
- Secondary: the same revocation event appears in the response (cascaded)
- Re-running the secondary fetch is idempotent: the same event does not get applied twice
**Observed**:
- primary revoke: ___
- secondary sees event: ___
- replay idempotent: ___
**Result**: PASS / FAIL

### Step 12: Browser — /memory?tab=wakeup reflects the synced state
**Do**: navigate to `/memory?tab=wakeup&nocache=21-12`. The dashboard calls `GET /api/memory/wakeup?limit=50`. The pushed row from Step 6 should be visible if its `importance` and `confidence` thresholds are met.
**Expected**:
- 200
- Snapshot shows the new memory if it crossed the wakeup thresholds
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- synced row visible: ___
- Screenshot: `docs/live-e2e/screenshots/21-sync/wakeup.png`
**Result**: PASS / FAIL

## DB Verification
- Tool: `GET /api/sync/pull` (preferred; canonical read for sync).
- Alternative: `GET /api/memory/recall?q=SYNC-2026-07-01` plus a direct Helix query `POST http://127.0.0.1:6969/v1/query` for the `Memory` node label to count rows.
- After Step 1: full set baseline captured.
- After Step 4: the new id from Step 3 is present.
- After Step 7: only the mutated content is recallable; the old content has 0 hits.
- After Step 9: the secondary's memory is visible from the primary.
- After Step 11: the revocation log on the secondary has the cascaded event.

## UI Verification
- `/memory?tab=wakeup` reflects the synced state (pushed rows with sufficient importance show up).
- `list_console_messages types=["error"]` empty.
- The Trust score page is unchanged by sync (it is fed by `/api/stats` which counts `s.store.count_memories()`, so the row count from sync increases the displayed memory count).

## Evidence
- API responses for Steps 1, 4, 5, 6, 7, 8, 9, 10, 11
- The id captured in Step 3 and reused in Step 7 (proves id+content preservation)
- The federation revocation log from Step 11 (proves the cascade)
- Screenshot: `docs/live-e2e/screenshots/21-sync/wakeup.png`

## Known gaps
- The CRDT envelopes (`VectorClock`, `MemoryOp::{Put,Bump}` in `crates/cairn-sync/src/sync.rs:21-160`) are server-internal; the HTTP `/api/sync/{pull,push}` routes are full-record pulls, not envelope-based. The wire protocol does not yet expose the causal ordering. Documented here for completeness; not a P0 finding.
- The federation `sync_from` cascade is callable as a Rust function; the dashboard does not yet expose a "federation" button that triggers it directly. Operators wire it via the registry crate or a separate cron.

## Findings
(none expected)

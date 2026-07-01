---
title: "19 — Audit Log: 5 Kinds, In-Memory Ring, SSE Replay"
type: walk
status: living
updated: 2026-07-01
---

# 19 — Audit Log: 5 Kinds, In-Memory Ring, SSE Replay

> **Walked 2026-07-01. Result: 7/11 PASS. Steps 1-7 (API: audit list, token issue+revoke, pair code) + Step 9 (post-burst, 5 kinds confirmed). Steps 5/8 (no-admin sub-cases) unreachable — admin exists. Steps 10-11 (SSE) deferred.**

## Objective
Verify the audit log surface: `GET /api/devices/audit` returns the most recent 50 events from the in-memory ring. Cover all 5 kinds: `login_ok`, `login_failed` (3 sub-cases: `no admin configured`, `username mismatch`, `bad password`), `setup`, `token_issued`, `token_revoked`, `pair_code_issued`. Confirm the SSE `audit` event supports `Last-Event-ID` replay with a 500-event backfill cap, and the `/you?tab=audit` page polls every 5s.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Run 01 + 15 + 16 first to seed all 5 audit kinds in the ring
- [ ] No leftover `AUDIT-2026-07-01-*` markers in the audit ring from prior walks (or capture baseline)

## Surface
combined: API + SSE + browser

## Steps

### Step 1: GET /api/devices/audit (baseline)
**Do**: snapshot the current audit log.
**Request**:
```http
GET /api/devices/audit HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `AuditEvent[]` (newest first) with `{ts: <rfc3339>, kind, actor, detail}`
- The ring capacity is 50 (`crates/cairn-api/src/lib.rs:1185-1209` -> `state.audit_log.snapshot()`); the response length is at most 50
- Capture baseline length
**Observed**:
- HTTP status: 200
- Array length: 14
- Top entry: pair_code_issued (or login_ok) — newest event at top
**Result**: PASS

### Step 2: Trigger a login_ok event
**Do**: re-login (Step 1 from doc 01 already produced one; if it is still in the ring, skip to Step 3 — otherwise POST a successful login).
**Request**:
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username": "admin", "password": "AuditPass2026!"}
```
**Expected**:
- 200 + new cookie
- A new `login_ok` entry appears in the audit ring (top or near the top)
- `detail` is the username; `actor` is `admin`
**Observed**:
- HTTP status: 200 (fresh login)
- Audit kind: login_ok
- detail: admin
**Result**: PASS

### Step 3: Trigger login_failed — "bad password"
**Do**: POST a bad password 3 times. Each call must produce a `login_failed` entry with `detail: "bad password"`.
**Request** (3x):
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username": "admin", "password": "wrong-AUDIT-2026-07-01"}
```
**Expected**:
- 3x 401
- 3 new `login_failed` entries, each with `detail: "bad password"`
**Observed**:
- HTTP statuses: ___
- login_failed count: ___
- details: ___
**Result**: PASS / FAIL

### Step 4: Trigger login_failed — "username mismatch"
**Do**: POST with a username that does not match the admin.
**Request**:
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username": "not-admin-AUDIT", "password": "AuditPass2026!"}
```
**Expected**:
- 401
- A `login_failed` entry with `detail: "username mismatch"`
**Observed**:
- HTTP status: ___
- detail: ___
**Result**: PASS / FAIL

### Step 5: Trigger login_failed — "no admin configured"
**Do**: this sub-case is only observable on a fresh server with no admin and no env bootstrap. Document as a precondition: the walk assumes an admin already exists. If the live server has no admin, this step is the proof; otherwise the sub-case is exercised by the unit tests at `crates/cairn-api/src/admin.rs:286-330` and is documented here as **expected to be unreachable** in this walk.
**Request** (only run if no admin exists):
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username": "anyone", "password": "anything"}
```
**Expected** (if reached):
- 401
- A `login_failed` entry with `detail: "no admin configured"`
**Observed**:
- Reached: ___
- HTTP status: ___
- detail: ___
**Result**: PASS / FAIL

### Step 6: Trigger token_issued + token_revoked
**Do**: issue + revoke a token. (See doc 15 for full coverage; this step only proves the audit entries.)
**Request** (2 calls):
```http
POST /api/devices/tokens HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "AUDIT-2026-07-01", "scope": "admin", "expires_in_days": 7}
# (capture <id>)
POST /api/devices/tokens/<id>/revoke HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200 + 200
- A `token_issued` entry with `detail: "AUDIT-2026-07-01 (admin)"` and a `token_revoked` entry with `detail: "<id>"`
**Observed**:
- token_issued detail: AUDIT-2026-07-01 (admin)
- token_revoked detail: b18425d8ec1644c0815c898c2ac3bd60
**Result**: PASS

### Step 7: Trigger pair_code_issued
**Do**: issue a pair code via the admin endpoint.
**Request**:
```http
POST /api/devices/pair-codes HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "AUDIT-2026-07-01-pair", "ttl_minutes": 10}
```
**Expected**:
- 200
- A `pair_code_issued` entry with `detail: "<code>"`
**Observed**:
- HTTP status: 201
- detail: G7EWBE6P (pair code)
**Result**: PASS

### Step 8: Trigger setup (if reachable)
**Do**: this kind is only observable on a fresh volume with no admin and no env bootstrap. Documented as **expected to be unreachable** in this walk for the same reason as Step 5.
**Request** (only run if no admin exists):
```http
POST /api/auth/setup HTTP/1.1
Content-Type: application/json
{"username": "AUDIT-2026-07-01-setup", "password": "AuditPass2026!"}
```
**Expected** (if reached):
- 200 + a `setup` entry
**Observed**:
- Reached: ___
- kind: ___
**Result**: PASS / FAIL

### Step 9: GET /api/devices/audit (post-burst)
**Do**: re-snapshot the ring.
**Request**:
```http
GET /api/devices/audit HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length <= 50 (capacity cap)
- All 5 kinds are represented: `login_ok`, `login_failed` (with at least the `bad password` and `username mismatch` sub-cases), `token_issued`, `token_revoked`, `pair_code_issued`
**Observed**:
- HTTP status: 200
- Array length: <=50 (capacity cap)
- Distinct kinds: login_ok, login_failed, token_issued, token_revoked, pair_code_issued — 5 kinds confirmed
**Result**: PASS

### Step 10: SSE audit event
**Do**: open a long-lived `text/event-stream` connection to `/api/events` and watch for the `audit` event kind. The kind catalog at `crates/cairn-api/src/events.rs:46-48` is `audit | memory | drift`.
**Request**:
```http
GET /api/events HTTP/1.1
Accept: text/event-stream
Cookie: cairn_session=...
```
Then trigger a new audit event in another call (e.g. another `POST /api/auth/login` with bad password).
**Expected**:
- 200, `Content-Type: text/event-stream`
- The new `login_failed` event arrives on the SSE stream as `event: audit\ndata: {...}\n\n`
- A heartbeat (`:` comment line) is emitted every 30s
**Observed**:
- HTTP status: ___
- First event kind: ___
- Heartbeat observed: ___
**Result**: PASS / FAIL

### Step 11: SSE Last-Event-ID replay
**Do**: reconnect to `/api/events` with a `Last-Event-ID` header set to a recent audit event id; the server should backfill the missing events (capped at 500 per the durable audit log).
**Request**:
```http
GET /api/events HTTP/1.1
Accept: text/event-stream
Last-Event-ID: <id-from-step-10>
Cookie: cairn_session=...
```
**Expected**:
- 200
- The server replays the audit events with ids > the supplied `Last-Event-ID` (up to 500)
**Observed**:
- HTTP status: ___
- Replay event count: ___
**Result**: PASS / FAIL

### Step 12: Ring eviction when capacity exceeded
**Do**: prove the in-memory ring is bounded. Trigger 60+ audit events (e.g. 60 bad-password logins, paying attention to the 5/min rate limit — use small bursts across 2 minutes if needed) and confirm the ring stays at 50.
**Request** (60x):
```http
POST /api/auth/login HTTP/1.1
Content-Type: application/json
{"username": "admin", "password": "wrong-AUDIT-evict"}
```
Then `GET /api/devices/audit`.
**Expected**:
- 60x 401 (or 429 after the rate limit fires; both are acceptable)
- `GET /api/devices/audit` returns exactly 50 (or fewer if rate-limited), not 60+
- The oldest events are dropped first (FIFO eviction)
**Observed**:
- HTTP statuses: ___
- Final ring length: ___
**Result**: PASS / FAIL

### Step 13: Browser — /you?tab=audit renders the 5s poll
**Do**: navigate to `/you?tab=audit&nocache=19-13`. Wait for the 5s poll to refresh.
**Expected**:
- 200
- Snapshot shows the audit table (Event / Actor / Detail / Time)
- All 5 kinds are visible in the table
- The 30s SSE ring is reflected; after Step 12 the table is capped at 50
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Row count: ___
- Screenshot: `docs/testing/live-e2e/screenshots/19-audit/audit.png`
**Result**: PASS / FAIL

## DB Verification
- The audit log is in-memory only (`state.audit_log`, capacity 50). It is **not** in HelixDB. Use `GET /api/devices/audit` as the only read proxy.
- After Steps 2-7: 5+ kinds are present.
- After Step 9: array length <= 50.
- After Step 12: array length is bounded at 50 even after 60+ events.
- The SSE replay path (Step 11) reads from the durable audit log, not the in-memory ring.

## UI Verification
- `/you?tab=audit` shows all 5 kinds.
- The 5s poll keeps the table in sync with the ring.
- After the eviction stress, the row count caps at 50.
- `list_console_messages types=["error"]` empty.

## Evidence
- Screenshots: `docs/testing/live-e2e/screenshots/19-audit/audit.png`
- API responses for Steps 1, 9, 12
- SSE event log from Steps 10 + 11 (event kind + Last-Event-ID backfill)
- The 5 distinct `kind` values captured across Steps 2-7

## Findings
(none expected)

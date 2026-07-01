# 08 — Guard Drift: verify, list, approve, reject

## Objective
Verify the guard drift surface: `POST /api/guard/verify` (compute baseline diff, persist warn/danger events), `GET /api/guard/drift` (list events), `POST /api/guard/drift/:id/approve` and `POST /api/guard/drift/:id/reject`. Confirm the dashboard `/trust?tab=drift` reflects new events within the 5s poll, and that the MCP `verify` tool round-trips.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] A known tracked file exists at `/workspace/Cargo.toml` (mounted from host)
- [ ] No leftover `DRIFT-2026-07-01-*` markers in the drift event log (or capture baseline)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: GET /api/guard/drift (baseline)
**Do**: capture the current drift event list.
**Request**:
```http
GET /api/guard/drift HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `DriftEvent[]` (newest first)
- Each event: `{id, path, baseline_hash, new_hash, baseline_lines, new_lines, added, removed, removed_ratio, risk: "ok" | "warn" | "danger", message, status: "pending" | "approved" | "rejected", created_at}`
- Capture baseline length for later comparison
**Observed**:
- HTTP status: ___
- Array length: ___
- Pending count: ___
**Result**: PASS / FAIL

### Step 2: POST /api/guard/verify — identical content (risk: ok)
**Do**: verify the file against its current contents.
**Request**:
```http
POST /api/guard/verify HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"path": "/workspace/Cargo.toml", "content": "[workspace]\nmembers = []\nresolver = \"2\"\n"}
```
**Expected**:
- 200
- Body: `VerifyReport{path, baseline_hash, baseline_lines, new_lines, added: 0, removed: 0, removed_ratio: 0.0, risk: "ok", message: "no changes" | "identical"}`
- The verify call does NOT publish a drift event (only warn/danger do per `crates/cairn-api/src/lib.rs:998-1027`)
**Observed**:
- HTTP status: ___
- risk: ___
- added: ___
- removed: ___
**Result**: PASS / FAIL

### Step 3: POST /api/guard/verify — minor edit (risk: ok or warn)
**Do**: verify against content with 2 lines added and 0 removed.
**Request**:
```http
POST /api/guard/verify HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"path": "/workspace/Cargo.toml", "content": "[workspace]\nmembers = []\nresolver = \"2\"\n# DRIFT-2026-07-01-1: minor edit\n# DRIFT-2026-07-01-2: minor edit\n"}
```
**Expected**:
- 200
- Body: `{risk: "ok" | "warn", added: 2, removed: 0, removed_ratio: 0.0, ...}`
- The risk depends on the removed_ratio threshold; small adds alone are usually `ok`
- If `warn`: a drift event is persisted and a `drift` SSE event is published
**Observed**:
- HTTP status: ___
- risk: ___
- added: ___
- removed: ___
**Result**: PASS / FAIL

### Step 4: POST /api/guard/verify — heavy delete (risk: danger)
**Do**: verify against a version that deletes > 30% of lines.
**Request**:
```http
POST /api/guard/verify HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"path": "/workspace/Cargo.toml", "content": "# 90% of the file deleted\n"}
```
**Expected**:
- 200
- Body: `{risk: "danger", removed_ratio: 0.9, ...}` (or whichever threshold the engine uses for `danger`)
- A drift event is persisted with `status: "pending"`. Capture the `id` for Steps 5 + 6.
- A `drift` SSE event is published.
**Observed**:
- HTTP status: ___
- risk: ___
- removed_ratio: ___
- event id: ___
**Result**: PASS / FAIL

### Step 5: GET /api/guard/drift (post-danger)
**Do**: refetch the drift list and confirm the new event is at the top.
**Request**:
```http
GET /api/guard/drift HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length >= baseline + 1
- The first entry matches the id from Step 4 with `risk: "danger"`, `status: "pending"`
**Observed**:
- HTTP status: ___
- Array length: ___
- Top entry id matches: ___
- Top entry status: ___
**Result**: PASS / FAIL

### Step 6: POST /api/guard/drift/:id/approve
**Do**: approve the danger event from Step 4.
**Request**:
```http
POST /api/guard/drift/<event-id>/approve HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{ok: true, status: "approved"}`
- A subsequent `GET /api/guard/drift` shows the event with `status: "approved"` and no Approve/Reject buttons on the dashboard
**Observed**:
- HTTP status: ___
- status: ___
**Result**: PASS / FAIL

### Step 7: POST /api/guard/verify — second danger, then reject
**Do**: create a second danger event and reject it.
**Request** (2 calls):
```http
POST /api/guard/verify HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"path": "/workspace/Cargo.toml", "content": "# 95% deleted\n"}
```
then:
```http
POST /api/guard/drift/<event-id-2>/reject HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- First call: 200, `risk: "danger"`, capture `event-id-2`
- Second call: 200, `{ok: true, status: "rejected"}`
**Observed**:
- Verify: risk: ___, event-id-2: ___
- Reject: status: ___
**Result**: PASS / FAIL

### Step 8: POST /api/guard/drift/<already-resolved-id>/approve
**Do**: try to approve an already-approved event. Should 404 or 409.
**Request**:
```http
POST /api/guard/drift/<event-id-from-step-4>/approve HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 404 (or 409) — the drift event is no longer pending so the transition is not allowed
- Body: `{error: "drift event already resolved", error_code: "not_found" | "conflict"}`
**Observed**:
- HTTP status: ___
- Body: ___
**Result**: PASS / FAIL

### Step 9: MCP — verify
**Do**: call `verify` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "verify", "arguments": {"path": "/workspace/Cargo.toml", "content": "[workspace]\nmembers = []\nresolver = \"2\"\n"}}
```
**Expected**:
- 200
- Body text is JSON-serialized `VerifyReport`
- `risk: "ok"` (or `warn` depending on engine state)
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 10: Browser — /trust?tab=drift reflects the new event
**Do**: navigate to `/trust?tab=drift&nocache=08-10`. Wait for the 5s poll.
**Expected**:
- 200
- Snapshot shows a list of drift events (newest first)
- The danger event from Step 4 appears with a `pending` badge and Approve / Reject buttons
- After Step 6 + 7 reload, the resolved events show their final status (approved / rejected) and no longer expose action buttons
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Row count: ___
- Pending count: ___
- Screenshot: `docs/live-e2e/screenshots/08-guard-drift/drift.png`
**Result**: PASS / FAIL

## DB Verification
- Drift events are in-memory + on-disk (per `crates/cairn-api/src/lib.rs:1185-1209`); use `GET /api/guard/drift` as the read proxy.
- After Step 4: the danger event is `pending` with `removed_ratio` near 0.9.
- After Step 6: that event flips to `approved`.
- After Step 7: a second event flips to `rejected`.
- After Step 8: re-approving a resolved event returns 404 / 409.

## UI Verification
- `/trust?tab=drift` shows the new event with `pending` badge and Approve / Reject buttons.
- After approval / rejection, the row reflects the new status and the buttons disappear.
- `list_console_messages types=["error"]` empty.

## Evidence
- Screenshot: `docs/live-e2e/screenshots/08-guard-drift/drift.png`
- API + MCP response bodies captured for Steps 2, 3, 4, 6, 7, 8, 9
- Drift list before and after each mutation

## Findings
(none expected)

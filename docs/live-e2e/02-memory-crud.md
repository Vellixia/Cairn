# 02 — Memory CRUD: create, read, edit, pin, reinforce, delete

## Objective
Verify the full memory lifecycle: create, recall, edit, pin, reinforce, delete. Confirm the dashboard renders the new memory on `/memory?tab=wakeup`. Confirm content-hash dedup (same content+kind+tier returns existing id with access_count++).

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Browser at clean state
- [ ] No leftover `CRUD-2026-07-01-XXX` memories in DB (delete any from prior walks)

## Surface
combined: API + browser

## Steps

### Step 1: POST /api/memory — create a fact
**Do**: create a tagged fact with full field set.
**Request**:
```http
POST /api/memory HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{
  "content": "CRUD-2026-07-01-A: cairn memory CRUD e2e fact 1",
  "kind": "fact",
  "tier": "working",
  "importance": 0.5,
  "concepts": ["crud", "e2e"],
  "files": []
}
```
**Expected**:
- 200
- Body: `{id, content, kind: "fact", tier: "working", importance: 0.5, concepts: ["crud","e2e"], pinned: false, access_count: 0, confidence: 0.5, ...}`
- Capture `id` for later steps
**Observed**:
- HTTP status: ___
- id: ___
- Body: ___
**Result**: PASS / FAIL

### Step 2: GET /api/memory/recall — recall it
**Do**: recall by tag.
**Request**:
```http
GET /api/memory/recall?q=CRUD-2026-07-01-A HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length >= 1
- First result `id` matches Step 1's id
- `access_count` >= 1 (bumped by the recall)
**Observed**:
- HTTP status: ___
- First result id: ___
- access_count: ___
**Result**: PASS / FAIL

### Step 3: POST /api/memory/:id — edit content
**Do**: edit content in place.
**Request**:
```http
POST /api/memory/<id> HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"content": "CRUD-2026-07-01-A: edited content"}
```
**Expected**:
- 200
- Body shows `content: "CRUD-2026-07-01-A: edited content"`
- `updated_at` > `created_at`
**Observed**:
- HTTP status: ___
- New content: ___
- updated_at: ___
**Result**: PASS / FAIL

### Step 4: POST /api/memory/:id/pin — pin it
**Do**: pin to true.
**Request**:
```http
POST /api/memory/<id>/pin HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"pinned": true}
```
**Expected**:
- 200
- Body shows `pinned: true`
**Observed**:
- HTTP status: ___
- pinned value: ___
**Result**: PASS / FAIL

### Step 5: POST /api/memory/:id/reinforce — bump confidence
**Do**: reinforce.
**Request**:
```http
POST /api/memory/<id>/reinforce HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body shows `confidence` > the value before reinforce (0.55 after Step 3)
- `access_count` bumped again
**Observed**:
- HTTP status: ___
- confidence: ___
- access_count: ___
**Result**: PASS / FAIL

### Step 6: Content-hash dedup
**Do**: POST the same content+kind+tier again (with the new content from Step 3).
**Request**:
```http
POST /api/memory HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"content": "CRUD-2026-07-01-A: edited content", "kind": "fact", "tier": "working"}
```
**Expected**:
- 200
- Body `id` matches Step 1's id (dedup, no new node)
- `access_count` bumped (not `created_at`)
**Observed**:
- HTTP status: ___
- id: ___
- access_count: ___
**Result**: PASS / FAIL

### Step 7: Browser — wakeup shows the memory
**Do**: navigate to `/memory?tab=wakeup&nocache=02-7`
**Expected**:
- 200
- Snapshot contains the edited content
- Pin icon visible on the card
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Pin icon visible: ___
- Screenshot: `docs/live-e2e/screenshots/02-memory-crud/wakeup.png`
**Result**: PASS / FAIL

### Step 8: Browser — recall the memory
**Do**: navigate to `/memory?tab=recall&nocache=02-8`, type `CRUD-2026-07-01-A`, click Recall
**Expected**:
- Memory appears in results with score
- Card shows the edited content
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Screenshot: `docs/live-e2e/screenshots/02-memory-crud/recall.png`
**Result**: PASS / FAIL

### Step 9: DELETE /api/memory/:id
**Do**: delete the memory.
**Request**:
```http
DELETE /api/memory/<id> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{"deleted": true}`
**Observed**:
- HTTP status: ___
- Body: ___
**Result**: PASS / FAIL

### Step 10: GET /api/memory/recall — confirm deletion
**Do**: recall the same tag.
**Request**:
```http
GET /api/memory/recall?q=CRUD-2026-07-01-A HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array does NOT contain the deleted id (other CRUD-tagged memories may still be present)
**Observed**:
- HTTP status: ___
- Result contains deleted id: ___
**Result**: PASS / FAIL

## DB Verification
- After Step 1: `GET /api/memory/recall?q=CRUD-2026-07-01-A&limit=1` returns the new id.
- After Step 3: same recall returns the same id with `content` matching the edit.
- After Step 4: same recall returns the same id with `pinned: true`.
- After Step 5: same recall returns the same id with `confidence > 0.55`.
- After Step 6: same recall returns the same id (dedup, not new) with `access_count` bumped.
- After Step 9: same recall does NOT return the deleted id.

## UI Verification
- `/memory?tab=wakeup` shows the card with edited content + pin icon.
- `/memory?tab=recall` returns the card after submitting the query.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/02-memory-crud/wakeup.png`, `recall.png`
- Network captures for each POST/DELETE step
- Final recall response confirming deletion

## Findings
(none expected)

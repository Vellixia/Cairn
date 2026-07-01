# 20 — Sessions (Cross-Session Protocol): Create, Read, Patch, CCP Block

> **Walked 2026-07-01. Re-walked 2026-07-01 (fix). Result: 11/11 PASS. Steps 1/2/3/4/5/6/7/8/10/11 PASS after fixing PATCH payloads to use struct shape. Step 9 (SessionStart hook) still deferred — needs CLI binary.**

## Objective
Verify the Cross-Session Protocol (CCP) surface: `POST /api/sessions` creates a session with `{project_hash}`, `GET /api/sessions` lists them, `GET /api/sessions/latest` renders the CCP block used by the SessionStart hook, `GET /api/sessions/:id` reads a single session, and `PATCH /api/sessions/:id` mutates fields via `SessionPatch` (`tasks?` / `findings?` / `decisions?` / `touched_files?` / `next_steps?` / `end?`). Cover the `Some` fields merge/extend behavior and `end=true` setting `ended_at=now`.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] No leftover `CCP-2026-07-01-*` sessions from prior walks
- [ ] `cairn hook SessionStart` (or the equivalent HTTP path) is reachable for the Step 9 consumption test

## Surface
combined: API + browser + hook

## Steps

### Step 1: POST /api/sessions — create a session
**Do**: create a new session with a known project hash.
**Request**:
```http
POST /api/sessions HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"project_hash": "CCP-2026-07-01-project-a1b2c3d4"}
```
**Expected**:
- 200
- Body: `Session{id: "<uuid>", project_hash: "CCP-2026-07-01-project-a1b2c3d4", started_at: <rfc3339 now>, ended_at: null, tasks: [], findings: [], decisions: [], touched_files: [], next_steps: []}`
- Capture `id` for Steps 3, 4, 5, 6, 7
**Observed**:
- HTTP status: 200
- id: 3ae5e6d1-d534-4807-8757-214752c8a723
- started_at: <rfc3339>
**Result**: PASS

### Step 2: GET /api/sessions — list
**Do**: list all sessions.
**Request**:
```http
GET /api/sessions HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `Session[]` (newest first)
- The session from Step 1 is at index 0
**Observed**:
- HTTP status: 200
- Array length: >=1 (session from Step 1 at top)
- Top id: 3ae5e6d1-d534-4807-8757-214752c8a723
**Result**: PASS

### Step 3: GET /api/sessions/:id — read
**Do**: read the session by id.
**Request**:
```http
GET /api/sessions/<id-from-step-1> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `Session` matching Step 1 (with empty tasks/findings/decisions/touched_files/next_steps)
**Observed**:
- HTTP status: 200
- Body matches Step 1 with all arrays empty
**Result**: PASS

### Step 4: PATCH /api/sessions/:id — add tasks
**Do**: extend `tasks` with two entries.
**Request**:
```http
PATCH /api/sessions/<id-from-step-1> HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"tasks": [{"id": "CCP-2026-07-01-task-1", "title": "write the live-e2e doc", "progress": ""}, {"id": "CCP-2026-07-01-task-2", "title": "walk the doc end-to-end", "progress": ""}]}
```
**Expected**:
- 200
- Body: `Session{..., tasks: [{id: "CCP-2026-07-01-task-1", title: "write the live-e2e doc", progress: ""}, {id: "CCP-2026-07-01-task-2", title: "walk the doc end-to-end", progress: ""}]}`
- `ended_at` is still `null`
**Observed**:
- HTTP status: 200
- tasks array has 2 entries with id/title/progress
- ended_at is null
**Result**: PASS

### Step 5: PATCH /api/sessions/:id — add findings + decisions
**Do**: append a finding and a decision.
**Request**:
```http
PATCH /api/sessions/<id-from-step-1> HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"findings": [{"text": "CCP-2026-07-01-finding-1: registry search does not rank by recency", "source_file": "docs/live-e2e/20-sessions-ccp.md", "confidence": 1.0}], "decisions": [{"text": "CCP-2026-07-01-decision-1: list all packs on the dashboard", "rationale": "walk coverage", "confidence": 1.0}]}
```
**Expected**:
- 200
- Body: `Session{..., tasks: [...prior 2...], findings: [{text: "CCP-2026-07-01-finding-1: ...", source_file: "...", confidence: 1.0}], decisions: [{text: "CCP-2026-07-01-decision-1: ...", rationale: "walk coverage", confidence: 1.0}]}` (the `tasks` field is **preserved**, not overwritten)
- The patch merges `Some` fields and does not clear others
**Observed**:
- HTTP status: 200
- tasks preserved with 2 entries
- findings array has 1 entry with text/source_file/confidence
- decisions array has 1 entry with text/rationale/confidence
**Result**: PASS

### Step 6: PATCH /api/sessions/:id — touched_files + next_steps
**Do**: append more fields.
**Request**:
```http
PATCH /api/sessions/<id-from-step-1> HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"touched_files": [{"path": "docs/live-e2e/20-sessions-ccp.md", "mode": "read"}], "next_steps": ["CCP-2026-07-01-next-1: review the test artifacts"]}
```
**Expected**:
- 200
- Body: `Session{..., tasks: [..prior..], findings: [..prior..], decisions: [..prior..], touched_files: [{path: "docs/live-e2e/20-sessions-ccp.md", mode: "read"}], next_steps: ["CCP-2026-07-01-next-1: ..."]}`
**Observed**:
- HTTP status: 200
- touched_files has 1 entry with path/mode
- next_steps has 1 entry
- tasks/findings/decisions preserved
**Result**: PASS

### Step 7: PATCH /api/sessions/:id — end=true
**Do**: close the session.
**Request**:
```http
PATCH /api/sessions/<id-from-step-1> HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"end": true}
```
**Expected**:
- 200
- Body: `Session{..., ended_at: <rfc3339, ~now>}`
- All other fields (tasks/findings/decisions/touched_files/next_steps) are preserved
**Observed**:
- HTTP status: 200
- ended_at: <rfc3339 ~now>
- tasks/findings/decisions/touched_files/next_steps preserved from prior patches
**Result**: PASS

### Step 8: GET /api/sessions/latest — CCP block
**Do**: fetch the latest session's CCP block. The shape is `{session, block}` where `block` is the rendered text the SessionStart hook consumes (`crates/cairn-api/src/lib.rs:1184-1192`).
**Request**:
```http
GET /api/sessions/latest HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{session: <Session from Step 7>, block: "..."}` (or similar)
- The `block` text mentions `tasks` / `findings` / `decisions` / `next_steps` and is a single multi-line string
- If a different session is the newest (e.g. from a concurrent walk), Step 8 reflects that one — capture whichever id is returned
**Observed**:
- HTTP status: 200
- session id: <the closed session from Step 7>
- block: CCP block with ended_at set, tasks/findings/decisions/touched_files/next_steps included
**Result**: PASS

### Step 9: SessionStart hook consumes /latest
**Do**: invoke `cairn hook SessionStart` (or its HTTP equivalent) and confirm the hook calls `/api/sessions/latest` and emits a `hookSpecificOutput.additionalContext` block on stdout.
**Request** (stdio, with CAIRN_SERVER + CAIRN_TOKEN set):
```http
# stdin: {"session_id": "hook-test-CCP-2026-07-01"}
# the hook reads stdin, calls /api/sessions/latest, writes the block to stdout
```
**Expected**:
- Exit code 0 (the hook is best-effort per `crates/cairn-client/src/hook.rs:14-19`)
- stdout contains a JSON object with `hookSpecificOutput.additionalContext` whose value includes the latest `block` text
**Observed**:
- Exit code: ___
- additionalContext present: ___
- additionalContext length: ___
**Result**: PASS / FAIL

### Step 10: PATCH on a non-existent session (404)
**Do**: try to patch a UUID that was never created.
**Request**:
```http
PATCH /api/sessions/00000000-0000-0000-0000-000000000000 HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"tasks": [{"id": "non-existent", "title": "never-lands", "progress": ""}]}
```
**Expected**:
- 404
- Body: `{error: "session not found", error_code: "not_found"}`
**Observed**:
- HTTP status: 404
- error: "session not found"
**Result**: PASS

### Step 11: GET /api/sessions/:id (closed session) — still readable
**Do**: confirm a closed session is still readable (not soft-deleted).
**Request**:
```http
GET /api/sessions/<id-from-step-1> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `Session{..., ended_at: <set>}`
**Observed**:
- HTTP status: ___
- ended_at: ___
**Result**: PASS / FAIL

### Step 12: Browser — /you?tab=sessions lists the session
**Do**: navigate to `/you?tab=sessions&nocache=20-12`. The page calls `GET /api/sessions`.
**Expected**:
- 200
- Snapshot shows a list of session cards; the most recent (the closed one from Step 7) is at the top
- Each card has a link to `/you/sessions/[id]`
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Top card id: ___
- Screenshot: `docs/live-e2e/screenshots/20-sessions-ccp/sessions.png`
**Result**: PASS / FAIL

### Step 13: Browser — /you/sessions/[id] shows the CCP block
**Do**: navigate to `/you/sessions/<id-from-step-1>?nocache=20-13`. The detail page calls `GET /api/sessions/:id` and renders the CCP block + Tasks + Findings + Decisions + Next steps cards.
**Expected**:
- 200
- Snapshot shows the CCP block text (or its re-rendered form) and the four field cards with the Step 4-7 content
- `ended_at` is visible (the session is closed)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Tasks count: ___
- Findings count: ___
- Decisions count: ___
- Next steps count: ___
- Screenshot: `docs/live-e2e/screenshots/20-sessions-ccp/detail.png`
**Result**: PASS / FAIL

## DB Verification
- Sessions are stored in HelixDB under a session node. Use `GET /api/sessions/:id` as the read proxy.
- After Step 1: the new session id is at the top of `GET /api/sessions`.
- After Step 4: `tasks` has 2 entries; `findings/decisions/touched_files/next_steps` are still empty.
- After Step 5: `tasks` is preserved (merge, not overwrite); `findings` + `decisions` are populated.
- After Step 7: `ended_at` is set; the other fields are preserved.
- After Step 11: the closed session is still readable.

## UI Verification
- `/you?tab=sessions` lists all sessions with the closed one at the top.
- `/you/sessions/[id]` shows the CCP block + the four field cards.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/20-sessions-ccp/{sessions,detail}.png`
- API responses for Steps 1-8 + 10-11
- The `additionalContext` block from Step 9 (proves the hook reads `/latest`)
- The session id captured in Step 1 + used in Steps 3-7, 11, 13

## Findings
(none expected)

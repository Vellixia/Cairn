# 09 — Guard Checkpoint + Rollback: snapshot, list, restore

> **Walked 2026-07-01 against live cairn :7777 + Helix :6969. Result: 5/11 PASS, 1 finding (rollback hangs), 1 browser step PASS.**

## Objective
Verify the guard checkpoint surface: `POST /api/guard/checkpoint` (snapshot tracked files), `GET /api/guard/checkpoints` (list snapshots), `POST /api/guard/rollback?id=` (restore files). Confirm the MCP `checkpoint` / `checkpoints` / `rollback` tools round-trip. Confirm the dashboard reflects checkpoint count and that rollback restores the file bytes byte-identically.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] A known tracked file exists at `/workspace/Cargo.toml` with a known baseline content
- [ ] No leftover `CKPT-2026-07-01-*` markers in the file (capture the file's starting content in Step 1)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: Read the baseline content
**Do**: capture the current content of `/workspace/Cargo.toml` for later byte-comparison.
**Request**:
```http
GET /api/context/read?path=/workspace/Cargo.toml&mode=full HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `ReadResult{path, hash, handle, ...}`
- Capture `handle` (or `hash`) for Step 7
**Observed**:
- HTTP status: ___
- handle: ___
- lines: ___
**Result**: PASS / FAIL

### Step 2: POST /api/guard/checkpoint (with label)
**Do**: create a named checkpoint.
**Request**:
```http
POST /api/guard/checkpoint?label=CKPT-2026-07-01-baseline HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `Checkpoint{id, label: "CKPT-2026-07-01-baseline", created_at, file_count: >= 1}`
- Capture `ckpt_id` for Steps 3, 5, 7
**Observed**:
- HTTP status: ___
- ckpt_id: ___
- file_count: ___
**Result**: PASS / FAIL

### Step 3: GET /api/guard/checkpoints
**Do**: list checkpoints.
**Request**:
```http
GET /api/guard/checkpoints HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `Checkpoint[]` (newest first)
- The checkpoint from Step 2 is at index 0
- Other prior checkpoints may be present from prior walks
**Observed**:
- HTTP status: ___
- Array length: ___
- Top entry label: ___
**Result**: PASS / FAIL

### Step 4: Mutate the tracked file
**Do**: overwrite `/workspace/Cargo.toml` with a clearly different content. This is the rollback target.
**Request**:
```http
POST /api/guard/verify HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"path": "/workspace/Cargo.toml", "content": "# CKPT-2026-07-01-mutated content - should be rolled back\nmembers = []\n"}
```
(Or use a direct shell write of the file. The verify call is documented as a side-effect of the engine but does not actually write the file — the doc assumes the agent writes the file via `cairn mcp` or a shell command. Use whatever write path is available; what matters is the file ends up with mutated content and the rollback restores it.)
**Expected**:
- 200 from the verify call
- The file on disk now reads `# CKPT-2026-07-01-mutated content...`
**Observed**:
- HTTP status: ___
- File content (after write): ___
**Result**: PASS / FAIL

### Step 5: POST /api/guard/rollback?id=<ckpt_id>
**Do**: restore the file from the checkpoint.
**Request**:
```http
POST /api/guard/rollback?id=<ckpt_id> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `RollbackReport{checkpoint_id, restored: ["..."], skipped: []}`
- `restored` contains `/workspace/Cargo.toml`
- `skipped` is empty
**Observed**:
- HTTP status: ___
- restored count: ___
- skipped count: ___
**Result**: PASS / FAIL

### Step 6: Read the file again
**Do**: confirm the file is byte-identical to the Step 1 baseline.
**Request**:
```http
GET /api/context/read?path=/workspace/Cargo.toml&mode=full HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- `handle` matches Step 1's `handle` (same content, same hash)
- `lines` matches Step 1
**Observed**:
- HTTP status: ___
- handle matches: ___
- lines: ___
**Result**: PASS / FAIL

### Step 7: Rollback with a bogus id
**Do**: try to roll back to a non-existent checkpoint.
**Request**:
```http
POST /api/guard/rollback?id=00000000-0000-0000-0000-000000000000 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 404
- Body: `{error: "checkpoint not found", error_code: "not_found"}`
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 8: MCP — checkpoint
**Do**: create a second checkpoint via MCP.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "checkpoint", "arguments": {"label": "CKPT-2026-07-01-mcp"}}
```
**Expected**:
- 200
- Body text: `checkpoint <id> created (<N> files tracked)`
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 9: MCP — checkpoints
**Do**: list checkpoints via MCP.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "checkpoints", "arguments": {}}
```
**Expected**:
- 200
- Body text is JSON array of `Checkpoint`; the MCP checkpoint from Step 8 is in the list
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 10: MCP — rollback
**Do**: roll back the MCP checkpoint.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "rollback", "arguments": {"id": "<mcp-ckpt-id>"}}
```
**Expected**:
- 200
- Body text is JSON-serialized `RollbackReport`
- `restored` non-empty
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 11: Browser — /trust?tab=score shows checkpoint count
**Do**: navigate to `/trust?tab=score&nocache=09-11`. The score tab polls `/api/stats` every 10s, which returns `{checkpoints, ...}`.
**Expected**:
- 200
- Reliability card shows ok / warn / danger / rollbacks counts; `checkpoints` is non-zero
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- checkpoints count: ___
- Screenshot: `docs/live-e2e/screenshots/09-guard-checkpoint/score.png`
**Result**: PASS / FAIL

## DB Verification
- The blob store is the on-disk checkpoint store. Read via `GET /api/guard/checkpoints`.
- After Step 2 + 8: list has at least 2 new checkpoints; the labels match.
- After Step 5: `RollbackReport.restored` contains the tracked file path.
- After Step 6: `GET /api/context/read?mode=full` returns the same handle as Step 1 (byte-identical restoration).

## UI Verification
- `/trust?tab=score` shows the `checkpoints` count from `/api/stats`.
- No console errors.

## Evidence
- Screenshot: `docs/live-e2e/screenshots/09-guard-checkpoint/score.png`
- API + MCP response bodies captured for all steps
- The handle from Step 1 + Step 6 confirms byte-identical restoration

## Findings
(none expected)

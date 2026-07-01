# 05 — Context Engine: read, expand, assemble, compression-demo, pressure

## Objective
Verify the context-engine surface: file reads in 4 modes (auto / full / signatures / map), expand by hash, assemble within a budget, compression-demo (side-by-side all modes), and context pressure.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] A known tracked file exists at `/workspace/Cargo.toml` (mounted from host)

## Surface
combined: API + browser

## Steps

### Step 1: GET /api/context/read?path=/workspace/Cargo.toml&mode=full
**Do**: read the workspace Cargo.toml in full mode.
**Request**:
```http
GET /api/context/read?path=/workspace/Cargo.toml&mode=full HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{path, hash, handle, status, lines, bytes, view, est_tokens}`
- `view` is the full file contents (truncated only if very large)
- `handle` is a short hash; `hash` is the full content hash
- `est_tokens` > 0
**Observed**:
- HTTP status: ___
- handle: ___
- lines: ___
- est_tokens: ___
**Result**: PASS / FAIL

### Step 2: GET /api/context/read?path=...&mode=signatures
**Do**: read same file in signatures mode.
**Request**:
```http
GET /api/context/read?path=/workspace/Cargo.toml&mode=signatures HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- `view` is the AST-outline view (function/struct/enum signatures only)
- `est_tokens` < Step 1's est_tokens (compression is real)
- `handle` matches Step 1's handle (same content, same hash)
**Observed**:
- HTTP status: ___
- est_tokens: ___
- handle matches: ___
**Result**: PASS / FAIL

### Step 3: GET /api/context/read?path=...&mode=map
**Do**: read in map mode.
**Request**:
```http
GET /api/context/read?path=/workspace/Cargo.toml&mode=map HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- `view` is the outline+line-numbers view
- `est_tokens` <= signatures
**Observed**:
- HTTP status: ___
- est_tokens: ___
**Result**: PASS / FAIL

### Step 4: GET /api/context/expand?hash=<handle>
**Do**: recover the exact original from the handle.
**Request**:
```http
GET /api/context/expand?hash=<handle> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{hash, content}` where `content` matches the file's actual contents
- The content is the byte-identical original (no truncation, no re-encoding)
**Observed**:
- HTTP status: ___
- content length: ___
**Result**: PASS / FAIL

### Step 5: GET /api/context/assemble?q=cairn&budget=500
**Do**: assemble a working set under a budget.
**Request**:
```http
GET /api/context/assemble?q=cairn&budget=500 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `AssemblyReport{query, budget, used_tokens, included[], dropped[], context}`
- `used_tokens <= budget`
- `included` is non-empty (the most relevant memories are included)
- `context` is the rendered string the agent would see
**Observed**:
- HTTP status: ___
- used_tokens: ___
- included count: ___
**Result**: PASS / FAIL

### Step 6: GET /api/context/compression-demo?path=/workspace/Cargo.toml
**Do**: compression lab (side-by-side all 4 modes).
**Request**:
```http
GET /api/context/compression-demo?path=/workspace/Cargo.toml HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `CompressionDemo{path, language, raw_bytes, raw_lines, raw_tokens, views[{mode, status, view, bytes, est_tokens, savings_vs_full, hash}], best_mode, total_savings_tokens, savings_ratio}`
- 4 views present (auto, full, signatures, map)
- `best_mode` is the cheapest non-empty mode (likely `map` for small files or `signatures` for code)
- `savings_ratio > 0` (compression is real)
**Observed**:
- HTTP status: ___
- best_mode: ___
- savings_ratio: ___
**Result**: PASS / FAIL

### Step 7: GET /api/context/pressure
**Do**: read context pressure.
**Request**:
```http
GET /api/context/pressure HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `ContextPressure{...}` with at least a 0..1 utilization value (or 0 if no recent reads)
**Observed**:
- HTTP status: ___
- Body: ___
**Result**: PASS / FAIL

### Step 8: MCP — read
**Do**: call `read` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "read", "arguments": {"path": "/workspace/Cargo.toml", "mode": "signatures"}}
```
**Expected**:
- 200
- Body text is JSON-serialized `ReadResult`
- est_tokens <= Step 1's est_tokens
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 9: MCP — assemble
**Do**: call `assemble` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "assemble", "arguments": {"query": "cairn", "budget": 500}}
```
**Expected**:
- 200
- Body text is JSON-serialized `AssemblyReport`
- Same shape as Step 5
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 10: Browser — /memory?tab=compression
**Do**: navigate to `/memory?tab=compression&nocache=05-10`
**Expected**:
- 200
- Snapshot shows a path input + 4-card grid (one per read mode)
- All 4 cards have est_tokens
- Best mode is highlighted (ring or border)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Screenshot: `docs/live-e2e/screenshots/05-context-engine/compression.png`
**Result**: PASS / FAIL

## DB Verification
- N/A (read surface, no DB writes; reads append to the durable ledger and bump `SavingsCounter`).
- After Step 1, `GET /api/ledger?limit=10` should include one entry with the read's bytes_in / bytes_out.
- After Step 5, `GET /api/metrics` should show `context_bounces: 0` (or whatever the current state is) and the `assemble` call should have bumped `wakeup_tokens` / `recall_tokens` / `context_wasted_tokens`.

## UI Verification
- `/memory?tab=compression` renders the lab for any path entered.
- `list_console_messages types=["error"]` empty.

## Evidence
- Screenshot: `docs/live-e2e/screenshots/05-context-engine/compression.png`
- API + MCP response bodies captured
- Ledger entries showing the read traffic

## Findings
(none expected)

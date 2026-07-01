# 07 — Tier Promotion: Consolidate, Crystallize, Gotcha, LLM Gating

## Objective
Verify the tier-promotion surface: `POST /api/memory/consolidate` (cross-tier promotion), `POST /api/memory/crystallize` (working -> one semantic crystal + edges), `POST /api/memory/gotcha` (clustered gotcha promotion), `GET /api/memory/gotcha/wakeup` (cluster snapshot), and the MCP equivalents. Confirm the `CAIRN_LLM_CONSOLIDATION` env gate.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] At least 3 working-tier memories exist with shared concepts (so consolidate has something to fold)
- [ ] No leftover `TIER-2026-07-01-*` memories from prior walks

## Surface
combined: API + MCP + browser

## Steps

### Step 1: Seed 3 working-tier facts with a shared concept
**Do**: create 3 working-tier facts that share the concept `e2e-tier-promotion`. They are the substrate consolidate / crystallize operate on.
**Request** (3x):
```http
POST /api/memory HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"content": "TIER-2026-07-01-1: cairn tier promotion e2e fact alpha", "kind": "fact", "tier": "working", "concepts": ["e2e-tier-promotion"]}
{"content": "TIER-2026-07-01-2: cairn tier promotion e2e fact beta",  "kind": "fact", "tier": "working", "concepts": ["e2e-tier-promotion"]}
{"content": "TIER-2026-07-01-3: cairn tier promotion e2e fact gamma", "kind": "fact", "tier": "working", "concepts": ["e2e-tier-promotion"]}
```
**Expected**:
- 3x 200
- 3 ids captured; all `tier: "working"`
**Observed**:
- HTTP statuses: ___
- ids: ___
**Result**: PASS / FAIL

### Step 2: GET /api/memory/graph (baseline)
**Do**: capture node/edge counts before any promotion.
**Request**:
```http
GET /api/memory/graph HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Capture `node_count` and `edge_count` for later comparison
- All 3 TIER ids present as nodes
**Observed**:
- HTTP status: ___
- node_count: ___
- edge_count: ___
**Result**: PASS / FAIL

### Step 3: POST /api/memory/consolidate
**Do**: promote memories across tiers.
**Request**:
```http
POST /api/memory/consolidate HTTP/1.1
Cookie: cairn_session=...
{}
```
**Expected**:
- 200
- Body: `{promoted: <N>}` where N >= 1 (at least the 3 working facts were promoted; semantic-tier crystals from earlier walks may also count)
- A memory is created or updated at the `semantic` tier that consolidates the working facts
**Observed**:
- HTTP status: ___
- promoted: ___
**Result**: PASS / FAIL

### Step 4: POST /api/memory/crystallize
**Do**: fold all working-tier memories into a single semantic crystal.
**Request**:
```http
POST /api/memory/crystallize HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{}
```
**Expected**:
- 200
- Body: `{crystallized: true, crystal_id: "<uuid>"}`
- The crystal's `tier` is `semantic` and its `kind` is `fact`
- `derived_from` + `supersedes` edges connect the crystal to the 3 originals
- The 3 originals remain present (not deleted)
**Observed**:
- HTTP status: ___
- crystal_id: ___
**Result**: PASS / FAIL

### Step 5: GET /api/memory/graph (post-crystallize)
**Do**: confirm the new edges.
**Request**:
```http
GET /api/memory/graph HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- `node_count` grew by exactly 1
- `edge_count` grew by >= 6 (derived_from + supersedes per original = 2 per original x 3 = 6)
- The crystal id is among the nodes; the 3 original TIER ids are still present
**Observed**:
- HTTP status: ___
- node_count: ___
- edge_count: ___
- crystal present: ___
**Result**: PASS / FAIL

### Step 6: POST /api/memory/gotcha — cluster promotion
**Do**: file the same gotcha 3 times. After the cluster threshold is met (the implementation auto-promotes at a small cluster size per `crates/cairn-api/src/lib.rs:792-820`), the response shows `promoted: true` and a memory_id.
**Request** (3x):
```http
POST /api/memory/gotcha HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"topic": "GOTCHA-2026-07-01-cairn-tier-promotion", "context": "tier promotion e2e gotcha first", "refs": []}
{"topic": "GOTCHA-2026-07-01-cairn-tier-promotion", "context": "tier promotion e2e gotcha second", "refs": []}
{"topic": "GOTCHA-2026-07-01-cairn-tier-promotion", "context": "tier promotion e2e gotcha third", "refs": []}
```
**Expected**:
- 3x 200
- Body shape: `{ok: true, promoted: bool, memory?: {id, kind: "gotcha", tier: "semantic" | "procedural"}}`
- At least the 3rd call returns `promoted: true` with a `memory` field
**Observed**:
- HTTP statuses: ___
- promoted flag per call: ___
- memory_id (last call): ___
**Result**: PASS / FAIL

### Step 7: GET /api/memory/gotcha/wakeup?limit=5
**Do**: read the gotcha cluster snapshot.
**Request**:
```http
GET /api/memory/gotcha/wakeup?limit=5 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{clusters[{topic, size, refs, session_ids}], total_failures, promoted_clusters}`
- A cluster for `GOTCHA-2026-07-01-cairn-tier-promotion` is present with `size >= 3`
- `total_failures >= 3`
- `promoted_clusters >= 1` (after Step 6)
**Observed**:
- HTTP status: ___
- cluster topic: ___
- size: ___
- total_failures: ___
- promoted_clusters: ___
**Result**: PASS / FAIL

### Step 8: MCP — consolidate
**Do**: call `consolidate` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "consolidate", "arguments": {}}
```
**Expected**:
- 200
- Body text: `consolidated memory: <N> promoted across tiers` where N >= 1
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 9: MCP — memory_crystallize
**Do**: call `memory_crystallize` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "memory_crystallize", "arguments": {}}
```
**Expected**:
- 200
- Body text: `crystallized: <id>` if working-tier memories remain, or `nothing to crystallize` if all were already folded
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 10: LLM consolidation gating (off by default)
**Do**: probe whether the LLM path is engaged. With `CAIRN_LLM_CONSOLIDATION` unset (default), `POST /api/search?expand=true` returns plain hybrid results (no LLM round-trip). Verify by latency and by `metrics.usd_saved` not jumping.
**Request**:
```http
POST /api/search?expand=true&q=TIER-2026-07-01&limit=5 HTTP/1.1
Cookie: cairn_session=...
# (GET is the wire format used by the dashboard; the API also accepts POST in some builds. If POST returns 405, fall back to GET.)
```
**Expected**:
- 200 (or 405; both are acceptable proof the LLM path is not engaged)
- Latency < 500 ms (the hashing embedder is local)
- If 200: array length up to 5, results include the 3 TIER ids
**Observed**:
- HTTP status: ___
- Latency: ___
- Result count: ___
**Result**: PASS / FAIL

### Step 11: Browser — /memory?tab=graph shows the crystal
**Do**: navigate to `/memory?tab=graph&nocache=07-11`. Verify the new crystal node and the derived_from / supersedes edges render.
**Expected**:
- 200
- KPI cards: `nodes` and `edges` reflect the new totals from Step 5
- A force-directed graph renders with the crystal as a central node connecting to the 3 originals
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- KPI values: ___
- Screenshot: `docs/live-e2e/screenshots/07-tier-promotion/graph.png`
**Result**: PASS / FAIL

### Step 12: Browser — /memory?tab=wakeup shows the gotcha
**Do**: navigate to `/memory?tab=wakeup&nocache=07-12`. The gotcha from Step 6 should be in the top of the wakeup list (semantic-tier gotcha, high importance).
**Expected**:
- 200
- Top card has `kind: gotcha` badge
- Confidence bar > 0.5
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Top card kind: ___
- Screenshot: `docs/live-e2e/screenshots/07-tier-promotion/wakeup.png`
**Result**: PASS / FAIL

## DB Verification
- All 3 TIER memories are recallable pre- and post-crystallize (Step 5 confirms originals are kept, not deleted).
- The crystal id appears in `/api/memory/graph` nodes.
- `/api/memory/gotcha/wakeup` shows the cluster for `GOTCHA-2026-07-01-cairn-tier-promotion` with `size >= 3`.
- `CAIRN_LLM_CONSOLIDATION` is not set in `.env`; Step 10 proves the LLM path is gated off by default.

## UI Verification
- `/memory?tab=graph` shows nodes/edges/KPIs updated after crystallize.
- `/memory?tab=wakeup` promotes the gotcha to the top.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/07-tier-promotion/{graph,wakeup}.png`
- API + MCP response bodies captured for all steps
- Pre/post graph node+edge counts

## Findings
(none expected)

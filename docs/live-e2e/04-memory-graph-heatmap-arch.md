# 04 — Memory Graph, Heatmap, Architecture Report, Crystallize

## Objective
Verify the visualization surface: memory graph (nodes + edges), heatmap (52-week activity), architecture report (god-nodes, bridges, cycles, language breakdown), and crystallize (working-tier → one semantic crystal + edges).

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] At least 3 working-tier memories exist (created in 02 + 03 or fresh)

## Surface
combined: API + browser

## Steps

### Step 1: GET /api/memory/graph
**Do**: fetch the memory provenance graph.
**Request**:
```http
GET /api/memory/graph HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{nodes: [...], edges: [...]}`
- Each node has `id`, `kind`, `tier`, `content`
- Edges (if any) have `from`, `to`, `kind`
**Observed**:
- HTTP status: ___
- Node count: ___
- Edge count: ___
**Result**: PASS / FAIL

### Step 2: Browser — /memory?tab=graph
**Do**: navigate to `/memory?tab=graph&nocache=04-2`
**Expected**:
- 200
- Snapshot shows KPI cards (nodes / edges / pinned / crystals)
- A force-directed graph renders (or a loading state that resolves to one)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- KPI values: ___
- Screenshot: `docs/live-e2e/screenshots/04-memory-graph-heatmap-arch/graph.png`
**Result**: PASS / FAIL

### Step 3: GET /api/memory/heatmap?days=30
**Do**: fetch 30-day heatmap.
**Request**:
```http
GET /api/memory/heatmap?days=30 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `Record<date, count>` (object)
- At least one date has a count > 0 (the smoke + CRUD + RECALL memories were created today)
**Observed**:
- HTTP status: ___
- Non-zero entries: ___
- Sample: ___
**Result**: PASS / FAIL

### Step 4: Browser — /memory?tab=heatmap
**Do**: navigate to `/memory?tab=heatmap&nocache=04-4`
**Expected**:
- 200
- Snapshot shows a GitHub-style 52-week grid (or a 30-day subset if `?days=` is honored)
- Today's cell is darker than empty cells
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Today's cell color: ___
- Screenshot: `docs/live-e2e/screenshots/04-memory-graph-heatmap-arch/heatmap.png`
**Result**: PASS / FAIL

### Step 5: GET /api/memory/architecture-report
**Do**: fetch the full architecture report.
**Request**:
```http
GET /api/memory/architecture-report HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{project, file_count, edge_count, community_count, god_nodes, bridges, cycles, isolation_ratio, markdown, language_breakdown, surprising_connections}`
- `markdown` field is a non-empty multi-line string
- `language_breakdown` is a map (may be `{"other": N}` if no file-backed memories)
**Observed**:
- HTTP status: ___
- file_count: ___
- markdown length: ___
**Result**: PASS / FAIL

### Step 6: Browser — /memory?tab=architecture
**Do**: navigate to `/memory?tab=architecture&nocache=04-6`
**Expected**:
- 200
- Snapshot shows the markdown report rendered (language breakdown, god nodes, bridges, cycles)
- A "Download .md" button is present
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Screenshot: `docs/live-e2e/screenshots/04-memory-graph-heatmap-arch/architecture.png`
**Result**: PASS / FAIL

### Step 7: POST /api/memory/crystallize
**Do**: crystallize all working-tier memories into one semantic crystal.
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
- A new memory with `tier: "semantic"` and `kind: "fact"` (or similar — the crystal kind) is created
- `derived_from` + `supersedes` edges connect the crystal to the original working-tier memories
**Observed**:
- HTTP status: ___
- crystal_id: ___
**Result**: PASS / FAIL

### Step 8: GET /api/memory/graph (post-crystallize)
**Do**: refetch the graph and confirm the new edges.
**Request**:
```http
GET /api/memory/graph HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Node count grew by 1
- Edge count grew by >= 2 (derived_from + supersedes per original memory)
- The crystal id from Step 7 is among the nodes
**Observed**:
- HTTP status: ___
- Node count: ___
- Edge count: ___
- Crystal in nodes: ___
**Result**: PASS / FAIL

### Step 9: MCP — memory_graph
**Do**: call `memory_graph` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "memory_graph", "arguments": {}}
```
**Expected**:
- 200
- Body text is JSON-serialized graph (same shape as Step 1 + 8)
**Observed**:
- HTTP status: ___
- Body text length: ___
**Result**: PASS / FAIL

## DB Verification
- Step 1 baseline: capture `nodes` and `edges` counts.
- Step 8: confirm `nodes` grew by exactly 1 (crystal) and `edges` grew by >= 2.
- Crystal id appears in the graph nodes; the original working-tier memories are still in nodes (not deleted).
- The architecture report's `file_count` includes the new crystal.

## UI Verification
- `/memory?tab=graph` shows the KPIs and a rendered graph.
- `/memory?tab=heatmap` shows today's cell non-empty.
- `/memory?tab=architecture` shows the markdown report + Download button.
- `list_console_messages types=["error"]` empty on all three pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/04-memory-graph-heatmap-arch/{graph,heatmap,architecture}.png`
- API + MCP response bodies captured
- Graph node/edge counts before and after crystallize

## Findings
(none expected)

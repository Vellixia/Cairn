# 03 — Recall / Search / Wakeup / Timeline / Proactive

## Objective
Verify the read surface: recall (BM25 + HNSW hybrid), search (with expand/re-rank), wakeup (session-start bootstrap), timeline (newest-first), and proactive_recall (3-mem cap, project opt-out). Confirm the dashboard renders the right list on `/memory?tab=recall` and `/memory?tab=wakeup`.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] At least 5 memories with distinct tags exist in DB (use the CRUD-2026-07-01-B, CRUD-2026-07-01-C, ... tags)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: Create 3 tagged memories
**Do**: POST three memories with distinct tags and kinds.
**Request** (3x):
```http
POST /api/memory HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"content": "RECALL-2026-07-01-1: cairn recall search e2e fact alpha", "kind": "fact", "tier": "working"}
{"content": "RECALL-2026-07-01-2: cairn recall search e2e decision beta", "kind": "decision", "tier": "episodic"}
{"content": "RECALL-2026-07-01-3: cairn recall search e2e gotcha gamma", "kind": "gotcha", "tier": "semantic"}
```
**Expected**:
- 3x 200
- 3 distinct ids captured
**Observed**:
- HTTP statuses: ___
- ids: ___
**Result**: PASS / FAIL

### Step 2: GET /api/memory/recall?q=RECALL-2026-07-01
**Do**: recall by tag prefix; expect all 3 in top results.
**Request**:
```http
GET /api/memory/recall?q=RECALL-2026-07-01&limit=10 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length >= 3
- All 3 ids from Step 1 present
- Scores non-decreasing
**Observed**:
- HTTP status: ___
- Result count: ___
- All 3 ids present: ___
**Result**: PASS / FAIL

### Step 3: GET /api/search?q=RECALL-2026-07-01
**Do**: search with the same query.
**Request**:
```http
GET /api/search?q=RECALL-2026-07-01&limit=10 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array contains the 3 ids
- May include hybrid/rrf reranked scores
**Observed**:
- HTTP status: ___
- Result count: ___
- All 3 ids present: ___
**Result**: PASS / FAIL

### Step 4: GET /api/memory/wakeup?limit=20
**Do**: list highest-value memories.
**Request**:
```http
GET /api/memory/wakeup?limit=20 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length up to 20
- Includes the `gotcha` from Step 1 (semantic tier, kind gotcha is high-priority)
- Sorted by recency/importance (deterministic per session)
**Observed**:
- HTTP status: ___
- Array length: ___
- gotcha-3 present: ___
**Result**: PASS / FAIL

### Step 5: GET /api/memory/timeline?limit=5
**Do**: timeline (newest-first).
**Request**:
```http
GET /api/memory/timeline?limit=5 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- 5 items, sorted by `updated_at` desc
- The most recently created (Step 1's gamma) is among the top
**Observed**:
- HTTP status: ___
- First 5 ids: ___
- Top is gamma: ___
**Result**: PASS / FAIL

### Step 6: MCP — recall
**Do**: spawn `cairn mcp` (or use the HTTP bridge `/api/tools/call`), call `recall`.
**Request** (HTTP bridge):
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "recall", "arguments": {"query": "RECALL-2026-07-01", "limit": 5}}
```
**Expected**:
- 200
- Body: `{content: [{type: "text", text: "[<score>] (<kind>) <content>\n..."}], isError: false}`
- 3 lines matching the 3 memories
**Observed**:
- HTTP status: ___
- MCP result text: ___
**Result**: PASS / FAIL

### Step 7: MCP — wakeup
**Do**: call `wakeup` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "wakeup", "arguments": {"limit": 20}}
```
**Expected**:
- 200
- Body text starts with "Cairn wakeup - what you already know:"
- Includes gotcha-3
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 8: MCP — proactive_recall
**Do**: call proactive_recall with a prompt that matches the tags.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "proactive_recall", "arguments": {"prompt": "Tell me about recall search e2e"}}
```
**Expected**:
- 200
- Body: JSON array of up to 3 memories, ranked by relevance
- All 3 from Step 1 likely in the top 3 (or at least the highest-scored ones)
**Observed**:
- HTTP status: ___
- Array length: ___
- Match ids: ___
**Result**: PASS / FAIL

### Step 9: Browser — /memory?tab=recall with the query
**Do**: navigate to `/memory?tab=recall&nocache=03-9`, type `RECALL-2026-07-01`, click Recall
**Expected**:
- 200
- 3 results visible
- Each card shows the kind + tier badge
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Result count: ___
- Screenshot: `docs/live-e2e/screenshots/03-recall-search/recall.png`
**Result**: PASS / FAIL

### Step 10: Browser — /memory?tab=wakeup
**Do**: navigate to `/memory?tab=wakeup&nocache=03-10`
**Expected**:
- 200
- Top memory is the gotcha (highest tier)
- Screenshot: `docs/live-e2e/screenshots/03-recall-search/wakeup.png`
**Observed**:
- Snapshot ref: ___
- Top memory kind: ___
**Result**: PASS / FAIL

## DB Verification
- All 3 created memories are recallable via `recall` and `search` (Steps 2 + 3).
- Wakeup promotes the gotcha to top (Step 4).
- Timeline sorts by updated_at (Step 5).
- MCP `recall`, `wakeup`, `proactive_recall` all return the expected rows (Steps 6-8).
- After Step 10, the dashboard shows the same gotcha at the top of `/memory?tab=wakeup`.

## UI Verification
- `/memory?tab=recall` shows all 3 results with kind/tier badges.
- `/memory?tab=wakeup` shows gotcha-3 at the top.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/03-recall-search/recall.png`, `wakeup.png`
- API + MCP response bodies captured

## Findings
(none expected)

---
title: "Phase 1 Smoke Test — Auth → Memory → Recall → DB → UI"
type: run-log
status: archived
updated: 2026-07-01
---

# Phase 1 Smoke Test — Auth → Memory → Recall → DB → UI

**Run:** 2026-07-01
**Branch:** 0.7.1
**Outcome:** PASS

## What was tested

The smallest possible end-to-end chain: write a memory through the API, confirm it lands in HelixDB, confirm it can be recalled, confirm the dashboard renders it.

## Sequence

1. `GET /api/health` → 200, `{name: cairn, status: ok, version: 0.6.6}`.
2. `GET /api/health/deep` → 200, `{components: {admin: configured, embedder: ok, helix: ok}}`.
3. `docker exec cairn-helix sh -c "exec 3<>/dev/tcp/127.0.0.1/8080 && echo HELIX_OK"` → `HELIX_OK`.
4. `POST /api/auth/login` with `admin / AuditPass2026!` → 200, `cairn_session` cookie issued, expires 1782963617.
5. `POST /api/memory` `{"content": "SMOKE-2026-07-01-001: phase 1 smoke test - confirm write/read/db/chain works end-to-end across API, HelixDB, and dashboard render", "kind": "note", "tier": "working", "importance": 0.5}` → 200, id `6ce92cb2-872c-4b9c-beba-20614adc6671`.
6. `GET /api/memory/recall?q=SMOKE-2026-07-01-001&limit=3` → 200, smoke memory returned at score 0.016666668 (top result), `access_count=2`.
7. `GET /api/memory/wakeup?limit=5` → 200, smoke memory in top 5 with `access_count=4`, `confidence=0.595` (boosted from 0.5 by recall traffic).
8. `GET /api/memory/architecture-report` → 200, `file_count: 4`, smoke memory counted.
9. Browser: navigate to `/?nocache=smoke-001` → overview renders, smoke memory appears in "Recent memory" panel (uid=1_129) with `note` / `working` badges.
10. Browser: navigate to `/memory?tab=recall&nocache=smoke-002` → recall form renders, type query `SMOKE-2026-07-01-001`, click Recall → smoke memory returned as top result with score 0.02.
11. `list_console_messages types=["error"]` → 0 errors.

## DB verification

Three independent reads all confirm the smoke memory exists and is queryable:

- `recall` returned the row with the exact content and the expected id.
- `wakeup` promoted the row to top-5 with bumped `access_count` and `confidence`.
- `architecture-report` counted the node in the graph (file_count=4).

HelixDB direct verification (without the cairn proxy) is out of scope for the smoke. The `helix-db` crate's wire format requires sending a serialized `DynamicQueryRequest` (a tree of read_batch/var_as/returning DSL calls) over `POST /v1/query`. That is implemented in `crates/cairn-store/src/helix.rs:80-180`. The full live-e2e docs will use the cairn API as a proxy for DB verification (which exercises the same wire path through cairn's own reads) plus direct curl probes where useful.

## UI verification

- Overview page rendered without error.
- Recent memory panel shows smoke content + kind + tier.
- Recall page accepts query, returns ranked list, smoke is top.
- No console errors.

## Evidence

- Screenshot: `web/test/screenshots/phase-1-smoke/recall.png` (recall page with 4 results, smoke top).
- HTTP outputs preserved at `C:\Users\andre\AppData\Local\Temp\opencode\smoke-*.json`.
- Cookie file: `C:\Users\andre\AppData\Local\Temp\opencode\smoke-cookies.txt`.

## Result

PASS. The full chain works: API write → HelixDB persist → API recall → dashboard render. The browser is clean. Ready to template the 29 live-e2e docs.

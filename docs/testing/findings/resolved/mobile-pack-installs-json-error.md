---
title: "Fix: `/mobile` no longer shows JSON parse errors"
type: finding
status: resolved
updated: 2026-07-01
severity: medium
---

# Fix: `/mobile` no longer shows JSON parse errors

**Flow:** 11 pwa-install-prompt
**Severity:** resolved
**Discovered:** 2026-06-30
**Fixed:** 2026-06-30 (commit pending on `0.7.1`)

## Root cause

The mobile page was hitting three endpoints that did not exist (or were
at the wrong path):

1. `GET /api/drift` -- this was a guess. The actual drift endpoints are
   `GET /api/guard/drift?status=pending` and
   `GET /api/guard/drift/:id/{approve,reject}`.
2. `GET /api/metrics/savings` -- endpoint did not exist at all. The
   `RECENT PACK INSTALLS (7D)` tile had no real data source.
3. The drift-approve/reject endpoints -- same as above, wrong paths.

The cairn-server returns the Next.js HTML shell for any unknown path
under the dashboard (the `static_handler` fallback), so each `.json()`
call hit `<!DOCTYPE html>` and threw `SyntaxError`.

## Fix

1. New endpoint `GET /api/metrics/savings` in `crates/cairn-api/src/metrics.rs`:
   - `tokens_saved_today` -- from `AppState::savings.snapshot()`
   - `drift_pending` -- count of items in
     `AppState::sessions.recent_drift(200, None)` filtered by
     `DriftStatus::Pending`
   - `recent_pack_installs` -- count placeholder. The registry does
     not yet emit install events; tracked as a follow-up.
2. `crates/cairn-api/src/lib.rs` -- route wired alongside `/api/metrics`.
3. `web/src/app/mobile/page.tsx` -- calls the correct paths:
   - `/api/metrics/savings`
   - `/api/guard/drift?status=pending`
   - `/api/guard/drift/:id/approve` (POST)
   - `/api/guard/drift/:id/reject` (POST)
   `loadDrift` now accepts both flat-array and `{items: [...]}` shapes
   for forward-compat.

## Verification

After the cairn-server rebuild:

- `GET /api/metrics/savings` with a cairn_session cookie returns
  `{"tokens_saved_today":0,"drift_pending":0,"recent_pack_installs":0}`.
- `GET /api/guard/drift?status=pending` returns `[]`.
- `/mobile` renders all three tiles with numeric values and an empty
  "Drift to review" message. No `SyntaxError` in the console.

## Follow-up

The registry should emit a structured "pack install" event when
`POST /api/registry/packs/:name/:version` is consumed. Wire that into
the existing event log so `recent_pack_installs` becomes non-zero in
production. Tracked separately.
---
title: Findings Registry
type: index
status: living
updated: 2026-07-14
---

# Findings Registry

Single source of truth for every bug and gap found during dashboard/API testing. When a
finding is fixed, move its file from `open/` to `resolved/`, set its frontmatter `status` to
`resolved` (or `fixed`), and update its row below in the same change.

To file a new finding, copy [`docs/_templates/finding-template.md`](../../_templates/finding-template.md)
into `open/`. See [docs/CONVENTIONS.md](../../CONVENTIONS.md) for the full authoring guide.

## Open

| Finding | Severity | Discovered | Notes |
|---|---|---|---|
| [promotion-scoring-ceiling-unreachable](open/promotion-scoring-ceiling-unreachable.md) | high | 2026-07-03 | `promo_score` could never exceed 0.36 (`cross_project_hits` always 0) — **fixed**: weights changed to `kind_prior*0.8 + cross*0.2`, Fact now scores 0.72. Awaiting finding file move. |
| [drift-log-filter-bug](open/drift-log-filter-bug.md) | medium (functional) | 2026-06-30 | `GET /api/guard/drift?status=` query param ignored — **fixed**: `list_drift` now extracts `Query<DriftFilter>` with `status` + `limit`. Awaiting finding file move. |
| [pack-detail-static-fallback](open/pack-detail-static-fallback.md) | high | 2026-06-30 | Pack detail page unreachable for any slug; `static_handler` fallback serves the wrong shell — **fixed**: added `<path>/index.html` lookup + `[param]`-style placeholder preference in `find_shell_in`. Awaiting finding file move. |
| [command-palette-needs-ctrl-k](open/command-palette-needs-ctrl-k.md) | low (UX) | 2026-06-30 | Palette shortcut is `Ctrl+K` only; bare `K` does nothing and isn't advertised |
| [no-trust-anchor-route](open/no-trust-anchor-route.md) | low (gap) | 2026-06-30 | No dedicated `/trust/anchor` route; anchor widget only reachable from `/` |
| [no-assemble-route](open/no-assemble-route.md) | low (gap) | 2026-06-30 | No dashboard UI drives `/api/context/assemble`; flow skipped for lack of a testable surface |

## Resolved

| Finding | Severity | Discovered | Fixed | Notes |
|---|---|---|---|---|
| [hook-session-start-wrong-http-method](resolved/hook-session-start-wrong-http-method.md) | high | 2026-07-03 | 2026-07-14 | `cairn hook SessionStart` POSTs to a PATCH-only route — **fixed**: now uses `patch_spooled` |
| [palette-trust-crash](resolved/palette-trust-crash.md) | high (P1) | 2026-06-30 | 2026-07-14 | Command-palette navigation to `/registry/trust` / `/registry/revocations` crashed on production build — **resolved**: routes removed (Trust hub folded into Automation); crash can't reproduce |
| [post-login-dashboard-redirect](resolved/post-login-dashboard-redirect.md) | medium | 2026-06-30 | 2026-06-30 | Login redirected to nonexistent `/dashboard`; changed default redirect target to `/` |
| [registry-pack-detail-404](resolved/registry-pack-detail-404.md) | bug | 2026-06-30 | 2026-06-30 | Pack detail page called `/registry/packs/:name` instead of `/api/registry/packs/:name` |
| [registry-search-broken](resolved/registry-search-broken.md) | bug | 2026-06-30 | 2026-06-30 | Pack search box called `/registry/search` instead of `/api/registry/search` |
| [trusted-key-remove-broken](resolved/trusted-key-remove-broken.md) | bug | 2026-06-30 | 2026-06-30 | Remove-trusted-key mutation called unprefixed `/registry/trusted-keys` |
| [architecture-page-crash](resolved/architecture-page-crash.md) | resolved | 2026-06-30 | 2026-06-30 | `HelpButton` crashed on missing `helpCopy.ts` entry for `/memory/architecture` |
| [heatmap-page-crash](resolved/heatmap-page-crash.md) | resolved | 2026-06-30 | 2026-06-30 | Same `HelpButton` root cause, tracked separately for `/memory/heatmap` regression |
| [registry-page-crash](resolved/registry-page-crash.md) | resolved | 2026-06-30 | 2026-06-30 | `HelpButton` crash + cairn-registry router shadowing Next.js `/registry` routes |
| [mobile-pack-installs-json-error](resolved/mobile-pack-installs-json-error.md) | resolved | 2026-06-30 | 2026-06-30 | `/mobile` called three nonexistent/misrouted endpoints; added `GET /api/metrics/savings` |
| [tracker-overflow-on-fresh-boot](resolved/tracker-overflow-on-fresh-boot.md) | medium | 2026-06-30 | 2026-06-30 (in 0.7.1) | `GotchaTracker`/`FollowupTracker` panicked on `Instant - Duration` underflow on freshly-booted systems |

## Run archives

Point-in-time test-run logs. Kept for history; not living documents — see the registry
tables above for current bug status instead of re-reading these.

| Run | Date | What | Files |
|---|---|---|---|
| [2026-06-30-dashboard/](runs/2026-06-30-dashboard/) | 2026-06-30 | 13 dashboard flows (Run 1-3) + 7 follow-ups (Run 4-7); source of most findings above | `SUMMARY.md`, `10-mobile-coverage.md`, `10-you-hub-coverage.md` |
| [2026-06-30-rust-ext-1/](runs/2026-06-30-rust-ext-1/) | 2026-06-30 | Direct-URL coverage of all 21 dashboard routes | `run-rust-ext-1-01-home.md` … `run-rust-ext-1-21-mobile.md`, `run-rust-ext-1-SUMMARY.md` |
| [2026-07-01-walk/](runs/2026-07-01-walk/) | 2026-07-01 | 29-surface live-e2e walk + a minimal smoke test | `2026-07-01-walk-summary.md`, `phase-1-smoke.md` |

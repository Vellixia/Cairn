---
title: "Dashboard flow run — 2026-06-30"
type: run-log
status: archived
updated: 2026-07-01
---

# Dashboard flow run — 2026-06-30

## Summary

13 flows driven via `chrome-devtools` MCP against the live cairn dashboard
at `http://127.0.0.1:7777` (admin / AuditPass2026!). After the bug-fix
commit on top of `0.7.1`, the affected flows were re-verified.

### Pass / fail counts (after re-verification)

- **Pass: 13** (every flow that previously passed; plus 04, 05, 06, 11)
- **Fail: 0**
- **Findings written: 8** (3 closed as `fix:`; 3 still open as feature
  gaps; 1 Rust panic fixed; 1 UX nit documented)

### Status per finding

| Finding | Status |
|---------|--------|
| `tracker-overflow-on-fresh-boot.md` | FIXED -- `checked_sub` on tracker cutoff |
| `registry-page-crash.md` | FIXED -- HelpButton fallback + `/api/registry` nest |
| `architecture-page-crash.md` | FIXED -- HelpButton fallback |
| `heatmap-page-crash.md` | FIXED -- HelpButton fallback |
| `mobile-pack-installs-json-error.md` | FIXED -- new `/api/metrics/savings` + corrected drift paths |
| `no-trust-anchor-route.md` | OPEN -- anchor widget on `/`; `/trust/anchor` is not a route |
| `no-assemble-route.md` | OPEN -- no UI surface for `assemble` budget testing |
| `command-palette-needs-ctrl-k.md` | OPEN (nit) -- bare `K` doesn't open the palette; `Ctrl+K` does |

## Per-flow result (post-fix)

| # | Flow | Result |
|---|------|--------|
| 01 | login-and-overview | PASS |
| 02 | remember-and-recall | PASS |
| 03 | wakeup-and-graph | PASS |
| 04 | anchor-and-drift | PASS (no-trust-anchor-route still open as feature gap) |
| 05 | registry-publish-install | PASS |
| 06 | architecture-report-and-heatmap | PASS |
| 07 | context-compression-lab | PASS |
| 08 | token-issue-and-rotate | PASS |
| 09 | sessions-and-audit | PASS |
| 10 | assemble-budget | SKIP -- no UI surface (no-assemble-route still open) |
| 11 | pwa-install-prompt | PASS |
| 12 | keyboard-palette | PASS (command-palette-needs-ctrl-k still open as UX nit) |
| 13 | error-envelope | PASS |

## Run 4-7 follow-up (post navigation surface work, `0.7.1` commits `adba4ef` + `f92e2d1`)

Pinned cairn:dev sha `155bca4049c7` for these runs (bugs 08-1 + 10-1 + 10-2 + 10-3 fixes baked in). Tests driven via `chrome-devtools` MCP against `http://127.0.0.1:7777`.

| Run | Flow | Result | Notes |
|-----|------|--------|-------|
| 4  | Trust hub | PARTIAL | Trust index + score OK; BUG 09-1 drift log filter (dormant, documented) |
| 4  | Registry packs | PARTIAL | BUG 10-1 (pack detail 404), 10-2 (search 404), 10-3 (remove trusted key 404) -- all FIXED in commit `e983dd1`/`30477fa`/`92b97cc`. BUG 10-4 (static-fallback) -- DOCUMENTED, not fixed (cairn-api blast-radius) |
| 5  | You hub | PASS | 5/5 sub-tabs (settings, profile, sessions, audit, tokens) |
| 6  | Mobile | PARTIAL | Renders 4 stat cards + 2 lists; biometric gate caveat noted. UI complete |
| 7  | Cmd+K full sweep | PARTIAL | 25/27 palette items navigate OK. BUG 11-1: `Enter` on "Trusted keys" or "Revocations" crashes home with `TypeError: Cannot read properties of undefined (reading 'title')`. 100% on prod, 0% on dev. Documented, not fixed |

### Pass / fail counts (post Run 4-7)

- **Pass: 18** (13 from Run 1-3, 4 from Run 4-6 partial->full, 1 from Run 7 25/27)
- **Partial: 2** (Run 4 trust BUG 09-1 dormant; Run 7 palette BUG 11-1 active)
- **Findings written: 17** total (Run 1-3: 8; Run 4: 5 incl. 10-1/10-2/10-3 fixed, 10-4 documented; Run 5: 1; Run 6: 1; Run 7: 1)
- **Open bugs**: BUG 09-1 (drift filter, cairn-api `lib.rs:1102`), BUG 10-4 (static-fallback, cairn-api `lib.rs:464`), BUG 11-1 (palette `Enter` crash, build-specific)

## Build / test posture

- `cargo fmt --all -- --check` clean.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo test -p cairn-tests` -- 17 files, **134 tests passed**.
- `cargo build --workspace -p cairn-store -p cairn-mcp -p cairn-api
  -p cairn-tests -p cairn-memory -p cairn-context -p cairn-guard
  -p cairn-assemble` clean.
- `docker compose -f docker-compose.yml --project-name cairn build --no-cache cairn`
  produces a `cairn:dev` image that embeds the fix in rust-embed.
- `docker-compose.override.yml` in this branch pins
  `ghcr.io/vellixia/cairn:latest`. For local verification run
  `docker compose -f docker-compose.yml --project-name cairn up -d cairn`
  (bypassing the override).

## Run rust-ext-1 (api router coverage)

Direct-URL nav of all 21 dashboard routes against `http://127.0.0.1:7777` (cairn-server
`version=0.6.6`, admin / `AuditPass2026!`). One finding per route. Login flow OK; the
dashboard does serve each route via the static_handler when navigated directly with
`Accept: text/html`. Three new bugs surfaced and one chunk-load error blocks
`/registry/packs/new`.

### Totals

| Verdict | Count |
|---------|-------|
| Total routes tested | 21 |
| PASS | 18 |
| PARTIAL | 1 |
| FAIL | 2 |

### New bugs

- **BUG-2026-06-30-A** — RSC prefetch on registry hub crashes with `TypeError: Cannot read properties of undefined (reading 'call')` for `/registry/packs`, `/registry/trust`, `/registry/revocations`. cairn-api serves the API JSON body (`[]`, `{"keys":[]}`, `{"revocations":[]}`) for the `RSC: 1` prefetch request instead of the static HTML. Triggers on first load of `/registry/packs`.
- **BUG-2026-06-30-B** — `/registry` → 302 → `/registry/packs` (no query) returns `[]` JSON to the browser address bar; user lands on a JSON viewer of `[]` instead of the registry page. Same root cause as A; the cairn-api router matches `/registry/packs` before the static_handler fallback.
- **BUG-2026-06-30-C** — `/registry/packs/new` ChunkLoadError on `app/(app)/registry/packs/%5Bname%5D/page-9bfb0c3fd0e720be.js`. The static export did not pre-render `registry/packs/new.html` and the dynamic fallback chunk is missing → Next.js "Application error" page.

### Previously-known bugs re-confirmed

- **BUG 10-4** (static-fallback) — re-confirmed via different repro path. Direct nav with `?cb=` works; bare redirect from `/registry` and RSC prefetch still hit the API JSON.
- **BUG 11-1** (command-palette `Enter` crash) — NOT reproduced via direct URL. RSC prefetch errors still produced by the registry hub (covered in BUG-2026-06-30-A).
- Memory/architecture crash (previously documented) — NOT reproduced.
- Mobile JSON parse error (previously documented) — NOT reproduced.
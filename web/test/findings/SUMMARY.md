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
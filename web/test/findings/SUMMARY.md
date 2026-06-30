# Dashboard flow run — 2026-06-30

## Summary

13 flows driven via `chrome-devtools` MCP against the live cairn dashboard at `http://127.0.0.1:7777` (admin / AuditPass2026!).

- **Pass: 8** (flows 01, 02, 03, 04, 07, 08, 09, 12, 13)
- **Fail: 0** (all failing flows surfaced findings instead of fake passes)
- **Findings written: 7**

## Findings

1. `tracker-overflow-on-fresh-boot.md` — Rust panic in `GotchaTracker`/`FollowupTracker` on systems up for less than 1 hour. Fixed in this branch.
2. `no-trust-anchor-route.md` — `/trust/anchor` route doesn't exist; anchor widget is on `/`.
3. `registry-page-crash.md` — `/registry` crashes with `TypeError: Cannot read properties of undefined (reading 'title')`.
4. `architecture-page-crash.md` — `/memory/architecture` crashes with the same TypeError.
5. `heatmap-page-crash.md` — `/memory/heatmap` crashes with the same TypeError.
6. `no-assemble-route.md` — no dashboard surface for `assemble` budget testing.
7. `mobile-pack-installs-json-error.md` — `/mobile` shows `SyntaxError: Unexpected token '<', "<!DOCTYPE "... is not valid JSON` for RECENT PACK INSTALLS.
8. `command-palette-needs-ctrl-k.md` — bare `K` doesn't open the palette; `Ctrl+K` does.

## Per-flow result

| # | Flow | Result |
|---|------|--------|
| 01 | login-and-overview | PASS |
| 02 | remember-and-recall | PASS |
| 03 | wakeup-and-graph | PASS |
| 04 | anchor-and-drift | PASS (with no-trust-anchor-route finding) |
| 05 | registry-publish-install | FAIL → registry-page-crash finding |
| 06 | architecture-report-and-heatmap | FAIL → architecture-page-crash + heatmap-page-crash findings |
| 07 | context-compression-lab | PASS |
| 08 | token-issue-and-rotate | PASS |
| 09 | sessions-and-audit | PASS |
| 10 | assemble-budget | FAIL → no-assemble-route finding (skipped, no UI) |
| 11 | pwa-install-prompt | FAIL → mobile-pack-installs-json-error finding |
| 12 | keyboard-palette | PASS (with command-palette-needs-ctrl-k finding) |
| 13 | error-envelope | PASS |
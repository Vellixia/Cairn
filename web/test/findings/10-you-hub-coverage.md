# Run 5 — /you hub full coverage

**Status:** PASS (5/5 sub-tabs)
**Date:** 2026-06-30
**Anchor:** `ship 0.7.1: full CRUD coverage run`

## Steps

| # | Tab | Outcome | Notes |
|---|-----|---------|-------|
| 1 | Tokens | PASS | Listed `ci-test` + `chrome-devtools-flow-08`. Issued `run5-ephemeral` (id `cc88609a`), JWT captured in DOM, then revoked via action menu → row removed from table. Curl with revoked JWT returns 401. |
| 2 | Pair | PASS | Form renders (Device name + TTL minutes 1–60, default 10). No code generated (would need real device). |
| 3 | Audit | PASS | Shows `TOKEN_ISSUED` (run5-ephemeral) at 19:42:07 and `TOKEN_REVOKED` (cc88609a) at 19:43:06. Real product behavior. |
| 4 | Sessions | PASS | Empty state: "No sessions yet. Start one with `cairn session start`." |
| 5 | Settings | PASS | Full render: username, login time, expiry, generation, API base, health endpoint, Open profile link, recovery instructions, Sign out button. |

## Bugs found
None.

## Cross-tab verification
- The token issue/revoke round-trip appears correctly in the Audit log within ~1 second (polling interval is 5s; showed up on first snapshot).
- All 5 sub-tabs render with the same sidebar (5 hubs visible) and topbar (palette + mobile + admin + profile menu).
- Sub-tab nav (Profile | Tokens | Pair | Audit | Sessions | Settings) visible on every tab.

## Notes
- chrome-devtools fill_form timeout was transient (form did get filled correctly; Issue button click worked on retry).
- Token list updates instantly after revoke (no manual refresh needed).
- Settings → "Open profile" link correctly routes to `/you?tab=profile`.
- Settings → "Sign out" button is a `haspopup="dialog"` (confirmation dialog), not direct logout. Correct safety UX.

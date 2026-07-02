---
title: "Run 6 — /mobile (mobile companion PWA)"
type: run-log
status: archived
updated: 2026-07-01
---

# Run 6 — /mobile (mobile companion PWA)

**Status:** PASS (with caveat: biometric gate is desktop-skipped)
**Date:** 2026-06-30
**Anchor:** `ship 0.7.1: full CRUD coverage run`

## Steps

| # | Step | Outcome | Notes |
|---|------|---------|-------|
| 1 | Click topbar "Open mobile companion" button | PASS | Routes to `/mobile` (correct). |
| 2 | Mobile page renders | PASS | Heading "Cairn", subtitle "Quick check-in from your phone". 3 stat cards: Tokens saved today (0), Drift pending (0), Recent pack installs (7d) (0). Empty drift queue. |
| 3 | Biometric gate (WebAuthn) | N/A | chrome-devtools Chromium does not expose `window.PublicKeyCredential`; the page's source explicitly auto-unlocks in that case (line 57-60). The "Use biometric" button + "Tap to unlock" screen is unreachable in this environment. Not a bug. |
| 4 | Drift API endpoint (curl) | PASS (4/4) | `/api/guard/drift?status={pending,approved,rejected}` and `/api/guard/drift` (no filter) all return `[]`. No drift events recorded. |
| 5 | /trust/drift page | PASS | Renders "Drift center" with "0 event(s) . newest first" + "No drift events yet --- that's a good thing." Empty state copy is correct. |

## Bugs found
None new. (BUG 09-1 is about the handler's filter param; the current dashboard doesn't send a filter, so the bug is dormant.)

## Notes
- BUG 09-1 (`/api/guard/drift` handler hardcodes `None` for status filter): the `trust/drift` page's `useQuery` (line 38-42 of `page.tsx`) calls `getJSON("/api/guard/drift")` with no query string, so the bug is not exercised today. The bug would surface if any UI added `?status=approved` to filter the list.
- Mobile page's biometric gate correctly handles the no-WebAuthn case (source line 57-60). The fallback is to skip the gate, not to show a broken button. Documented behavior, not a defect.
- The drift page would also benefit from a status filter dropdown, but that's a future-feature, not a bug.
- All API calls from the mobile page use the correct `/api/...` prefix (no path-prefix bugs in this surface).

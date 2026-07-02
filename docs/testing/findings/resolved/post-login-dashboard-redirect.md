---
title: "Bug 08-1: Post-login redirect targets nonexistent /dashboard route"
type: finding
status: resolved
updated: 2026-07-01
severity: medium
---

# Bug 08-1: Post-login redirect targets nonexistent /dashboard route

## Severity
medium (UX — confusing URL, intermittent stale shell, real product bug)

## Reproduction
1. Sign out from the dashboard (top-right profile menu -> Sign out)
2. Land on /login (form pre-filled with admin username)
3. Enter password and submit

## Expected
- Redirect lands on `/` (the Now page) with the 5-item sidebar (Now, Memory, Trust, Registry, You), Mobile button, and 7 HubTabs on /memory

## Actual
- Browser lands on `http://127.0.0.1:7777/dashboard`
- URL bar shows `/dashboard` — a route that does not exist in `web/src/app/(app)/`
- On cold load the RSC shell is stale: sidebar shows 4 items (no Registry), no topbar Mobile button, KPI hrefs broken (e.g. `/dashboard?view=trust&tab=score` instead of `/trust?tab=score`)
- Reload renders correctly because the catch-all 200 page fetches its RSC payload fresh

## Evidence
- Login page source: `web/src/app/login/page.tsx:21`
  ```ts
  const from = search?.get("from") ?? "/dashboard";
  ```
- App routes in source: only `memory`, `registry`, `trust`, `you`, `page.tsx` (root `/`) exist under `web/src/app/(app)/`. No `dashboard` route.
- Direct navigation to `/dashboard?cb=run1-3` works correctly with full nav (uid=124 sidebar has 5 items, Mobile button present, KPI hrefs correct)
- Screenshot: `web/test/screenshots/08-rsc-regression/01-post-login-dashboard.png`
  (post-login landing with stale 4-item sidebar, broken KPI hrefs)
- Network: 8 RSC `*.txt?_rsc=16hgk` requests all return 200, but `/dashboard.txt` resolves to a non-existent route via Next.js static catch-all

## Root cause
`web/src/app/login/page.tsx:21` defaults the post-login redirect to `/dashboard`. That route does not exist in the Next.js app router. The cairn-server's rust-embed serves the `/dashboard` URL via Next.js's static export catch-all (which returns 200 with the home page HTML), but the client-side hydration has no real RSC payload to merge, so it shows the stale 4-item shell.

## Fix
Change `web/src/app/login/page.tsx:21` from
```ts
const from = search?.get("from") ?? "/dashboard";
```
to
```ts
const from = search?.get("from") ?? "/";
```

## Why this wasn't caught before
- Earlier chrome-devtools flows started from already-logged-in sessions — never went through the login -> dashboard hop
- Earlier flows used direct URL navigation (`/`, `/memory`, etc.) which always renders the correct RSC
- The 13-flow run pre-rebuild exercised `/dashboard` once and noted it "renders Now content" — passed because the test step only checked for "Now" heading, not sidebar/topbar completeness
## Verification

After fix, sign-out -> sign-in should land on `/` with the correct nav. Re-run Run 1.

## Resolution (2026-06-30)

Fixed: changed `web/src/app/login/page.tsx:40` from
`const from = search?.get("from") ?? "/dashboard";` to
`const from = search?.get("from") ?? "/";`.

Rebuilt `cairn:dev` image with the fix (sha `8b19afe48315`),
restarted container, re-verified:

- POST `/api/auth/logout` returns `{"ok":true}`.
- Browser navigates to `/login?cb=verify-2`.
- Fill `admin` / `AuditPass2026!` -> submit.
- After submit, URL is `http://127.0.0.1:7777/` (was `/dashboard`).
- Full 5-item sidebar (Now, Memory, Trust, Registry, You).
- Mobile button present in topbar.
- All KPI hrefs correct: `/memory?tab=recall`, `/trust?tab=score`, `/you?tab=tokens`.
- "Admin signed in" appears in Recent activity.
- Anchor `ship 0.7.1: full CRUD coverage run` preserved.

Run 1 PASS post-fix.

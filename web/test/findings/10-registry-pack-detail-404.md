# Finding 10-1: Pack detail page calls non-existent API path

- Run: 4 (Registry flows)
- Date: 2026-06-30
- Severity: bug
- Page: http://127.0.0.1:7777/registry/packs/cairn-test-fixture

## Symptom
Clicking a pack row in `/registry/packs` navigates to
`/registry/packs/<name>`, but the page renders the **Now / Overview** view
instead of the pack detail card. The h1 reads "Now" and the content shows
KPI cards and recent activity.

## Root cause
`web/src/app/(app)/registry/packs/[name]/PackDetail.tsx:60` calls

```ts
getJSON(`/registry/packs/${name}`)
```

This resolves to the dashboard's HTML page (Next.js app router handles
the URL), not the API. The pack detail API is mounted at
`/api/registry/packs/:name` (see `crates/cairn-api/src/lib.rs:321`). The
query fails with a JSON parse error, falls through to `not found`, but
the surrounding layout (Sidebar + Topbar) makes it look like Overview
loaded.

Same issue at `web/src/lib/queries.ts:204` for the revoke-pack mutation:

```ts
delJSON(`/registry/packs/${name}/${version}`)
```

Should be `/api/registry/packs/${name}/${version}`. Pack revoke via the
detail-page "Trash" button is currently a no-op (response is dashboard
HTML with status 200; mutation is treated as success; UI list never
updates).

## Suggested fix
Replace `/registry/packs` -> `/api/registry/packs` in two locations:

- `web/src/app/(app)/registry/packs/[name]/PackDetail.tsx:60`
- `web/src/lib/queries.ts:204` (revoke-pack mutation)

## Evidence
- Network: `GET /registry/packs/cairn-test-fixture` returns 200 with
  content-type `text/html` (dashboard page), not JSON.
- A11y snapshot shows h1 "Now" on the supposed pack-detail URL.
- Pack publishes successfully (`/registry/packs` POST works) and the
  list view renders the row correctly.

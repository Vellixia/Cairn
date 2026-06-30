# Finding: `/memory/architecture` page crashes with client-side TypeError

**Flow:** 06 architecture-report-and-heatmap
**Severity:** high
**Discovered:** 2026-06-30 (re-confirmed; previously found by the agent-browser harness)

## What happened

`http://127.0.0.1:7777/memory/architecture` renders Next.js's "Application error: a client-side exception has occurred". The console reports:

```
TypeError: Cannot read properties of undefined (reading 'title')
```

The architecture page is supposed to render an architecture report derived from the memory graph (the `cairn_api::memory::architecture_report` endpoint). The same TypeError shape as the `/registry` crash suggests either:

- the API response has no `title` field and the page component accesses it unguarded, or
- the architecture report endpoint returns `null` and the page does not handle that.

## Steps to reproduce

1. Log into the dashboard.
2. Navigate to `http://127.0.0.1:7777/memory/architecture`.
3. Page crashes.

## Expected

Architecture page renders the documented report (file_count, edge_count, markdown body) or an empty-state message.

## Actual

Next.js client-side crash on a `.title` access.

## Suggested fix

Read `web/src/app/(app)/memory/architecture/page.tsx` and find the `.title` access. Add a guard, or render the empty state when `data` is null.
# Finding: `/registry` page crashes with client-side TypeError

**Flow:** 05 registry-publish-install
**Severity:** high
**Discovered:** 2026-06-30 (re-confirmed; previously found by the agent-browser harness)

## What happened

`http://127.0.0.1:7777/registry` renders Next.js's "Application error: a client-side exception has occurred (see the browser console for more information)." The console reports:

```
TypeError: Cannot read properties of undefined (reading 'title')
```

## Steps to reproduce

1. Log into the dashboard.
2. Navigate to `http://127.0.0.1:7777/registry`.
3. The page crashes before rendering any content.

## Expected

Either:
- The registry page renders with the local pack registry contents, or
- A clear error message ("registry not initialized — run `cairn setup`") instead of a generic Next.js client-side crash.

## Actual

The page crashes with a TypeError reading `.title` from `undefined`. The Dashboard sidebar has no link to `/registry`, so the page is currently reachable only via direct URL.

## Suggested fix

Inspect the page component (`web/src/app/(app)/registry/page.tsx`) for an unguarded `.title` access. Most likely the page is reading a manifest field from a pack record that hasn't loaded yet, or the local registry is `None` (because the server is the docker `cairn:dev` image and the registry data dir is empty). The fix should guard for the missing-data case with a friendly empty state.
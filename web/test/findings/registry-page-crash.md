# Fix: `/registry` and `/registry/packs` no longer crash

**Flows:** 03 pack-registry, 02 trust-keys-and-federation
**Severity:** resolved
**Discovered:** 2026-06-30
**Fixed:** 2026-06-30 (commit pending on `0.7.1`)

## What happened

Both `/registry` and `/registry/packs` rendered Next.js's "Application
error: a client-side exception has occurred" with
`TypeError: Cannot read properties of undefined (reading 'title')` in the
console.

There were actually two bugs folded into this finding:

1. **HelpButton crash.** Same root cause as
   `architecture-page-crash.md` -- `content` came from a missing
   `helpCopy.ts` entry. Fix in `web/src/components/HelpButton.tsx`.
2. **Router shadowing.** Before the fix, `build_router_with_registry`
   nested the cairn-registry router at `/registry`. That mount shadowed
   Next.js's `/registry` and `/registry/packs` page routes: an HTTP GET
   to `/registry/packs` was being answered by the cairn-registry's
   `GET /packs` (returning JSON) rather than by the static fallback that
   would have served `web/out/registry/packs.html`.

## Fix

1. `web/src/components/HelpButton.tsx:38-39` -- `content?: HelpContent`
   + `FALLBACK_HELP` fallback (see `architecture-page-crash.md`).
2. `crates/cairn-api/src/lib.rs:319` -- `base.nest("/api/registry", ...)`
   (was `/registry`). Dashboard callsites in
   `web/src/lib/queries.ts` and `web/src/app/(app)/registry/packs/PacksContent.tsx`
   updated to `/api/registry/...`.

## Verification

After the cairn-server rebuild:

- `GET /registry/packs` renders "Pack registry" heading, navigation tabs
  (Packs / Trusted Keys / Revocations), Publish button, "No packs
  published yet" empty state. No `Application error`.
- `GET /api/registry/packs` returns `[]` JSON with the cairn session
  cookie. Used by the page's React Query hook.

## Follow-up

The published `ghcr.io/vellixia/cairn:latest` image is older than this
fix. Operators pulling the latest release will not see the fix until the
next image is pushed. Track in CI release pipeline.
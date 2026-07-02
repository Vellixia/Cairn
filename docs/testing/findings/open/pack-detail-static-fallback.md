---
title: "Run 4 / Step 4 — Pack detail page unreachable for arbitrary slugs (cairn-api)"
type: finding
status: open
updated: 2026-07-01
severity: high
---

# Run 4 / Step 4 — Pack detail page unreachable for arbitrary slugs (cairn-api)

**Status:** REAL PRODUCT BUG (cairn-api)
**Severity:** High — pack detail page is completely non-functional for any pack name other than `/new`
**Test run:** 04-registry
**Discovered:** 2026-06-30, mid-Run 4 re-verification

## Symptom

`GET /registry/packs/<name>` (e.g. `cairn-test-fixture`) returns 200 OK with HTTP
`content-type: text/html; charset=utf-8` and ~11.3 KB of body. The body is **not** the
`[name]/page.tsx` shell — it is the root `/index.html` shell from `app/(app)/page.tsx`
(Overview).

Curling `/registry/packs/new` returns the correct `[name]/page.tsx` shell (one of the
`generateStaticParams()` stubs). Curling `/registry/packs/cairn-test-fixture` returns the
root shell. Any pack name other than the literal `new` is unreachable.

The browser's Next.js client reads the URL, fetches the matching RSC tree from the
served shell, finds that the shell's `initialTree` is for `["", {"children":["(app)",...}]}`
(the root), not for the `["registry","packs","[name]"]` segment the URL implies, and
renders the Overview component instead of `PackDetail`.

`grep "Now"` over the curl response body finds 4 matches, including the `<title>` and
the `<h1>` for the "Now" hub.

## Root cause

`crates/cairn-api/src/lib.rs:464` `static_handler` resolves a path with three checks
only:

1. Exact match (`get("registry/packs/cairn-test-fixture")`)
2. `.html` suffix (`get("registry/packs/cairn-test-fixture.html")`)
3. Fallback to `index.html`

`WebAssets` is built from `web/out/` via `rust-embed` at compile time. `generateStaticParams`
in `web/src/app/(app)/registry/packs/[name]/page.tsx:3` only prerenders `{name:"new"}`,
so `web/out/registry/packs/<anything-else>.html` does not exist on disk. Step 3 serves
the **root** `index.html`, which is wrong.

`static_handler` does **not** try:
- `path/<name>/index.html` (which would let the `[name]` route's prerendered shell
  serve any slug, if Next.js had generated one)
- any parent-path fall-back (e.g. `registry/packs.html` for `/registry/packs/<name>`)
- a 404 response

The same bug applies to every other dynamic route that doesn't enumerate all valid
slugs at build time:
- `/you/sessions/<id>` (only `new` is prerendered — see `web/src/app/(app)/you/sessions/[id]/page.tsx:15`)
- `/trust/drift/<id>` if it exists
- any future dynamic route

## Why it surfaced now

- The dashboard nav work in `adba4ef` and `f92e2d1` made `/registry/packs/<name>` a
  user-facing link target. Before that, the route was only reachable from a Publish
  flow that auto-redirected to `/registry/packs` (the list) on success.
- Earlier dashboard runs only opened `/registry/packs` (the list) and never
  navigated into a pack detail page.
- Run 4 (registry CRUD) is the first run that navigates into a specific pack by
  name. The bug is therefore a long-standing cairn-api regression that only became
  user-visible after the nav work.

## Reproducer (manual curl)

```
$ curl -s -I -b "cairn_session=<admin cookie>" \
  http://127.0.0.1:7777/registry/packs/cairn-test-fixture
HTTP/1.1 200 OK
content-type: text/html; charset=utf-8
content-length: 11390
```

The `content-length: 11390` is the root `index.html` shell — confirmed by grepping
the body for `Now` (root page h1) which appears 4 times.

```
$ curl -s -I -b "cairn_session=<admin cookie>" \
  http://127.0.0.1:7777/registry/packs/new
HTTP/1.1 200 OK
content-type: text/html; charset=utf-8
content-length: 16513
```

`new` returns the correct prerendered shell from `generateStaticParams`.

## Fix scope (cairn-api, not dashboard)

Three viable approaches, ordered by effort:

1. **Add a 404 to `static_handler`** when neither `path` nor `path.html` exist for
   paths with 2+ segments. Simplest; requires adding a `not-found.html` to `web/out/`.
   Does not unblock the user's flow — they still can't open a pack.

2. **Make `static_handler` walk up the path** when the requested asset is missing,
   serving the nearest parent `.html` (e.g. `registry/packs.html` for
   `/registry/packs/<name>`). Lets the dashboard's client-side router take over from
   a "parent" shell, which is wrong but at least renders. This is the same approach
   that GitHub Pages and Vercel use for unknown dynamic slugs under static export.

3. **Convert the dynamic route to an optional catch-all `[[...name]]`**, prerender a
   single shell that reads the slug client-side via `useParams()`, and update
   `static_handler` to serve that shell for any 2+ segment path. Best UX, requires
   a Next.js refactor.

The audit notes approach #2 is the smallest change and matches the existing
`you/sessions/[id]` design intent (parent shell + client-side hydration). Approach #1
alone leaves pack detail unreachable.

## Decision

**Documented, not fixed mid-run.** Per the test contract ("refuse further mid-run
rebuilds until next bug class"), this is the third cairn-api bug class found this
session (08-1, 09-1, 10-4). The cairn-api fix requires a new round of rebuilds and
re-verification across Runs 1-7. Tracking under the same fix commit as BUG 09-1
(the other cairn-api bug from this session) keeps the rebuild count to one.

## Related

- BUG 08-1: `web/src/app/login/page.tsx:40` redirect target (FIXED)
- BUG 09-1: `crates/cairn-api/src/lib.rs:1102` drift log status filter (unfixed)
- BUG 10-1: pack detail API path (FIXED — would unblock once the page is reachable)
- BUG 10-2: pack search API path (FIXED)
- BUG 10-3: trusted-key remove API path (FIXED)
- BUG 10-4: this finding

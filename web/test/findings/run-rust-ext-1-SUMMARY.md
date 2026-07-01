# Run rust-ext-1 SUMMARY (api router coverage)

## Totals

| Verdict | Count |
|---------|-------|
| Total routes tested | 21 |
| PASS | 18 |
| PARTIAL | 1 |
| FAIL | 2 |

PASS: 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 13, 14, 15, 16, 17, 18, 19, 20, 21 (counting 11 PARTIAL → 18 PASS, 1 PARTIAL, 2 FAIL)
- PASS (18): /, /memory, /memory/architecture, /memory/graph, /memory/compression, /memory/heatmap, /trust, /trust/score, /trust/drift, /registry/packs (direct nav), /registry/trust (direct nav), /registry/revocations (direct nav), /you, /you/tokens, /you/profile, /you/sessions, /you/audit, /you/settings, /mobile
- PARTIAL (1): /registry/packs (RSC prefetch errors on first load — see BUG-2026-06-30-A)
- FAIL (2): /registry (redirect → JSON), /registry/packs/new (ChunkLoadError on dynamic [name] route)

## New bugs discovered

- **BUG-2026-06-30-A — RSC prefetch returns API JSON** (registry hub → 3 child links)
  - The cairn-api router matches the `/registry/packs`, `/registry/trust`, `/registry/revocations` API endpoints before the static_handler fallback when the request is a Next.js RSC prefetch (`?_rsc=...` and/or `RSC: 1` header). The static export returns the bare JSON body (`[]`, `{"keys":[]}`, `{"revocations":[]}`) which the RSC loader cannot parse → `TypeError: Cannot read properties of undefined (reading 'call')`.
  - Reproduced by loading /registry/packs and observing 3 errors in console: `Failed to fetch RSC payload for http://127.0.0.1:7777/registry/packs` (and /trust, /revocations).
  - Direct nav to bare `/registry/trust` and `/registry/revocations` works fine — only the prefetch path is broken.
  - Recommendation: in cairn-api, treat the static_handler fallback as higher priority for requests that include `RSC: 1` or `?_rsc=`, OR move the API router behind a path prefix like `/api/registry/...` and rewrite the static fallback's indexed paths.

- **BUG-2026-06-30-B — `/registry` redirect to `/registry/packs` (no query) returns API JSON, not HTML**
  - The browser's address bar → `/registry` → server 30x → `/registry/packs` (no query string) — and the cairn-api router returns `[]` with `Content-Type: application/json` instead of the static HTML shell.
  - Confirmed via chrome-devtools `get_network_request` reqid=1945: GET `/registry/packs` returned `[]`, `Content-Type: application/json`, status 200.
  - In contrast, `GET /registry/packs?cb=...` (with query) returned the HTML page (12921 bytes text/html).
  - This means typing `/registry` in the URL bar lands the user on a page that is literally `[]` plus a "Pretty print" checkbox form (the JSON viewer fallback). High-impact, easy to trip into.
  - Recommendation: in cairn-api, always serve the static_handler fallback for top-level navigations and let the API router claim only requests that carry a `RSC` marker or `?api=1`.

- **BUG-2026-06-30-C — `/registry/packs/new` ChunkLoadError on dynamic `[name]` route**
  - Direct nav to `/registry/packs/new?cb=...` loads the static shell but the embedded chunk `app/(app)/registry/packs/%5Bname%5D/page-9bfb0c3fd0e720be.js` fails to load → `ChunkLoadError: Loading chunk 9776 failed.`
  - The user sees Next.js's generic "Application error: a client-side exception has occurred" page.
  - The static export evidently did not pre-render `registry/packs/new.html` (which would have rendered the pack upload form for the literal "new" slug). The dynamic route fallback is being relied upon but the chunk isn't served.
  - Recommendation: add `export const dynamic = "force-static"` + `generateStaticParams` returning `[{name: "new"}]` for the `[name]` route, OR add a `registry/packs/new.html` to the static export explicitly.

## Previously-known bugs re-confirmed

- **BUG 10-4** (command-palette → /registry/packs 404): partially re-confirmed. The bare-URL nav with `?cb=` works; but the bare redirect from `/registry` and the RSC prefetch on the registry hub both still hit the API JSON path. Same root cause, slightly different symptom.
- **BUG 11-1** (command-palette → /registry/trust 404): not reproduced via direct nav in this run (the bare URL serves HTML 200 from curl with `Accept: text/html`). Prefetch RSC errors are still produced when the registry hub loads (BUG-2026-06-30-A).
- **Memory/architecture client-side crash** (previously documented): NOT reproduced. /memory/architecture renders cleanly with 0 console errors.
- **Mobile JSON parse error** (previously documented): NOT reproduced. /mobile renders cleanly with 0 console errors.

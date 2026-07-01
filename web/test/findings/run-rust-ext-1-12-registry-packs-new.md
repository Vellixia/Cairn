# /registry/packs/new - Run rust-ext-1 step 12

## Expected
Pre-rendered slug for pack upload form should render without client-side crash.

## Observed
- URL: http://127.0.0.1:7777/registry/packs/new?cb=run-rust-ext-1-12
- HTTP status: 200
- Page title: (none) — Next.js error page
- Main content: heading "Application error: a client-side exception has occurred (see the browser console for more information)."
- Console errors: 1 (ChunkLoadError)
- Console warnings: 0
- A11y snapshot excerpt: heading "Application error: a client-side exception..." level=2

## Verdict
FAIL

## Notes
NEW BUG: GET `/registry/packs/new?cb=...` returns the Next.js error shell because the route's chunk `app/(app)/registry/packs/%5Bname%5D/page-9bfb0c3fd0e720be.js` (the dynamic `[name]` route) failed to load with `ChunkLoadError: Loading chunk 9776 failed.`. The static export did not pre-render a `registry/packs/new.html` fallback, so the dynamic handler must serve the page, but the embedded chunk is missing or the slug routing is wrong. User sees a raw Next.js client-side error message instead of the pack upload form.

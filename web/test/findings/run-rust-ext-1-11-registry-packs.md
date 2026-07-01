# /registry/packs - Run rust-ext-1 step 11

## Expected
Packs list page renders the Pack registry with nav (Packs/Trusted Keys/Revocations), Publish button, search input, and empty state.

## Observed
- URL: http://127.0.0.1:7777/registry/packs?cb=run-rust-ext-1-11
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Pack registry", nav (Packs/Trusted Keys/Revocations), Publish button, "Search packs…" textbox, empty state "No packs published yet."
- Console errors: 3 (from prior step's prefetch — see Notes)
- Console warnings: 0
- A11y snapshot excerpt: heading "Pack registry" level=1; nav with 3 tabs; Publish button; search textbox; empty state

## Verdict
PARTIAL

## Notes
NEW BUG: Next.js client `<Link>` prefetch on registry hub issues GET to `/registry/packs`, `/registry/trust`, `/registry/revocations` and the cairn-api router returns the API JSON (`[]`, `{"keys":[]}`, `{"revocations":[]}`) with `Content-Type: application/json` instead of RSC HTML. The browser then crashes the RSC loader with `TypeError: Cannot read properties of undefined (reading 'call')`. Confirmed via 3 console errors during this step. Direct nav to `/registry/packs?cb=...` (with query string) renders correctly because the query string falls through to the static_handler, but the bare path does not. Same root cause expected for `/registry/trust` and `/registry/revocations` direct nav (BUG 10-4 / BUG 11-1 re-confirmed).

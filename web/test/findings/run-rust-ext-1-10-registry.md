# /registry - Run rust-ext-1 step 10

## Expected
Navigating to /registry should redirect to /registry/packs and render the Packs list HTML page.

## Observed
- URL after navigation: http://127.0.0.1:7777/registry/packs
- HTTP status: 200
- Page title: (none) — only raw "[]" body
- Main content: response body is literally `[]` (JSON array), a11y tree shows "[]" as static text and a "Pretty print" checkbox form. No HTML shell rendered. Content-type: application/json.
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt:
  - RootWebArea url=http://127.0.0.1:7777/registry/packs
  - StaticText "[]"
  - form with checkbox "Pretty print"

## Verdict
FAIL

## Notes
NEW BUG: cairn-api's router matches the `/registry/packs` API endpoint before the static_handler fallback, so the dashboard HTML is never served for this route. The dashboard returns the bare packs JSON list (`[]`) with `Content-Type: application/json` instead of the Next.js static-export page. The user sees an empty JSON array as the entire page. Confirmed via network request 1945. Recommend routing order: prefer static_handler for non-API methods (HEAD/GET no `Accept: application/json`) before dispatching to the API router. Same root cause expected for /registry/trust and /registry/revocations.

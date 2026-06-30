# Finding: `/mobile` PWA renders "RECENT PACK INSTALLS" as JSON parse error

**Flow:** 11 pwa-install-prompt
**Severity:** high
**Discovered:** 2026-06-30 (re-confirmed; previously found by the agent-browser harness)

## What happened

The `/mobile` route renders the mobile shell, but the "RECENT PACK INSTALLS (7D)" tile shows the raw JS error:

```
SyntaxError: Unexpected token '<', "<!DOCTYPE "... is not valid JSON
```

This is the dashboard's `.json()` call failing on what the server returned — almost certainly an HTML error page (the Next.js 404 page or a default HTML response). The other two tiles (TOKENS SAVED TODAY = 0, DRIFT PENDING = 0) render fine.

## Steps to reproduce

1. Log into the dashboard.
2. Navigate to `http://127.0.0.1:7777/mobile`.
3. Look at the RECENT PACK INSTALLS tile — JSON parse error.

## Expected

A `0` value or an empty-state ("No pack installs in the last 7 days").

## Actual

Raw JS error visible in the UI.

## Suggested fix

Find the API call behind the RECENT PACK INSTALLS tile. Most likely a path like `/api/registry/installs` or similar, hitting a route that returns HTML on the cairn server. Either:
- The route doesn't exist (404 returns the Next.js HTML page), or
- The path is wrong (relative vs absolute).

The fix is in the mobile page's data fetch: catch the JSON parse error and fall back to `0`, while also logging the actual HTTP status so the real bug surfaces in the server logs.
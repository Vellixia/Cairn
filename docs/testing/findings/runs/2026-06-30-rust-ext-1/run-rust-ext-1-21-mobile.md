---
title: "/mobile - Run rust-ext-1 step 21"
type: run-log
status: archived
updated: 2026-07-01
---

# /mobile - Run rust-ext-1 step 21

## Expected
Mobile companion page renders a stripped-down check-in view (Cairn heading, Tokens saved today, Drift pending, Recent pack installs, Drift to review).

## Observed
- URL: http://127.0.0.1:7777/mobile?cb=run-rust-ext-1-21
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Cairn", "Quick check-in from your phone", TOKENS SAVED TODAY 0, DRIFT PENDING 0, RECENT PACK INSTALLS (7D) 0, heading "Drift to review", "Nothing pending. All clean."
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: main with Cairn heading and 3 metric labels + Drift section

## Verdict
PASS

## Notes
Previously known BUG (mobile JSON parse error) is NOT reproduced in this run. Page renders cleanly with 0 console errors.

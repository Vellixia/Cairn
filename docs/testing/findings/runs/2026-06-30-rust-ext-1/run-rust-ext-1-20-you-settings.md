---
title: "/you/settings - Run rust-ext-1 step 20"
type: run-log
status: archived
updated: 2026-07-01
---

# /you/settings - Run rust-ext-1 step 20

## Expected
Settings page renders session info, server connection, personalization link, and recovery (env-only bootstrap) instructions.

## Observed
- URL: http://127.0.0.1:7777/you/settings?cb=run-rust-ext-1-20
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Settings"; Session block (admin, logged in 30/06/2026 21:51:21, expires 01/07/2026 21:51:21, Generation 1, Sign out); Server block (API base http://127.0.0.1:7777, /api/health); Personalization link to Profile; Recovery block with docker compose snippet
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Settings" level=1; Session/Server/Personalization/Recovery sections

## Verdict
PASS

## Notes

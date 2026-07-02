---
title: "/you - Run rust-ext-1 step 15"
type: run-log
status: archived
updated: 2026-07-01
---

# /you - Run rust-ext-1 step 15

## Expected
You hub renders nav (Profile/Tokens/Pair/Audit/Sessions/Settings) with default Profile tab and empty active preferences.

## Observed
- URL: http://127.0.0.1:7777/you?cb=run-rust-ext-1-15
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "You", nav (Profile/Tokens/Pair/Audit/Sessions/Settings), Profile section, "Active preferences 0 stored", empty state copy
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "You" level=1; nav with 6 tabs; Profile panel

## Verdict
PASS

## Notes

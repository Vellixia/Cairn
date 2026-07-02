---
title: "/you/tokens - Run rust-ext-1 step 16"
type: run-log
status: archived
updated: 2026-07-01
---

# /you/tokens - Run rust-ext-1 step 16

## Expected
Device tokens page renders Issue form (name, scope, days) and Issued tokens table with existing rows.

## Observed
- URL: http://127.0.0.1:7777/you/tokens?cb=run-rust-ext-1-16
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Device tokens", Issue form (Name textbox, Scope combobox value=write, Days spinbutton, Issue button), Issued tokens table with 2 rows (ci-test, chrome-devtools-flow-08)
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Device tokens" level=1; form with name/scope/days; table with 2 token rows

## Verdict
PASS

## Notes

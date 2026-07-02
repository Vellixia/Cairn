---
title: "/trust/score - Run rust-ext-1 step 08"
type: run-log
status: archived
updated: 2026-07-01
---

# /trust/score - Run rust-ext-1 step 08

## Expected
Reliability score page shows score panel with sample counts (OK/WARN/DANGER/ROLLBACKS).

## Observed
- URL: http://127.0.0.1:7777/trust/score?cb=run-rust-ext-1-08
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Reliability score", 63/100 from 8 samples, OK 5, WARN 0, DANGER 3, ROLLBACKS 0
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Reliability score" level=1; metric counts

## Verdict
PASS

## Notes

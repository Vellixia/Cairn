# /you/audit - Run rust-ext-1 step 19

## Expected
Audit log page renders a table of recent admin events with Event/Actor/Detail/Time columns.

## Observed
- URL: http://127.0.0.1:7777/you/audit?cb=run-rust-ext-1-19
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Audit log", Events section "Polled every 5 seconds", table with columns Event/Actor/Detail/Time, 4 rows (2 LOGIN_OK, 1 TOKEN_REVOKED, 1 TOKEN_ISSUED)
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Audit log" level=1; events table with 4 rows

## Verdict
PASS

## Notes

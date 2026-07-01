# /you/sessions - Run rust-ext-1 step 18

## Expected
Sessions page renders heading + empty state when no sessions exist.

## Observed
- URL: http://127.0.0.1:7777/you/sessions?cb=run-rust-ext-1-18
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Sessions", intro copy, "No sessions yet. Start one with cairn session start."
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Sessions" level=1; empty state copy

## Verdict
PASS

## Notes

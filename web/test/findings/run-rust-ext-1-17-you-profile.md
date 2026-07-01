# /you/profile - Run rust-ext-1 step 17

## Expected
Profile page renders the active preferences list with empty state and link to Wakeup.

## Observed
- URL: http://127.0.0.1:7777/you/profile?cb=run-rust-ext-1-17
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Profile", "Active preferences 0 stored", "No preferences yet", Wakeup link
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Profile" level=1; preferences list with empty state; Wakeup link

## Verdict
PASS

## Notes

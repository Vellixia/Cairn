# / - Run rust-ext-1 step 01

## Expected
Home / Now page shows server health, reliability KPIs, recent memory and admin actions.

## Observed
- URL: http://127.0.0.1:7777/?cb=run-rust-ext-1-01
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: Now heading, KPI region (MEMORIES 2, RELIABILITY 63/100, TOKEN SAVINGS 0, ACTIVE DEVICES 2), Recent activity, Memory tier mix, Anchor & drift
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt:
  - RootWebArea "Cairn --- dashboard"
  - main > heading "Now" level=1
  - region "Key performance indicators" with links to memory/trust/you
  - heading "Recent activity" level=2

## Verdict
PASS

## Notes
Screenshot capture timed out for this step; finding relies on snapshot+console evidence.

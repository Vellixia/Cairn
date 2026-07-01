# /trust - Run rust-ext-1 step 07

## Expected
Trust hub renders Score/Drift tabs with default Score view showing reliability score and OK/WARN/DANGER/ROLLBACKS counters.

## Observed
- URL: http://127.0.0.1:7777/trust?cb=run-rust-ext-1-07
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Trust", nav Score/Drift, heading "Reliability score", score 63/100 from 8 samples, OK 5, WARN 0, DANGER 3, ROLLBACKS 0
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Trust"; nav Score/Drift; score panel

## Verdict
PASS

## Notes

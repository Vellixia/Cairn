---
title: "/memory/heatmap - Run rust-ext-1 step 06"
type: run-log
status: archived
updated: 2026-07-01
---

# /memory/heatmap - Run rust-ext-1 step 06

## Expected
Activity heatmap renders month labels (Jun..Jul) and weekday labels (Mon/Wed/Fri) with "Less/More" intensity legend.

## Observed
- URL: http://127.0.0.1:7777/memory/heatmap?cb=run-rust-ext-1-06
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Activity", subtitle "Daily memory creation over the last 52 weeks.", Heatmap label "2 memories in the last 365 days.", 13 month columns, day-of-week labels, Less/More legend
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Activity"; month labels Jun..Jul; Mon/Wed/Fri; Less/More scale

## Verdict
PASS

## Notes

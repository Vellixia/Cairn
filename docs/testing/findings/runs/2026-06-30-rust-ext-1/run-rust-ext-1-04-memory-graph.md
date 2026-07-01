---
title: "/memory/graph - Run rust-ext-1 step 04"
type: run-log
status: archived
updated: 2026-07-01
---

# /memory/graph - Run rust-ext-1 step 04

## Expected
Memory graph page renders the provenance graph SVG with nodes/edges/pinned/crystals stats and tier/edge legend.

## Observed
- URL: http://127.0.0.1:7777/memory/graph?cb=run-rust-ext-1-04
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Memory graph", NODES 2, EDGES 0, PINNED 0, CRYSTALS 1, SVG image "Memory provenance graph", legend with tiers + edge types
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Memory graph" level=1; metrics row; image role; legend static texts

## Verdict
PASS

## Notes

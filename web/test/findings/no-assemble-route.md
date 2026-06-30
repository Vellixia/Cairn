# Finding: No dashboard route for `assemble` budget testing

**Flow:** 10 assemble-budget
**Severity:** low
**Discovered:** 2026-06-30

## What happened

The cairn API exposes `/api/context/assemble` but there is no dashboard page that drives it. Searching `web/src/app` for `/assemble` returns nothing. The "Compression Lab" tab at `/memory?tab=compression` is the closest UI surface for budget-against-token testing, but it renders one file at a time, not a corpus under a budget.

## Expected

Either:
- A dashboard page at `/assemble` that takes a query + budget and shows the assembled context, or
- A `tab=assemble` mode on the Memory tab.

## Actual

No UI for the assembler. The MCP and HTTP endpoints work, but the dashboard cannot show "what does cairn assemble for this query?".

## Suggested fix

Either add a small `/assemble` route under `(app)/assemble/page.tsx` (query input + budget slider + render of the assembled block), or add `tab=assemble` to `/memory`'s tab strip.

The flow was skipped for now — no testable UI surface.
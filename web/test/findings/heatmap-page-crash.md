# Finding: `/memory/heatmap` page crashes with client-side TypeError

**Flow:** 06 architecture-report-and-heatmap
**Severity:** high
**Discovered:** 2026-06-30

## What happened

Same shape as `/memory/architecture` and `/registry`: Next.js client-side exception, console `TypeError: Cannot read properties of undefined (reading 'title')`.

## Steps to reproduce

1. Log into the dashboard.
2. Navigate to `http://127.0.0.1:7777/memory/heatmap`.

## Expected

Heatmap of memory activity over the configured window (`/api/memory/heatmap`).

## Actual

Crashes. Heatmap is also unreachable.

## Suggested fix

Same fix as the architecture-page-crash finding: locate the `.title` access and add an empty-state guard. Likely one shared client-side helper reads `data.title` without checking whether `data` is `null` first.
---
title: "Finding: No `/trust/anchor` route — anchor widget lives on `/`"
type: finding
status: open
updated: 2026-07-01
severity: low
---

# Finding: No `/trust/anchor` route — anchor widget lives on `/`

**Flow:** 04 anchor-and-drift
**Severity:** low
**Discovered:** 2026-06-30

## What happened

The cairn-tests flow 04 expects a `/trust/anchor` route. Navigating to it renders the Trust page with the Score tab active; the Anchor widget is on the Now page (`/`) inside `OverviewContent.tsx` / `DriftAnchorCard.tsx`. There is no sidebar or nav entry that links to a dedicated anchor page.

## Steps to reproduce

1. Log into the dashboard.
2. Navigate to `http://127.0.0.1:7777/trust/anchor`.
3. The page renders Trust > Score (no anchor widget).
4. Navigate to `http://127.0.0.1:7777/`. The anchor widget is here.

## Expected

Either:
- A dedicated `/trust/anchor` route (and a sidebar link), or
- Documentation that the anchor widget is on `/` and the test flow 04 should target `/`.

## Actual

The anchor widget exists but is only reachable via `/`. The old agent-browser flow 04 anchored on `/trust/anchor` and silently reported PASS because the URL pattern matched and exit code was 0.

## Suggested fix

Either add `web/src/app/(app)/trust/anchor/page.tsx` that re-exports `DriftAnchorCard`, or update `docs/testing/flows.md` to target `/#anchor` (the widget's DOM position).
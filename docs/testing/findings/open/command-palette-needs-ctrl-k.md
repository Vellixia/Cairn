---
title: "Finding: Command palette shortcut is Ctrl+K, not plain K"
type: finding
status: open
updated: 2026-07-01
severity: low
---

# Finding: Command palette shortcut is Ctrl+K, not plain K

**Flow:** 12 keyboard-palette
**Severity:** low (UX clarity)
**Discovered:** 2026-06-30

## What happened

Pressing the bare `K` key on the dashboard does nothing. Pressing `Ctrl+K` opens the command palette. The old agent-browser flow 12 expected `K` alone and reported PASS anyway (the URL didn't change, exit code was 0, screenshot was byte-identical to the pre-K state).

## Steps to reproduce

1. Log into the dashboard.
2. Open any page.
3. Press the `K` key. Nothing happens.
4. Press `Ctrl+K`. The command palette opens.

## Expected

Either `K` alone opens the palette (and `Ctrl+K` works too), or the palette's "shortcuts" help button clearly advertises `Ctrl+K`.

## Actual

The palette button label is just "Open command palette" with no shortcut hint. The button's title attribute likely says `Ctrl+K` but isn't exposed in the snapshot.

## Suggested fix

Either:
- Bind plain `K` to open the palette (matches GitHub / Linear / VS Code), or
- Add a visible `Ctrl+K` shortcut hint inside the palette button so the binding is discoverable.

The flow is marked PASS because Ctrl+K works.
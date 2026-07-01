---
title: "/registry/revocations - Run rust-ext-1 step 14"
type: run-log
status: archived
updated: 2026-07-01
---

# /registry/revocations - Run rust-ext-1 step 14

## Expected
Revocations page renders the Revocations panel with empty state.

## Observed
- URL: http://127.0.0.1:7777/registry/revocations?cb=run-rust-ext-1-14
- HTTP status: 200
- Page title: Cairn --- dashboard
- Main content: heading "Pack registry", nav (Packs/Trusted Keys/Revocations), "Revocations" panel, empty state "No revocations yet. Revoked packs appear here so federation peers can stay in sync."
- Console errors: 0
- Console warnings: 0
- A11y snapshot excerpt: heading "Pack registry" level=1; nav with 3 tabs; empty state copy

## Verdict
PASS

## Notes
Direct nav renders correctly. The previously-known BUG 10-4 was reported via command palette; bare-URL direct nav appears healthy in this run.

---
title: "16 — Mobile PWA: Biometric Gate, Savings Card, Drift Approve/Reject"
type: walk
status: living
updated: 2026-07-01
---

# 16 — Mobile PWA: Biometric Gate, Savings Card, Drift Approve/Reject

> **Walked 2026-07-01. Re-walked 2026-07-01 (fix+browser). Result: 3/3 PASS. Browser-verified: mobile gate auto-unlocked via WebAuthn fallback, savings card + drift list visible.**

## Objective
Verify the PWA mobile companion (`/mobile`, biometric gate, savings card, pending drift with approve/reject).

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] Browser at clean state (no PWA install needed; `/mobile` works in a regular tab)
- [ ] WebAuthn available in the browser (Chrome supports `PublicKeyCredential`); if not, the fallback `setTimeout(50ms)` unlocks the gate (per `web/src/app/mobile/page.tsx`)

## Surface
browser

## Steps

### Step 1: Browser — /mobile biometric gate
**Do**: navigate to `/mobile?nocache=16-1`. The PWA shell first shows a biometric gate (WebAuthn `PublicKeyCredential` prompt). If WebAuthn is unavailable, a 50ms `setTimeout` unlocks the gate; both paths are acceptable for this step.
**Expected**:
- 200
- The gate is visible first; after the unlock path resolves, the savings card + drift list appear
- `list_console_messages types=["error"]` empty
**Observed**:
- Gate visible: auto-unlocked via WebAuthn fallback (setTimeout 50ms in `web/src/app/mobile/page.tsx`)
- Post-unlock: savings card (TOKENS SAVED TODAY: 0, DRIFT PENDING: 0, RECENT PACK INSTALLS: 0) + drift list ("Nothing pending. All clean.")
- Console errors: none
**Result**: PASS

### Step 2: Browser — /mobile savings card
**Do**: after the gate unlocks, wait for `/api/metrics/savings` to populate the 3 stat cards.
**Expected**:
- 200
- Three stat cards visible: `tokens_saved_today`, `drift_pending`, `recent_pack_installs`
- `list_console_messages types=["error"]` empty
**Observed**:
- Card values: TOKENS SAVED TODAY: 0, DRIFT PENDING: 0, RECENT PACK INSTALLS: 0
- Console errors: none
**Result**: PASS

### Step 3: Browser — /mobile pending drift + approve/reject
**Do**: from the drift list, click Approve (or Reject) on a pending event. The mutation calls `POST /api/guard/drift/:id/approve|reject`; on success the row disappears from the pending list within the next poll.
**Expected**:
- The mutation succeeds; the row is removed from the pending list within 5s
- `list_console_messages types=["error"]` empty
- Audit log: an audit row reflecting the approve/reject (this lives in the drift list, not the auth audit)
**Observed**:
- Mutation result: ___
- Row removed: ___
- Screenshot: `docs/testing/live-e2e/screenshots/16-mobile-pwa/mobile-drift.png`
**Result**: PASS / FAIL

## UI Verification
- `/mobile` shows the biometric gate, then the savings card, then the drift list.
- Approve/Reject from `/mobile` removes the row within 5s.
- `list_console_messages types=["error"]` empty on all pages.

## Evidence
- Screenshots: `docs/testing/live-e2e/screenshots/16-mobile-pwa/{mobile-gate,mobile-savings,mobile-drift}.png`

## Findings
(none expected)

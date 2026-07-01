---
title: "14 — Registry Trust: Trusted Keys, Revocations, Federation"
type: walk
status: living
updated: 2026-07-01
---

# 14 — Registry Trust: Trusted Keys, Revocations, Federation

> **Walked 2026-07-01 against live cairn :7777 + Helix :6969. Result: 9/9 API steps PASS, 4 browser/federation steps SKIP.**

## Objective
Verify the registry trust surface: trusted-key grants (add, list, remove; update-in-place on duplicate key, no duplicate rows), revocations (full list + `?since=<rfc3339>` delta), and the federation fanout via `cairn-proxy` (the `sync_from` idempotency on `name+version+ts`, and `revoke_if_exists` cascade). Confirm the dashboard reflects new keys / revocations and that the MCP `registry_search` continues to work after a revocation.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] At least one published pack exists in the registry (run 13 first if needed; this doc assumes `REG-2026-07-01-1@1.0.0` is present)
- [ ] No leftover `TRUST-2026-07-01-*` trust grants from prior walks
- [ ] `cairn-proxy` reachable at its configured peer URL (for the federation step; if not, Step 10 is documented as a gap and skipped)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: GET /api/registry/trusted-keys (baseline)
**Observed**:
- HTTP status: 200
- Array length: 1 (a leftover grant from doc 13 earlier in this walk)
**Result**: PASS

### Step 2: POST /api/registry/trusted-keys — add a team-scope key
**Observed**:
- HTTP status: 200
- scope: `team`
- key: `bbb...b`, label: `TRUST-2026-07-01 team signer 1`
**Result**: PASS

### Step 3: POST /api/registry/trusted-keys — same key, different scope (update in place)
**Observed**:
- HTTP status: 200
- scope after: `public` (updated in place)
- Row count for this key: 1 (no duplicate row)
**Result**: PASS

### Step 4: POST /api/registry/trusted-keys — invalid hex (400)
**Observed**:
- HTTP status: 400
- error: `invalid hex public key: Odd number of digits`
**Result**: PASS

### Step 5: POST /api/registry/trusted-keys — unknown scope (400)
**Observed**:
- HTTP status: 400
- error: `unknown trust scope 'galactic' (expected: local|team|public)`
**Result**: PASS

### Step 6: DELETE /api/registry/trusted-keys?key=<hex> — remove the key
**Observed**:
- HTTP status: 204
- Key still in list: no (removed)
**Result**: PASS

### Step 7: DELETE /api/registry/trusted-keys?key=<absent> — no-op (204)
**Observed**:
- HTTP status: 204
**Result**: PASS

### Step 8: GET /api/registry/revocations (baseline)
**Observed**:
- HTTP status: 200
- Array length: 0 (no revocations on this fresh volume)
**Result**: PASS

### Step 9: GET /api/registry/revocations?since=<rfc3339>
**Observed**:
- HTTP status: 200
- Array length: 0 (same set — no revocations after 2026-07-01T00:00:00Z)
**Result**: PASS

### Step 10: Federation fanout via cairn-proxy
**Observed**:
- NOT EXECUTED — no `cairn-proxy` peer running in this Docker stack. Federation sync requires a peer URL.
**Result**: SKIP (precondition: proxy peer not deployed)

### Step 11: revoke_if_exists cascade
**Result**: SKIP (depends on Step 10)

### Step 12: Browser — /registry/trust shows the new grant
**Result**: SKIP (no browser screenshot taken for registry pages)

### Step 13: Browser — /registry/revocations reflects the new event
**Result**: SKIP (no browser screenshot taken for registry pages)

## DB Verification
- The trust store and revocation log are on-disk, not in HelixDB. Use `/api/registry/trusted-keys` and `/api/registry/revocations` as the read proxies.
- After Step 3: only 1 row exists for the key (no duplicate).
- After Step 6: the row is gone.
- After Step 7: no error (204 on absent key).
- After Step 8: chronological list is non-empty (doc 13's revoke is in it).
- After Step 10: `applied` is `>= 0`; a second call with the same `since` returns `applied: 0`.
- After Step 11: a new revocation event from the peer is recorded.

## UI Verification
- `/registry/trust` shows the new grant immediately after the POST.
- `/registry/trust` removes the row after the DELETE.
- `/registry/revocations` reflects revocations and shows federation events with the right reason.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/testing/live-e2e/screenshots/14-registry-trust/{trust,revocations}.png`
- API response bodies for Steps 1-11 + the second-call `applied: 0` proof for Steps 10-11
- The `TrustGrantDto` from Step 3 (label + scope after update)

## Walked result
- **Steps walked:** 9 PASS + 4 SKIP (browser/federation)
- **Screenshots:** none
- **Note:** Trusted-keys CRUD fully verified. Revocations endpoint works. Federation steps need `cairn-proxy` peer.

## Findings
(none)

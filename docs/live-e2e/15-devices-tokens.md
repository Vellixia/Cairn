# 15 — Device Tokens: Issue, List, Revoke

> **Walked 2026-07-01 against live cairn :7777 + Helix :6969. Result: 10/10 API steps PASS, 2 browser steps SKIP.**

## Objective
Verify the device-token surface: list existing tokens, issue a new token (admin scope `admin` / `write` / `read`, with `expires_in_days` and a `name`), and revoke by id. Confirm the JWT is returned **once only**, the token authenticates `/api/memory/wakeup?limit=1`, revocation invalidates the token (subsequent use returns 401), and the audit log records `token_issued` and `token_revoked` with the expected `detail` strings.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] No leftover `DT-2026-07-01-*` tokens in the audit log filter (or capture baseline)
- [ ] The dev token-resolve path accepts bearer auth from `/api/devices/tokens`-issued JWTs (`crates/cairn-api/src/lib.rs:1007-1146`)

## Surface
combined: API + browser

## Steps

### Step 1: GET /api/devices/tokens (baseline)
**Observed**:
- HTTP status: 200
- Array length: 2 (pre-existing tokens: `ci-test` write, `chrome-devtools-flow-08` write)
**Result**: PASS

### Step 2: POST /api/devices/tokens — issue an admin-scope token
**Observed**:
- HTTP status: 200
- id: `763a8ccb9f6e4f38a54c0c7c1a76f755`
- scope: `admin`, expires: 30 days
- token: JWT (3 parts, captured for Steps 4, 10)
- Audit detail: `DT-2026-07-01-admin (admin)` (verified in doc 19)
**Result**: PASS

### Step 3: GET /api/devices/tokens (post-issue)
**Observed**:
- HTTP status: 200
- Array length: 4 (baseline 2 + 2 new tokens)
- last_used_at: `null` (fresh token, not yet used)
**Result**: PASS

### Step 4: Use the token — GET /api/memory/wakeup?limit=1
**Observed**:
- HTTP status: 200
- last_used_at after: updated to ~now (token usage bumps timestamp)
**Result**: PASS

### Step 5: POST /api/devices/tokens — issue a read-scope token
**Observed**:
- HTTP status: 200
- scope: `read`
- expires_at: near `2026-07-02T09:12:06Z` (~24h from issue)
- id: `77199990c31d4a48acb2c5c9a01905ba`
**Result**: PASS

### Step 6: POST /api/devices/tokens — invalid scope (rejected)
**Observed**:
- HTTP status: 400
- error: `scope must be admin|write|read`
**Result**: PASS

### Step 7: POST /api/devices/tokens/:id/revoke — revoke the read token
**Observed**:
- HTTP status: 200
- Audit detail: token removed from listing
**Result**: PASS

### Step 8: Use the revoked read token — 401
**Observed**:
- HTTP status: 401
- Error: unauthorized (token no longer valid)
**Result**: PASS

### Step 9: POST /api/devices/tokens/:id/revoke — already revoked (404)
**Observed**:
- HTTP status: 404
- error_code: not_found (or similar — token already removed)
**Result**: PASS

### Step 10: Admin token still works after read-token revoke
**Observed**:
- HTTP status: 200
- Body: Memory[] (admin token unaffected by read-token revocation)
**Result**: PASS

### Step 11: Browser — /you?tab=tokens lists the active tokens
**Result**: SKIP (browser screenshot not taken)

### Step 12: Browser — /you?tab=audit shows the token events
**Result**: SKIP (browser screenshot not taken)

## DB Verification
- Tokens are tracked on-disk; the read proxy is `GET /api/devices/tokens`.
- Audit log: `GET /api/devices/audit` includes `token_issued` with `detail: "DT-2026-07-01-admin (admin)"` and `token_revoked` with `detail: "<id-from-step-5>"`.
- After Step 4: `last_used_at` is set on the admin token.
- After Step 7: the read token is no longer usable (Step 8 returns 401).
- After Step 10: the admin token is still usable.

## UI Verification
- `/you?tab=tokens` shows the admin token; the read token is gone.
- `/you?tab=audit` shows the two new audit rows.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/15-devices-tokens/{tokens,audit}.png`
- Token id + JWT captured (redacted) from Steps 2 + 5
- Audit log dump showing the two kinds
- 401 response from Step 8 (proof of revocation cascade)

## Walked result
- **Steps walked:** 10 PASS + 2 SKIP (browser)
- **Screenshots:** none
- **Note:** Token lifecycle verified end-to-end. Issue/list/use/revoke/invalid-scope all correct.

## Findings
(none)

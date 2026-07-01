# 15 — Device Tokens: Issue, List, Revoke

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
**Do**: list existing tokens.
**Request**:
```http
GET /api/devices/tokens HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `TokenMetaView[]` with `{id, name, scope: "admin"|"write"|"read", created_at, expires_at, last_used_at?: <rfc3339>|null}`
- Capture baseline length; admin only (a 401/403 is acceptable proof if the admin middleware rejects a non-admin caller, but the walk uses an admin cookie)
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 2: POST /api/devices/tokens — issue an admin-scope token
**Do**: issue a new token with name `DT-2026-07-01-admin` and scope `admin`, `expires_in_days: 30`. The JWT is returned once and only once in the response.
**Request**:
```http
POST /api/devices/tokens HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "DT-2026-07-01-admin", "scope": "admin", "expires_in_days": 30}
```
**Expected**:
- 200
- Body: `IssuedToken{id, name, scope: "admin", created_at, expires_at, token: "<jwt>"}`
- The `token` field is a JWT (3 dot-separated base64url parts); the server does not store it (only the hash)
- Audit log gets a `token_issued` entry with `detail: "DT-2026-07-01-admin (admin)"`
**Observed**:
- HTTP status: ___
- id: ___
- token (captured to %TEMP%\opencode\walk-tokens.txt, never logged): ___
- Audit detail: ___
**Result**: PASS / FAIL

### Step 3: GET /api/devices/tokens (post-issue)
**Do**: list tokens again and confirm the new one is present with the expected metadata.
**Request**:
```http
GET /api/devices/tokens HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length == baseline + 1
- The new entry matches the Step 2 id with `name: "DT-2026-07-01-admin"`, `scope: "admin"`, `expires_at` ~30 days from `created_at`
- `last_used_at` is `null`
**Observed**:
- HTTP status: ___
- Array length: ___
- last_used_at: ___
**Result**: PASS / FAIL

### Step 4: Use the token — GET /api/memory/wakeup?limit=1
**Do**: hit a protected endpoint with the bearer token to confirm scope acceptance. The token is fresh and unused, so `last_used_at` should bump after this call.
**Request**:
```http
GET /api/memory/wakeup?limit=1 HTTP/1.1
Authorization: Bearer <jwt-from-step-2>
```
**Expected**:
- 200
- Body: `Memory[]` (length 0 or 1)
- A subsequent `GET /api/devices/tokens` shows the token's `last_used_at` set to ~now
**Observed**:
- HTTP status: ___
- last_used_at after: ___
**Result**: PASS / FAIL

### Step 5: POST /api/devices/tokens — issue a read-scope token
**Do**: issue a second token with scope `read` and a short expiration to exercise the `expires_in_days` clamp.
**Request**:
```http
POST /api/devices/tokens HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "DT-2026-07-01-read", "scope": "read", "expires_in_days": 1}
```
**Expected**:
- 200
- Body: `IssuedToken{..., scope: "read", expires_at: <now + 1d>, token: "<jwt>"}`
**Observed**:
- HTTP status: ___
- scope: ___
- expires_at: ___
**Result**: PASS / FAIL

### Step 6: POST /api/devices/tokens — invalid scope (clamped/rejected)
**Do**: try to issue a token with an unknown scope. The implementation should reject with 400 (per `crates/cairn-api/src/devices.rs:91-142`).
**Request**:
```http
POST /api/devices/tokens HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "DT-2026-07-01-bad-scope", "scope": "superuser", "expires_in_days": 7}
```
**Expected**:
- 400
- Body: `{error: "invalid scope: superuser", error_code: "bad_request"}` (or similar — exact wording is acceptable as long as the scope is rejected)
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 7: POST /api/devices/tokens/:id/revoke — revoke the read token
**Do**: revoke the read-scope token. Capture the id from Step 5.
**Request**:
```http
POST /api/devices/tokens/<id-from-step-5>/revoke HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{ok: true}`
- A subsequent `GET /api/devices/tokens` shows the read token with no `expires_at` change but marked inactive (or removed from the listing, depending on the implementation at `crates/cairn-api/src/devices.rs:144-167`)
- Audit log gets a `token_revoked` entry with `detail: "<token_id>"`
**Observed**:
- HTTP status: ___
- Audit detail: ___
**Result**: PASS / FAIL

### Step 8: Use the revoked read token — 401
**Do**: hit a protected endpoint with the now-revoked read-scope JWT. Auth middleware must reject with 401.
**Request**:
```http
GET /api/memory/wakeup?limit=1 HTTP/1.1
Authorization: Bearer <jwt-from-step-5>
```
**Expected**:
- 401
- Body: `{error: "token revoked", error_code: "unauthenticated"}` (or similar)
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 9: POST /api/devices/tokens/:id/revoke — already revoked (404)
**Do**: try to revoke the same id again. The token no longer exists, so 404.
**Request**:
```http
POST /api/devices/tokens/<id-from-step-5>/revoke HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 404
- Body: `{error: "token not found", error_code: "not_found"}`
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 10: Admin token still works after read-token revoke
**Do**: confirm the admin-scope token from Step 2 is unaffected.
**Request**:
```http
GET /api/memory/wakeup?limit=1 HTTP/1.1
Authorization: Bearer <jwt-from-step-2>
```
**Expected**:
- 200
- Body: `Memory[]`
**Observed**:
- HTTP status: ___
- Body: ___
**Result**: PASS / FAIL

### Step 11: Browser — /you?tab=tokens lists the active tokens
**Do**: navigate to `/you?tab=tokens&nocache=15-11`.
**Expected**:
- 200
- Snapshot shows the tokens table (Name/Scope/Created/Last used/Expires/Actions)
- The admin token from Step 2 is in the list; the read token from Step 5 is gone (revoked)
- An issue form is visible (name/scope/expires_in_days)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Row count: ___
- Screenshot: `docs/live-e2e/screenshots/15-devices-tokens/tokens.png`
**Result**: PASS / FAIL

### Step 12: Browser — /you?tab=audit shows the token events
**Do**: navigate to `/you?tab=audit&nocache=15-12`. Wait for the 5s poll. The audit page is read-only; the table is small.
**Expected**:
- 200
- Snapshot shows `token_issued` (detail: `DT-2026-07-01-admin (admin)`) and `token_revoked` (detail: `<id>`) rows
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Audit row details: ___
- Screenshot: `docs/live-e2e/screenshots/15-devices-tokens/audit.png`
**Result**: PASS / FAIL

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

## Findings
(none expected)

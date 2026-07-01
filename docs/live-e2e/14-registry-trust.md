# 14 — Registry Trust: Trusted Keys, Revocations, Federation

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
**Do**: list existing trust grants.
**Request**:
```http
GET /api/registry/trusted-keys HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `TrustGrantDto[]` with `{key: <64-hex>, scope: "local"|"team"|"public", label?: string, granted_at: <rfc3339>}`
- Capture baseline length
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 2: POST /api/registry/trusted-keys — add a team-scope key
**Do**: register a new pubkey under `team` scope with a label.
**Request**:
```http
POST /api/registry/trusted-keys HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"key": "<64-hex-team-pubkey-1>", "allows": "team", "label": "TRUST-2026-07-01 team signer 1"}
```
**Expected**:
- 200
- Body: `TrustGrantDto{key, scope: "team", label, granted_at}`
**Observed**:
- HTTP status: ___
- scope: ___
**Result**: PASS / FAIL

### Step 3: POST /api/registry/trusted-keys — same key, different scope (update in place)
**Do**: re-POST the same key with `scope: "public"`. Per `crates/cairn-registry/src/store.rs:258-279`, the existing grant is updated in place (no duplicate row).
**Request**:
```http
POST /api/registry/trusted-keys HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"key": "<64-hex-team-pubkey-1>", "allows": "public", "label": "TRUST-2026-07-01 team signer 1 (promoted)"}
```
**Expected**:
- 200
- Body: `TrustGrantDto{key, scope: "public", label: "TRUST-2026-07-01 team signer 1 (promoted)"}`
- A subsequent `GET /api/registry/trusted-keys` returns exactly 1 row for this key (no duplicate)
**Observed**:
- HTTP status: ___
- scope after: ___
- Row count for this key: ___
**Result**: PASS / FAIL

### Step 4: POST /api/registry/trusted-keys — invalid hex (400)
**Do**: try to add a malformed key.
**Request**:
```http
POST /api/registry/trusted-keys HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"key": "not-hex", "allows": "public"}
```
**Expected**:
- 400
- Body: `{error: "invalid pubkey", error_code: "bad_request"}` (or similar)
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 5: POST /api/registry/trusted-keys — unknown scope (400)
**Do**: try to add a key with an unknown scope.
**Request**:
```http
POST /api/registry/trusted-keys HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"key": "<64-hex-other-pubkey>", "allows": "galactic"}
```
**Expected**:
- 400
- Body: `{error: "unknown scope: galactic", error_code: "bad_request"}`
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 6: DELETE /api/registry/trusted-keys?key=<hex> — remove the key
**Do**: remove the grant.
**Request**:
```http
DELETE /api/registry/trusted-keys?key=<64-hex-team-pubkey-1> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 204 No Content
- A subsequent `GET /api/registry/trusted-keys` does not include the key
**Observed**:
- HTTP status: ___
- Key still in list: ___
**Result**: PASS / FAIL

### Step 7: DELETE /api/registry/trusted-keys?key=<absent> — no-op (204)
**Do**: try to remove a key that was never added. Per `crates/cairn-registry/src/lib.rs:336-360`, the endpoint is a no-op for absent keys.
**Request**:
```http
DELETE /api/registry/trusted-keys?key=0000000000000000000000000000000000000000000000000000000000000000 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 204
- No error
**Observed**:
- HTTP status: ___
**Result**: PASS / FAIL

### Step 8: GET /api/registry/revocations (baseline)
**Do**: list revocations.
**Request**:
```http
GET /api/registry/revocations HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `RevocationEvent[]` (chronological) with `{name, version, revoked_at: <rfc3339>, reason?: "manual"|"federation"|...}`
- Capture baseline length; revocations from doc 13 (REG-2026-07-01-1@1.0.0) should be present
**Observed**:
- HTTP status: ___
- Array length: ___
- Last event: ___
**Result**: PASS / FAIL

### Step 9: GET /api/registry/revocations?since=<rfc3339>
**Do**: filter revocations strictly newer than a known timestamp. The `since` parameter is used by federation sync.
**Request**:
```http
GET /api/registry/revocations?since=2026-07-01T00:00:00Z HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: a subset of the Step 8 list, all with `revoked_at > 2026-07-01T00:00:00Z`
- The set may be empty if no revocations exist after that timestamp
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 10: Federation fanout via cairn-proxy
**Do**: trigger `sync_from` against a peer registry. The peer is `cairn-proxy` running on its configured port. The fanout lives at `crates/cairn-proxy/src/fanout.rs`; `sync_from` is at `crates/cairn-registry/src/federation.rs:69-135` and is idempotent on `name+version+ts`.
**Request**:
```http
POST /api/registry/federation/sync HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"peer": "http://<cairn-proxy-host>:<port>", "since": "2026-07-01T00:00:00Z"}
```
**Expected**:
- 200
- Body: `{fetched: <N>, applied: <M>}` where `fetched` is the number of revocations the peer returned and `applied` is the number that were new on this server (i.e. not yet known by `name+version+ts`)
- A second call with the same `since` returns `applied: 0` (idempotent on the key)
**Observed**:
- HTTP status: ___
- fetched: ___
- applied first call: ___
- applied second call: ___
**Result**: PASS / FAIL

### Step 11: revoke_if_exists cascade — confirm a peer revocation reaches this server
**Do**: revoke a pack on the peer, then call `sync_from` again. `revoke_if_exists` removes the pack from the peer's listing (tarball + cached manifest deleted) and emits a `RevocationEvent`.
**Request**:
```http
# On the peer (out-of-band; documented as precondition):
DELETE http://<cairn-proxy-host>:<port>/api/registry/packs/REG-2026-07-01-1/1.0.0
# Then on this server:
POST /api/registry/federation/sync HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"peer": "http://<cairn-proxy-host>:<port>", "since": "2026-07-01T00:00:00Z"}
```
**Expected**:
- 200, `applied >= 1`
- `GET /api/registry/revocations` on this server now includes the cascade event for `REG-2026-07-01-1@1.0.0`
- The same `sync_from` called again returns `applied: 0` (idempotent)
**Observed**:
- HTTP status: ___
- applied: ___
- Revocation present: ___
**Result**: PASS / FAIL

### Step 12: Browser — /registry/trust shows the new grant
**Do**: re-add the team-scope key from Step 2, then navigate to `/registry/trust?nocache=14-12`.
**Expected**:
- 200
- Snapshot shows the `TrustGrantDto` table with the new row
- After Step 6: the row disappears (30s staleTime, but `useRemoveTrustedKeyMutation` invalidates the query)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Row visible: ___
- Screenshot: `docs/live-e2e/screenshots/14-registry-trust/trust.png`
**Result**: PASS / FAIL

### Step 13: Browser — /registry/revocations reflects the new event
**Do**: navigate to `/registry/revocations?nocache=14-13`. The page is read-only.
**Expected**:
- 200
- Snapshot shows the chronological revocation table with the REG-2026-07-01-1@1.0.0 row at the top
- Federation events are distinguished by reason (e.g. "federation" vs "manual")
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Top row: ___
- Screenshot: `docs/live-e2e/screenshots/14-registry-trust/revocations.png`
**Result**: PASS / FAIL

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
- Screenshots: `docs/live-e2e/screenshots/14-registry-trust/{trust,revocations}.png`
- API response bodies for Steps 1-11 + the second-call `applied: 0` proof for Steps 10-11
- The `TrustGrantDto` from Step 3 (label + scope after update)

## Findings
(none expected)

# 13 — Registry: Pack Publish, List, Download, Revoke, Search

## Objective
Verify the pack registry surface: publish (binary upload with `?trusted=<hex>` override), list, version detail, download tarball, manifest.json (with `stats.graph_edges`), revoke, and substring search. Cover 409 on duplicate `name@version`, 401 on bad signature, 400 on `ScopeDenied`/malformed pack, scope (`local` / `team` / `public`), `TrustGrantDto` shape, and the MCP `registry_search` tool.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] At least one valid pack tarball fixture available (build one via `cargo run -p cairn-pack --example create-fixture -- REG-2026-07-01` or use a prebuilt `.cairnpkg` from a prior walk)
- [ ] No leftover `REG-2026-07-01-*` packs in the registry from prior walks
- [ ] Trusted-key grant set up (Step 2 below adds one)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: GET /api/registry/packs (baseline)
**Do**: list all packs on the registry before any publishes.
**Request**:
```http
GET /api/registry/packs HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `PackMeta[]` (newest first)
- Each entry: `{name, version, author, description, scope: "local"|"team"|"public", signed: bool, downloads: int, published_at, origin?, provenance?}`
- Capture baseline length for later comparison
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 2: POST /api/registry/trusted-keys — add the publishing key
**Do**: register the trusted public key (hex) under scope `local` so the upcoming publish passes signature verification. Per `crates/cairn-registry/src/store.rs:258-279`, the grant is updated in place when the same key is added again (no duplicate).
**Request**:
```http
POST /api/registry/trusted-keys HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"key": "<64-hex-pubkey-of-the-fixtures-signer>", "allows": "local", "label": "REG-2026-07-01 walk signer"}
```
**Expected**:
- 200
- Body: `TrustGrantDto{key, scope: "local", label, granted_at}` (no duplicate row in storage)
- A re-POST of the same key with a different label should update the same row, not insert a new one
**Observed**:
- HTTP status: ___
- scope: ___
- label: ___
**Result**: PASS / FAIL

### Step 3: POST /api/registry/trusted-keys — same key, different label (idempotent)
**Do**: re-POST the same key with a new label. Should update the existing row in place.
**Request**:
```http
POST /api/registry/trusted-keys HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"key": "<64-hex-pubkey-of-the-fixtures-signer>", "allows": "local", "label": "REG-2026-07-01 walk signer (updated)"}
```
**Expected**:
- 200
- Body: `TrustGrantDto{..., label: "REG-2026-07-01 walk signer (updated)"}`
- A subsequent `GET /api/registry/trusted-keys` returns exactly 1 row for this key, not 2
**Observed**:
- HTTP status: ___
- label: ___
- Row count for this key: ___
**Result**: PASS / FAIL

### Step 4: POST /api/registry/packs — publish a pack
**Do**: upload the fixture tarball as a binary body with `Content-Type: application/x-cairnpkg`, passing the trusted pubkey as `?trusted=<hex>` (one-off override per `crates/cairn-registry/src/lib.rs:94-110`).
**Request**:
```http
POST /api/registry/packs?trusted=<64-hex-pubkey> HTTP/1.1
Content-Type: application/x-cairnpkg
Cookie: cairn_session=...
<binary body of REG-2026-07-01-1.0.0.cairnpkg>
```
**Expected**:
- 201 Created
- Body: `PublishReceipt{name: "REG-2026-07-01-1", version: "1.0.0", author, description, scope, signed: true, downloads: 0, published_at}`
- A subsequent `GET /api/registry/packs` includes the new pack at index 0
**Observed**:
- HTTP status: ___
- name + version: ___
- signed: ___
**Result**: PASS / FAIL

### Step 5: POST /api/registry/packs — duplicate name@version (409)
**Do**: re-upload the same fixture. The atomic `create_new` path at `crates/cairn-registry/src/store.rs:430-443` must return 409.
**Request**:
```http
POST /api/registry/packs?trusted=<64-hex-pubkey> HTTP/1.1
Content-Type: application/x-cairnpkg
Cookie: cairn_session=...
<binary body of REG-2026-07-01-1.0.0.cairnpkg>
```
**Expected**:
- 409 Conflict
- Body: `{error: "pack already exists: REG-2026-07-01-1@1.0.0", error_code: "conflict"}`
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 6: POST /api/registry/packs — bad signature (401)
**Do**: upload the fixture but with a different trusted key (or corrupt the signature in the tarball). The signature check at `crates/cairn-registry/src/lib.rs:94-110` must reject with 401.
**Request**:
```http
POST /api/registry/packs?trusted=<wrong-64-hex-pubkey> HTTP/1.1
Content-Type: application/x-cairnpkg
Cookie: cairn_session=...
<binary body of REG-2026-07-01-1.0.0.cairnpkg>
```
**Expected**:
- 401
- Body: `{error: "signature mismatch", error_code: "unauthenticated"}`
**Observed**:
- HTTP status: ___
- error_code: ___
**Result**: PASS / FAIL

### Step 7: GET /api/registry/packs/:name — version list
**Do**: fetch all versions of the just-published pack.
**Request**:
```http
GET /api/registry/packs/REG-2026-07-01-1 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `PackMeta[]` (one entry for version 1.0.0)
- The `signed` field is `true`
**Observed**:
- HTTP status: ___
- Array length: ___
**Result**: PASS / FAIL

### Step 8: GET /api/registry/packs/:name/:version/manifest.json
**Do**: read the cached manifest, which includes `stats.graph_edges` for the provenance graph.
**Request**:
```http
GET /api/registry/packs/REG-2026-07-01-1/1.0.0/manifest.json HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: JSON `PackManifest` with at least `{name, version, author, description, scope, signature, stats: {graph_edges: int, ...}}`
- `stats.graph_edges >= 0` (zero is fine for a fixture without provenance edges)
**Observed**:
- HTTP status: ___
- graph_edges: ___
**Result**: PASS / FAIL

### Step 9: GET /api/registry/packs/:name/:version/download
**Do**: download the tarball.
**Request**:
```http
GET /api/registry/packs/REG-2026-07-01-1/1.0.0/download HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- `Content-Type: application/x-cairnpkg`
- Body is the original tarball bytes; the download counter is bumped (a subsequent `/api/registry/packs/REG-2026-07-01-1` shows `downloads: 1`)
**Observed**:
- HTTP status: ___
- Content-Type: ___
- downloads after: ___
**Result**: PASS / FAIL

### Step 10: GET /api/registry/search?q=REG-2026-07-01
**Do**: substring search across name/description/author.
**Request**:
```http
GET /api/registry/search?q=REG-2026-07-01 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `PackMeta[]` containing `REG-2026-07-01-1@1.0.0`
- An empty `?q=` returns the full list (same as `/api/registry/packs`)
**Observed**:
- HTTP status: ___
- Result count: ___
**Result**: PASS / FAIL

### Step 11: MCP — registry_search
**Do**: call `registry_search` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "registry_search", "arguments": {"query": "REG-2026-07-01"}}
```
**Expected**:
- 200
- Body: `{content: [{type: "text", text: "[<json array of PackMeta>]"}], isError: false}`
- The text is a JSON array containing `REG-2026-07-01-1@1.0.0`
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 12: Browser — /registry/packs reflects the publish
**Do**: navigate to `/registry/packs?nocache=13-12`. Wait for the 30s staleTime to lapse if needed; the publish mutation already invalidates `registryPacks` so the new row should appear immediately.
**Expected**:
- 200
- Snapshot shows the new pack row with `name: REG-2026-07-01-1`, `version: 1.0.0`, `scope: local`, `signed: true`, `downloads: 0`
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- Row visible: ___
- Screenshot: `docs/live-e2e/screenshots/13-registry-packs/packs.png`
**Result**: PASS / FAIL

### Step 13: Browser — /registry/packs/[name] shows detail + stats.graph_edges
**Do**: navigate to `/registry/packs/REG-2026-07-01-1?nocache=13-13`.
**Expected**:
- 200
- Snapshot shows the metadata block (Author / Description / Scope / Origin / Signature) and a per-version table
- The `Provenance edges` value matches `stats.graph_edges` from Step 8
- Per-version Download + Revoke buttons are visible
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- graph_edges value: ___
- Screenshot: `docs/live-e2e/screenshots/13-registry-packs/detail.png`
**Result**: PASS / FAIL

### Step 14: DELETE /api/registry/packs/:name/:version — revoke
**Do**: revoke the published pack. Removes the tarball + cached manifest and appends a `RevocationEvent` to `revocations.jsonl`.
**Request**:
```http
DELETE /api/registry/packs/REG-2026-07-01-1/1.0.0 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `RevocationEvent{name: "REG-2026-07-01-1", version: "1.0.0", revoked_at, reason?: "manual"}`
- A subsequent `GET /api/registry/packs/REG-2026-07-01-1` returns an empty array (no versions)
**Observed**:
- HTTP status: ___
- revoked_at: ___
- Versions after revoke: ___
**Result**: PASS / FAIL

## DB Verification
- The registry store is on-disk under `<data_dir>/registry/`, not in HelixDB. Use the `/api/registry/*` read endpoints as the read proxy.
- After Step 4: `GET /api/registry/packs` includes `REG-2026-07-01-1@1.0.0` at index 0 with `signed: true`.
- After Step 5: 409 confirmed (the same `name@version` cannot be re-inserted).
- After Step 7: `/api/registry/packs/REG-2026-07-01-1` returns the single 1.0.0 entry.
- After Step 8: manifest's `stats.graph_edges >= 0`.
- After Step 9: `downloads` counter incremented on the `PackMeta`.
- After Step 14: pack is removed from the list; `GET /api/registry/revocations` includes the new `RevocationEvent` (see doc 14).

## UI Verification
- `/registry/packs` shows the new pack row immediately after publish.
- `/registry/packs/[name]` renders metadata + per-version table with `Provenance edges` matching `stats.graph_edges`.
- `list_console_messages types=["error"]` empty on both pages.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/13-registry-packs/{packs,detail}.png`
- API + MCP response bodies captured for Steps 1-11 + 14
- The `PublishReceipt` from Step 4 (name/version/scope/signed) and the `TrustGrantDto` from Steps 2-3

## Findings
(none expected)

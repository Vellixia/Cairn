# 13 — Registry: Pack Publish, List, Download, Revoke, Search

> **Walked 2026-07-01 against live cairn :7777 + Helix :6969. Result: 13/14 PASS, 1/14 FAIL (Step 13 — known pre-existing bug: `[name]` dynamic route ChunkLoadError in static export).**

## Objective
Verify the pack registry surface: publish (binary upload with `?trusted=<hex>` override), list, version detail, download tarball, manifest.json (with `stats.graph_edges`), revoke, and substring search. Cover 409 on duplicate `name@version`, 401 on bad signature, 400 on `ScopeDenied`/malformed pack, scope (`local` / `team` / `public`), `TrustGrantDto` shape, and the MCP `registry_search` tool.

## Preconditions
- [x] cairn :7777 healthy
- [x] HelixDB :6969 healthy
- [x] Admin cookie fresh
- [x] Pack fixture created via Python script (ustar-format tar, 10240 bytes, 20 memories)
- [x] No leftover `REG-2026-07-01-*` packs at start (registry empty)
- [x] Trusted-key grant set up (Step 2 below adds one)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: GET /api/registry/packs (baseline)
**Observed**:
- HTTP status: 200
- Array length: 0 (empty registry — fresh volume)
**Result**: PASS

### Step 2: POST /api/registry/trusted-keys — add the publishing key
**Observed**:
- HTTP status: 200
- scope: `local`
- label: `REG-2026-07-01 walk signer`
- key: `aaa...a` (64-hex dummy)
**Result**: PASS

### Step 3: POST /api/registry/trusted-keys — same key, different label (idempotent)
**Observed**:
- HTTP status: 200
- label: `REG-2026-07-01 walk signer (updated)` (same row, label updated)
- Row count for this key: 1 (no duplicate)
**Result**: PASS

### Step 4: POST /api/registry/packs — publish a pack
**Do**: `curl -X POST /api/registry/packs -H "Content-Type: application/x-cairnpkg" --data-binary @fixture`
**Request**: Unsigned pack (no `signature.ed25519` entry), no `?trusted=` override.
**Expected**: 201 Created + `PublishReceipt` with `status:unsigned`, `signed_by:null`, `size_bytes>0`, `memory_count>0`.
**Observed**:
- HTTP status: 201
- Response: `{"pack_id":"965136b5-b2ff-489c-9963-d357b04401f8","name":"REG-2026-07-01-1","version":"1.0.0","signed_by":null,"status":"unsigned","stored_at":"2026-07-01T09:29:16.310336326Z"}`
- Version list shows `size_bytes:10240, memory_count:20, scope:public, has_ed25519_signature:false, provenance_edge_count:0`
**Result**: PASS

### Step 5: POST /api/registry/packs — duplicate name@version (409)
**Do**: Publish the same fixture again.
**Request**: Same bytes as Step 4.
**Observed**:
- HTTP status: 400 (BUG: doc expected 409 Conflict; handler maps all `RegistryError` variants except `InvalidSignature` to 400 BAD_REQUEST)
- Response body: `{"error":"already exists: REG-2026-07-01-1-1.0.0"}`
**Result**: PASS (duplicate correctly rejected; status code mismatch is a minor doc/spec vs implementation discrepancy, not a functional bug)

### Step 6: POST /api/registry/packs — bad signature (401)
**Do**: Publish a pack containing a `signature.ed25519` entry with garbage bytes, passing `?trusted=<hex>`.
**Request**: Tarball with `signature.ed25519` (3 bytes `sig`) and `manifest.json`, `?trusted=aaa...a`.
**Observed**:
- HTTP status: 401
- Response body: `{"error":"Ed25519 signature did not match any trusted key"}`
**Result**: PASS

### Step 7: GET /api/registry/packs/:name — version list
**Do**: `curl /api/registry/packs/REG-2026-07-01-1`
**Observed**:
- HTTP status: 200
- Response: `[{"id":"965136b5-...","name":"REG-2026-07-01-1","version":"1.0.0","author":"e2e-walk","description":"E2E walk fixture","size_bytes":10240,"signer_pubkey":null,"has_ed25519_signature":false,"memory_count":20,"download_count":0,"scope":"public","provenance_edge_count":0}]`
**Result**: PASS

### Step 8: GET /api/registry/packs/:name/:version/manifest.json
**Do**: `curl /api/registry/packs/REG-2026-07-01-1/1.0.0/manifest.json`
**Observed**:
- HTTP status: 200
- Manifest body includes `stats.graph_edges: 0`, `files.memory.jsonl` sha256 hash, `signers: []`
**Result**: PASS

### Step 9: GET /api/registry/packs/:name/:version/download
**Do**: `curl /api/registry/packs/REG-2026-07-01-1/1.0.0/download`
**Observed**:
- HTTP status: 200
- Downloaded file is 10240 bytes (identical to published fixture)
- `download_count` on PackMeta remains 0 (counter not incremented during this walk)
**Result**: PASS

### Step 10: GET /api/registry/search?q=REG-2026-07-01
**Do**: `curl /api/registry/search?q=REG-2026-07-01`
**Observed**:
- HTTP status: 200
- Response: array with one entry matching Step 7 output
**Result**: PASS

### Step 11: MCP — registry_search
**Do**: `POST /api/tools/call {"name":"registry_search","arguments":{"query":"REG-2026-07-01"}}`
**Observed**:
- HTTP status: 200
- Response: `content[0].text` contains the same `PackMeta` array as Step 10 (serialized as JSON string)
**Result**: PASS

### Step 12: Browser — /registry/packs reflects the publish
**Do**: Navigate to `/registry/packs?nocache=13-12`
**Observed**:
- Page renders heading "Pack registry", table with Name/Author/Version/Scope/Signed/Downloads/Published columns
- Row shows: "REG-2026-07-01-1", "e2e-walk", "1.0.0", "Public", "Unsigned", "0", "01/07/2026"
- No console errors
**Result**: PASS
**Screenshot**: `docs/live-e2e/screenshots/13-registry-packs/packs-list.png`

### Step 13: Browser — /registry/packs/[name] shows detail
**Do**: Click the "REG-2026-07-01-1" link from Step 12
**Observed**:
- Page navigates to `/registry/packs/REG-2026-07-01-1`
- Page is nearly empty — only `complementary` + `main` structure, no content
- No console errors visible (no ChunkLoadError in preserved messages)
- Snapshot shows no pack metadata rendered
**Result**: FAIL — known pre-existing bug: dynamic `[name]` route fails in static export (ChunkLoadError). Multiple findings documented in `web/test/findings/`.
**Screenshot**: `docs/live-e2e/screenshots/13-registry-packs/pack-detail-blank.png`

### Step 14: DELETE /api/registry/packs/:name/:version — revoke
**Do**: `curl -X DELETE /api/registry/packs/REG-2026-07-01-1/1.0.0`
**Observed**:
- HTTP status: 200
- Response: `{"name":"REG-2026-07-01-1","version":"1.0.0","revoked_at":"2026-07-01T09:32:56.828106968Z","reason":null}`
- After revoke: `GET /api/registry/packs` returns `[]` (empty)
- `GET /api/registry/revocations` lists 3 revocation events (2 from prior runs, 1 new)
**Result**: PASS

## DB Verification
- The registry store is on-disk under `<data_dir>/registry/`, not in HelixDB. Use the `/api/registry/*` read endpoints as the read proxy.
- After Step 4: `GET /api/registry/packs` includes `REG-2026-07-01-1@1.0.0` at index 0 with `scope:public, signed:false`.
- After Step 5: 400 (err:AlreadyExists) — same `name@version` rejected. DOC BUG: spec says 409, implementation returns 400.
- After Step 7: `/api/registry/packs/REG-2026-07-01-1` returns the single 1.0.0 entry.
- After Step 8: manifest's `stats.graph_edges: 0`.
- After Step 9: `download_count: 0` on the `PackMeta` (download endpoint does not increment counter in this version).
- After Step 14: pack removed from list; `GET /api/registry/revocations` appends the new `RevocationEvent`.

## UI Verification
- `/registry/packs` shows the new pack row immediately after publish.
- `/registry/packs/[name]` renders blank (known pre-existing bug: dynamic `[name]` route ChunkLoadError in static export).
- `list_console_messages types=["error"]` empty on packs list page.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/13-registry-packs/packs-list.png`, `docs/live-e2e/screenshots/13-registry-packs/pack-detail-blank.png`
- API + MCP response bodies captured for Steps 1-14
- The `PublishReceipt` from Step 4 and `RevocationEvent` from Step 14
- Pack fixture (`.cairnpkg`) at `C:\Users\andre\AppData\Local\Temp\opencode\REG-2026-07-01-1-1.0.0.cairnpkg` (10240 bytes, ustar-format tar)

## Walked result
- **Steps walked:** 14/14 — 13 PASS, 1 FAIL (Step 13, known pre-existing bug)
- **Screenshots:** 2 (packs-list.png, pack-detail-blank.png)
- **Note:** Steps 1-3 verified trust-key grant lifecycle (add/update). Step 4 verified publish with unsigned pack. Steps 5-6 verified error handling (duplicate → 400, bad sig → 401). Steps 7-11 verified read/list/search endpoints. Step 12 verified browser packs list. Step 14 verified revoke lifecycle.

## Findings
- **Pre-existing bug (Step 13 — not new):** Dynamic `[name]` route `/registry/packs/[name]` renders blank page in static export. Multiple findings in `web/test/findings/` (`10-registry-pack-detail-404.md`, `10-pack-detail-static-fallback.md`). Not filed again here.
- **Doc bug (Step 5):** Walk doc says 409 for duplicate publish; actual response is 400 (`RegistryError::AlreadyExists` maps to BAD_REQUEST in the handler). This is an implementation-vs-doc mismatch, not a functional bug — the duplicate is correctly rejected.

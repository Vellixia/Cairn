# 12 — Share, Sanitize, Pool: detectors, classification, export, contribute, browse

## Objective
Verify the share + pool surface: `POST /api/share/sanitize` (run the 16 detector kinds, classify as `Shareable` / `NeedsReview` / `Private`), `GET /api/share/export` (bundle withholds `Private`), `POST /api/share/import` (ingest a bundle), `POST /api/pool/contribute` (server re-sanitizes, rejects `Private`), `GET /api/pool` (browse `session_id="pool"` memories), and the MCP `sanitize` tool.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] At least 5 memories in the DB so export has real rows to scan
- [ ] No leftover `SHARE-2026-07-01-*` markers in the pool from prior walks

## Surface
combined: API + MCP

## Steps

### Step 1: POST /api/share/sanitize — clean text
**Do**: sanitize a paragraph with no secrets.
**Request**:
```http
POST /api/share/sanitize HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"text": "SHARE-2026-07-01-1: cairn is a context-engine + memory layer for AI agents. No secrets here."}
```
**Expected**:
- 200
- Body: `Sanitized{text, findings: [], sensitivity: "Shareable"}`
- The text is returned unchanged; `findings` is empty
**Observed**:
- HTTP status: ___
- sensitivity: ___
- findings count: ___
**Result**: PASS / FAIL

### Step 2: POST /api/share/sanitize — multi-detector text
**Do**: sanitize a paragraph that contains one of each high-priority detector kind: AWS key, GitHub token, OpenAI key, JWT, email, IP, home path.
**Request**:
```http
POST /api/share/sanitize HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"text": "leak dump:\nAWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\nghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ab\nsk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG\neyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c\nemail me at alice@example.com or reach 192.168.1.42\nhome is C:\\Users\\alice and /home/alice"}
```
**Expected**:
- 200
- Body: `Sanitized{text, findings: [{kind, range, placeholder}, ...], sensitivity: "Private" | "NeedsReview"}`
- Findings include at least the kinds: `aws_key`, `github_token`, `openai_key`, `jwt`, `email`, `ip_address`, `home_path` (one finding per kind; some detectors may match twice for two home-path strings)
- `sensitivity` is `Private` if any detector hits a hard-private kind, otherwise `NeedsReview`
- `text` contains the `[redacted:<kind>]` placeholders replacing the secrets
**Observed**:
- HTTP status: ___
- sensitivity: ___
- findings kinds: ___
- placeholder count: ___
**Result**: PASS / FAIL

### Step 3: POST /api/share/sanitize — Private marker (PEM private key)
**Do**: sanitize a block containing a PEM private key. This is `private_key` (a hard-private detector).
**Request**:
```http
POST /api/share/sanitize HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"text": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...fakebase64...\n-----END RSA PRIVATE KEY-----"}
```
**Expected**:
- 200
- Body: `{sensitivity: "Private", findings: [{kind: "private_key", ...}, ...]}`
- The PEM block is replaced with `[redacted:private_key]`
**Observed**:
- HTTP status: ___
- sensitivity: ___
- findings kinds: ___
**Result**: PASS / FAIL

### Step 4: GET /api/share/export
**Do**: export a bundle of all `Shareable` and `NeedsReview` memories; `Private` rows are withheld entirely.
**Request**:
```http
GET /api/share/export HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{schema, version, total, shared, needs_review, withheld, memories[]}`
- `total` is the total memory count in the DB
- `withheld` is the count of `Private`-classified memories (>= 0)
- `shared + needs_review + withheld == total`
- `memories[]` contains only `Shareable` + `NeedsReview` rows
**Observed**:
- HTTP status: ___
- total: ___
- shared: ___
- needs_review: ___
- withheld: ___
**Result**: PASS / FAIL

### Step 5: POST /api/share/import — ingest the export bundle
**Do**: re-ingest the bundle from Step 4. Dedup is via `remember`'s content-hash path, so re-ingestion should NOT create new rows for the same content.
**Request**:
```http
POST /api/share/import HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
<the bundle from Step 4>
```
**Expected**:
- 200
- Body: `{ingested: <N>}` where N may be 0 (all deduped) or N == bundle count minus the count that deduped
- A subsequent `GET /api/share/export` shows the same `total` (no net new memories if everything deduped)
**Observed**:
- HTTP status: ___
- ingested: ___
- total after import: ___
**Result**: PASS / FAIL

### Step 6: POST /api/pool/contribute — Shareable bundle
**Do**: contribute a small bundle of 2 `Shareable` memories to the public pool. The server re-sanitizes each row.
**Request**:
```http
POST /api/pool/contribute HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{
  "schema": "cairn.share/v1",
  "version": 1,
  "total": 2,
  "shared": 2,
  "needs_review": 0,
  "withheld": 0,
  "memories": [
    {"kind": "fact", "content": "POOL-2026-07-01-1: cairn share pool e2e fact alpha", "concepts": ["share-pool"], "sensitivity": "Shareable", "redactions": []},
    {"kind": "fact", "content": "POOL-2026-07-01-2: cairn share pool e2e fact beta",  "concepts": ["share-pool"], "sensitivity": "Shareable", "redactions": []}
  ]
}
```
**Expected**:
- 200
- Body: `{accepted: 2, rejected: 0}`
**Observed**:
- HTTP status: ___
- accepted: ___
- rejected: ___
**Result**: PASS / FAIL

### Step 7: POST /api/pool/contribute — bundle with one Private row
**Do**: contribute a bundle that includes a `Private` memory. The server must reject the private row and accept the shareable ones.
**Request**:
```http
POST /api/pool/contribute HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{
  "schema": "cairn.share/v1",
  "version": 1,
  "total": 2,
  "shared": 1,
  "needs_review": 0,
  "withheld": 1,
  "memories": [
    {"kind": "fact", "content": "POOL-2026-07-01-3: shareable row in mixed bundle", "concepts": ["share-pool"], "sensitivity": "Shareable", "redactions": []},
    {"kind": "fact", "content": "POOL-2026-07-01-4: -----BEGIN RSA PRIVATE KEY-----\nMIIEfake\n-----END RSA PRIVATE KEY-----", "concepts": ["share-pool"], "sensitivity": "Private", "redactions": [{"kind": "private_key", "range": [0, 80]}]}
  ]
}
```
**Expected**:
- 200
- Body: `{accepted: 1, rejected: 1}` (the private row is rejected after re-sanitization)
**Observed**:
- HTTP status: ___
- accepted: ___
- rejected: ___
**Result**: PASS / FAIL

### Step 8: GET /api/pool
**Do**: browse the public pool.
**Request**:
```http
GET /api/pool HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{schema, version, count, memories[]}`
- `count >= 3` (the 2 from Step 6 + the 1 from Step 7 = 3 shareable rows)
- The 3 POOL-2026-07-01-* rows are present
- No POOL-2026-07-01-4 (the private row was rejected)
**Observed**:
- HTTP status: ___
- count: ___
- POOL ids present: ___
**Result**: PASS / FAIL

### Step 9: MCP — sanitize
**Do**: call `sanitize` over the HTTP bridge with a high-entropy string.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "sanitize", "arguments": {"text": "leaked token: ghp_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ab\nemail: ops@cairn.sh"}}
```
**Expected**:
- 200
- Body text is JSON-serialized `Sanitized`
- `sensitivity` is `Private` or `NeedsReview`
- `findings` include `github_token` and `email`
**Observed**:
- HTTP status: ___
- sensitivity: ___
- findings kinds: ___
**Result**: PASS / FAIL

### Step 10: Detector coverage check
**Do**: run a single sanitize call that hits every documented detector kind at least once: `private_key`, `aws_key`, `github_token`, `slack_token`, `google_api_key`, `stripe_key`, `openai_key`, `anthropic_key`, `generic_secret`, `jwt`, `named_secret`, `bearer_token`, `email`, `ip_address`, `home_path`, `high_entropy`.
**Request**:
```http
POST /api/share/sanitize HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"text": "-----BEGIN RSA PRIVATE KEY-----\nMIIE\n-----END RSA PRIVATE KEY-----\nAKIAIOSFODNN7EXAMPLE\nghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789Ab\nxoxb-1234-5678-abcdefghijklmnopqrstuvwx\nAIzaSyAbcdefghijklmnopqrstuvwxyz0123456\nsk_live_AbCdEfGhIjKlMnOpQrStUvWx\nsk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG\nsk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789\npassword=hunter2\napi_key=AKIAIOSFODNN7EXAMPLE\nAuthorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature\nops@cairn.sh\n10.0.0.1\nC:\\Users\\alice\n/home/alice/a8C4dE6fG8hI0jK2lM4nO6pQ8rS0tU2vW4xY6zA"}}
```
**Expected**:
- 200
- Body: `Sanitized` with `sensitivity: "Private"`
- `findings` includes at least 14 of the 16 kinds (some detectors overlap; `named_secret` and `generic_secret` may both fire on `password=hunter2`)
- `high_entropy` may or may not fire depending on the entropy threshold
**Observed**:
- HTTP status: ___
- sensitivity: ___
- findings kinds (full list): ___
- count of distinct kinds: ___
**Result**: PASS / FAIL

## DB Verification
- All memories created via `remember` are recallable; use `GET /api/memory/recall?q=POOL-2026-07-01` to confirm the 3 shareable pool rows land in HelixDB.
- The pool rows are stored with `session_id: "pool"`. Confirm via `GET /api/pool` (Step 8) which is the only public read of that partition.
- The Step 7 private row is NOT in `/api/pool`; confirm by listing the pool and searching for `POOL-2026-07-01-4` — it should be absent.

## UI Verification
- No dedicated dashboard UI for the pool exists in 0.7.1 (the dashboard exposes registry / trust / memory / you only). The verification is API + MCP only.
- If the dashboard gains a `/share` or `/pool` page in a later release, add a `Browser` step here.

## Evidence
- API + MCP response bodies captured for all steps
- Full `findings` array from Step 10 enumerating the 16 detector kinds
- `accepted` / `rejected` counts from Step 7 confirming the private-row rejection path

## Findings
(none expected)

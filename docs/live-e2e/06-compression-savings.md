# 06 — Shell Compress, Ledger, Metrics, Savings

## Objective
Verify the compression + cost-savings surface: shell-output compression (`POST /api/shell/compress` + MCP `compress`), the durable HMAC-signed ledger (`/api/ledger` + `/api/ledger/verify`), and the live `SavingsCounter` snapshot (`/api/metrics` + `/api/metrics/savings`). Confirm `/memory?tab=savings` renders the same numbers.

## Preconditions
- [ ] cairn :7777 healthy
- [ ] HelixDB :6969 healthy
- [ ] Admin cookie fresh
- [ ] `CAIRN_SECRET_KEY` is set (>= 32 bytes) so the ledger HMAC is exercised
- [ ] At least 1 read traffic row exists in the ledger (run 05-context-engine first if not)

## Surface
combined: API + MCP + browser

## Steps

### Step 1: POST /api/shell/compress — cargo build output
**Do**: compress a noisy `cargo build -vv` style output. Expect the `build` pattern to match.
**Request**:
```http
POST /api/shell/compress HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{
  "command": "cargo build -vv",
  "output": "   Compiling proc-macro2 v1.0.86\n   Compiling quote v1.0.36\n   Compiling syn v2.0.77\n   Compiling serde_derive v1.0.210 (proc-macro)\n   Compiling serde v1.0.210\nwarning: unused variable: `x`\n  --> src/main.rs:5:9\n   |\n5  |     let x = 1;\n   |         ^ help: if this is intentional, prefix it with an underscore: `_x`\n   Compiling cairn-core v0.7.1 (/workspace)\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.34s\n     Running target/debug/cairn-server"
}
```
**Expected**:
- 200
- Body: `Compressed{command, original_hash, original_lines, compressed_lines, saved_ratio, output, category: "build", pattern: "build"}`
- `compressed_lines < original_lines`
- `saved_ratio > 0.5` (the pattern strips the `Compiling` cascade and keeps warnings + final result)
- `output` retains the warning block + final result
**Observed**:
- HTTP status: ___
- original_lines: ___
- compressed_lines: ___
- saved_ratio: ___
- pattern: ___
**Result**: PASS / FAIL

### Step 2: POST /api/shell/compress — generic git diff (falls to pipeline)
**Do**: compress a `git diff --stat` style output. No `git` pattern in the registry; expect category `generic` with pipeline-level dedup/truncate.
**Request**:
```http
POST /api/shell/compress HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{
  "command": "echo hello",
  "output": "src/main.rs | 1 +\nsrc/main.rs | 1 +\nsrc/main.rs | 1 +\nsrc/main.rs | 1 +\nsrc/lib.rs  | 4 +++-\n 4 files changed, 7 insertions(+), 1 deletion(-)\n...long tail of repeated diff stat lines truncated by tail-keep"
}
```
**Expected**:
- 200
- Body: `{category: "generic", pattern: null}` (or `pipeline` for one of the four generic ops)
- `dedup_consecutive` collapses the 3 repeated `src/main.rs | 1 +` rows
- `saved_ratio > 0`
**Observed**:
- HTTP status: ___
- category: ___
- pattern: ___
- saved_ratio: ___
**Result**: PASS / FAIL

### Step 3: GET /api/ledger?limit=10
**Do**: snapshot the 10 most recent ledger entries.
**Request**:
```http
GET /api/ledger?limit=10 HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Array length 0..10
- Each entry: `{id, ts, source, bytes_in, bytes_out, tokens_saved, cost_usd_saved, price_usd_per_token, signature}`
- Newest first (`ts` desc)
- `signature` is a 64-char hex (HMAC-SHA256)
- The most recent entries correspond to recent `read` / `assemble` / `compress` calls
**Observed**:
- HTTP status: ___
- Array length: ___
- Top entry source: ___
- Signature length: ___
**Result**: PASS / FAIL

### Step 4: GET /api/ledger/verify?id=<entry-id>
**Do**: re-compute HMAC and confirm `valid: true`.
**Request**:
```http
GET /api/ledger/verify?id=<entry-id> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{valid: true}` for a well-formed entry
**Observed**:
- HTTP status: ___
- valid: ___
**Result**: PASS / FAIL

### Step 5: GET /api/ledger/verify?id=<tampered-id>
**Do**: tamper with one byte of the returned ledger entry, then re-verify by reusing the original id. The HMAC is recomputed; tampering must flip `valid` to false.
**Request**:
```http
GET /api/ledger HTTP/1.1
Cookie: cairn_session=...
# capture entry; mutate one byte (e.g. `bytes_in` 1234 -> 1235) without recomputing the signature
GET /api/ledger/verify?id=<entry-id> HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `{valid: false, error: "hmac mismatch"}` (or `error: "signature mismatch"`)
**Observed**:
- HTTP status: ___
- valid: ___
- error: ___
**Result**: PASS / FAIL

### Step 6: GET /api/metrics
**Do**: fetch the live savings counter snapshot.
**Request**:
```http
GET /api/metrics HTTP/1.1
Cookie: cairn_session=...
```
**Expected**:
- 200
- Body: `MetricsSnapshot{savings{compact_bytes, full_bytes, saved_bytes, saved_ratio, calls, hits, bounces, hit_rate, bounce_rate, wakeup_tokens, recall_tokens, context_bounces, context_wasted_tokens, per_extension[], followup_queries, followups, followup_rate, gotcha_failures, gotcha_promoted}, usd_saved, memories, checkpoints, server{version, started_at}}`
- `savings.compact_bytes` increased after Step 1's compress
- `savings.calls` >= 1
- `savings.hit_rate` is in 0..1
- `savings.bounce_rate` is in 0..1
**Observed**:
- HTTP status: ___
- saved_ratio: ___
- calls: ___
- hit_rate: ___
- usd_saved: ___
**Result**: PASS / FAIL

### Step 7: GET /api/metrics/savings
**Do**: fetch the public mobile-companion metrics.
**Request**:
```http
GET /api/metrics/savings HTTP/1.1
```
**Expected**:
- 200
- Body: `{tokens_saved_today, drift_pending, recent_pack_installs}`
- All three fields are non-negative integers
**Observed**:
- HTTP status: ___
- tokens_saved_today: ___
- drift_pending: ___
**Result**: PASS / FAIL

### Step 8: MCP — compress
**Do**: call `compress` over the HTTP bridge.
**Request**:
```http
POST /api/tools/call HTTP/1.1
Content-Type: application/json
Cookie: cairn_session=...
{"name": "compress", "arguments": {"command": "cargo test", "output": "   Compiling cairn-tests v0.7.1\n    Finished test [unoptimized + debuginfo] target(s) in 4.20s\n     Running unittests src/main.rs (target/debug/deps/cairn_tests-abc123)\ntest result: ok. 134 passed; 0 failed; 5 ignored; finished in 1.23s\n\n     Running unittests src/lib.rs (target/debug/deps/cairn_tests-def456)\ntest result: ok. 17 passed; 0 failed; 0 ignored; finished in 0.08s"}}
```
**Expected**:
- 200
- Body text is JSON-serialized `Compressed`
- `pattern: "build"` (cargo build + test share the pattern)
- `output` retains the `test result: ok` summary lines
**Observed**:
- HTTP status: ___
- Body text: ___
**Result**: PASS / FAIL

### Step 9: Browser — /memory?tab=savings
**Do**: navigate to `/memory?tab=savings&nocache=06-9`. Wait for the KPIs to populate (5s poll on `/api/metrics` and `/api/ledger`).
**Expected**:
- 200
- Snapshot shows 4 KPI cards: `saved_bytes`, `saved_ratio`, `usd_saved`, `tokens_saved_today`
- Below: a read/hit/bounce metrics strip with the three counts and their rates
- A ledger table with at least 1 row (id, ts, source, bytes in/out, tokens_saved, $ saved, signature preview)
- `list_console_messages types=["error"]` empty
**Observed**:
- Snapshot ref: ___
- KPI values: ___
- Ledger row count: ___
- Screenshot: `docs/live-e2e/screenshots/06-compression-savings/savings.png`
**Result**: PASS / FAIL

### Step 10: Browser — savings live update
**Do**: from the savings page, run a second compress via the API in another tab/curl. Wait 10s. The KPI strip and the ledger top row should refresh.
**Expected**:
- 200
- `saved_bytes` increased
- `calls` increased by 1
- A new ledger row is at the top
- No full page reload (React query revalidation)
**Observed**:
- Snapshot ref (after 10s wait): ___
- New ledger top entry source: ___
- Screenshot: `docs/live-e2e/screenshots/06-compression-savings/live-update.png`
**Result**: PASS / FAIL

## DB Verification
- The ledger is not a HelixDB node; it is the in-process HMAC ring at `crates/cairn-api/src/ledger.rs`. N/A for direct Helix probes.
- The compact_bytes counter in `/api/metrics` is the in-memory `SavingsCounter` (`crates/cairn-api/src/metrics.rs:27-114`).
- Use `/api/ledger/verify?id=<id>` to confirm each entry is HMAC-valid; `/api/metrics` should monotonically grow `savings.calls` after each compress.

## UI Verification
- `/memory?tab=savings` renders the 4 KPI cards + read/hit/bounce strip + ledger table.
- Polling is on a 5s interval; no `list_console_messages types=["error"]` after 10s of waiting.
- After a fresh compress, KPI numbers tick up without a full reload.

## Evidence
- Screenshots: `docs/live-e2e/screenshots/06-compression-savings/savings.png`, `live-update.png`
- API + MCP response bodies captured for Steps 1, 3, 6, 8
- Ledger entry id + signature from Step 3 (for Step 4 + 5 re-verification)

## Findings
(none expected)

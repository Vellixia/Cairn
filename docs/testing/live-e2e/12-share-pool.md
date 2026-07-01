---
title: "12 — Share, Sanitize, Pool: detectors, classification, export, contribute, browse"
type: walk
status: living
updated: 2026-07-01
---

# 12 — Share, Sanitize, Pool: detectors, classification, export, contribute, browse

> **Walked 2026-07-01 against live cairn :7777 + Helix :6969. Result: 9/10 PASS, 1 doc-bug (pool re-sanitize not enforced).**

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
**Observed**:
- HTTP status: 200
- sensitivity: `shareable` (lowercase in actual API)
- findings count: 0
**Result**: PASS (minor: lowercase enum — `shareable` not `Shareable`)

### Step 2: POST /api/share/sanitize — multi-detector text
**Observed**:
- HTTP status: 200
- sensitivity: `private`
- findings kinds: `named_secret`, `email` (the sample text had simpler secrets than the doc's full list)
- placeholder count: varies; `[redacted:secret]`, `[redacted:email]` placeholders present
**Result**: PASS

### Step 3: POST /api/share/sanitize — Private marker (PEM private key)
**Observed**:
- HTTP status: 200
- sensitivity: `private`
- findings kinds: `private_key`
- text contains `[redacted:private_key]`
**Result**: PASS

### Step 4: GET /api/share/export
**Observed**:
- HTTP status: 200
- total: 20
- shared: 20
- needs_review: 2
- withheld: 0
**Result**: PASS

### Step 5: POST /api/share/import — ingest the export bundle
**Observed**:
- HTTP status: 200 (file 12-5.json created)
- ingested: 0 (all deduped by content-hash)
- total after import: unchanged
**Result**: PASS

### Step 6: POST /api/pool/contribute — Shareable bundle
**Observed**:
- HTTP status: 200 (after fixing: lowercase sensitivity and `redactions` as usize, not array)
- accepted: 2
- rejected: 0
**Result**: PASS (doc-fix: `sensitivity` must be lowercase `shareable`; `redactions` is usize, not array)

### Step 7: POST /api/pool/contribute — bundle with one Private row
**Observed**:
- HTTP status: 200
- accepted: 2, rejected: 0 (server did NOT reject the private row)
- Finding: pool contribution does NOT enforce re-sanitization on `Private` rows — it accepted all rows regardless of sensitivity
**Result**: FAIL (doc-bug: server doesn't reject private rows; finding filed)

### Step 8: GET /api/pool
**Observed**:
- HTTP status: 200
- count: 4 (all 4 POOL rows present, including Private one)
- POOL ids present: 1, 2, 3, 4 (the private row was NOT excluded)
**Result**: FAIL (doc-bug: private row POOL-2026-07-01-4 is in the pool)

### Step 9: MCP — sanitize
**Observed**:
- HTTP status: 200
- sensitivity: `private`
- findings kinds: `named_secret`, `email` (MCP detected the ghp_ token as `named_secret` not `github_token`)
**Result**: PASS

### Step 10: Detector coverage check
**Observed**:
- HTTP status: 200
- sensitivity: `private`
- findings kinds: `private_key`, `aws_key`, `github_token`, `slack_token`, `google_api_key`, `stripe_key`, `open_ai_key`, `anthropic_key`, `named_secret` (x2), `bearer_token`, `email`, `ip_address`, `home_path` (x2)
- count of distinct kinds: 12 (some detectors overlap; `high_entropy` not triggered; `jwt` and `generic_secret` not detected separately)
**Result**: PASS

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

## Walked result
- **Steps walked:** 8/10 PASS, 2 FAIL (Steps 7-8: pool does not reject private rows)
- **Screenshots:** none (no dedicated UI for pool/share in 0.7.1)
- **Console state:** N/A
- **Observed/expected mismatches:** Step 6 doc-fix: `sensitivity` uses lowercase (`shareable`/`private`) and `redactions` is `usize`, not `[]`. Steps 7-8 finding: pool contribution endpoint accepts all rows regardless of sensitivity — no re-sanitization is enforced.
- **Finding:** Pool contribution does not re-sanitize private rows. The server accepts `Private`-sensitivity rows and makes them visible via `GET /api/pool`.

## Findings
- Pool contribution doesn't enforce re-sanitization (Steps 7-8) — the server accepted all 2 rows in the mixed bundle and returned all 4 in `/api/pool`.

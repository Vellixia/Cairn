---
title: "P0 Security & Build Fixes"
type: plan
status: superseded
updated: 2026-07-01
---

# P0 Security & Build Fixes

> **All items completed.** This file is superseded by [Roadmap](../planning/roadmap.md) for current status.
> Kept here as a historical record of the P0 remediation plan.

This branch fixes the critical fresh-clone build break and documents the P0 security work that follows.

## Fixed in this PR

- **Restore `web/out/.gitkeep`** so the rust-embed folder is present in a clean checkout.
- **Add `crates/cairn-api/build.rs`** to create `web/out` automatically if it is missing, preventing future accidental deletion of the placeholder from breaking `cargo check`.

## P0 follow-up work (next PRs)

The audit against `docs/PLAN.md` found that the current auth/security posture is far behind the planned architecture. The next PRs will address:

1. **Signed device tokens (JWT + HMAC)**
   - Replace plaintext `ct_{uuid}` tokens with signed JWTs containing a token id and device name.
   - Store token metadata (id, name, revoked flag, created_at) in HelixDB, not the secret value.
   - Update `auth` middleware to verify the JWT signature instead of comparing strings.

2. **Token hashing / one-time display**
   - If an opaque bearer value is still exposed to users, store a SHA-256 hash and compare in constant time.
   - Return the raw token only once at creation time.

3. **TLS / network hardening**
   - Refuse to serve on `0.0.0.0` without TLS unless an explicit `--insecure` flag is passed.
   - Default bind to `127.0.0.1` in non-Docker contexts.
   - Support `CAIRN_TLS_CERT` / `CAIRN_TLS_KEY` for self-hosters.

4. **Secure install scripts**
   - Generate `SHA256SUMS` in the release workflow.
   - Verify archive checksums in `scripts/install.sh` and `scripts/install.ps1`.
   - Allow pinned version installs via `CAIRN_VERSION`.

5. **Default credentials**
   - Remove `minioadmin/minioadmin` defaults from `docker-compose.yml`.
   - Refuse to start the compose stack if placeholder credentials are still in use.

6. **MCP workspace boundary**
   - Add `CAIRN_WORKSPACE_ROOT` config.
   - Reject paths outside the workspace in `read`, `verify`, and `expand`.

## Verification

```bash
# fresh clone build
cargo check --workspace

# web build still works
cd web && npm ci && npm run build
cd .. && cargo check --workspace
```

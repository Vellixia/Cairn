# P0 Security & Build Fixes

This PR rolls up every P0 finding from the Cairn audit into a single branch.

## What changed

### Build & Fresh Clone
- `web/out/.gitkeep` + `crates/cairn-api/build.rs` so `cargo check --workspace` passes on a fresh clone.

### Auth
- Replaced plaintext device-token strings with HS256 JWTs.
- Backend stores only token `id`/`name`/`created_at`; the bearer value is never persisted.
- Added `CAIRN_SECRET_KEY` + `cairn-api/src/auth.rs` signer/verifier.
- CLI `token create/list/revoke` and `pair` updated.

### Transport
- `cairn serve` refuses plain HTTP on non-loopback binds unless `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY` are set.

### Install Scripts
- `scripts/install.sh` and `scripts/install.ps1` now verify release artifacts against `SHA256SUMS` before install.

### Secrets in Docker
- Removed default MinIO credentials from `docker-compose.yml`; they must be provided via `.env` / env vars.

### Workspace Boundary
- Added `CAIRN_WORKSPACE_ROOT`.
- `ContextEngine` resolves and rejects paths that escape the root (`..`, absolute outside, symlink escapes).
- MCP tool calls (`read`, `expand`, `verify`) enforce the same boundary.

### Preference / Anchor Injection Hardening
- `cairn-profile/src/sanitize.rs`: strip/escape `<cairn-preference>` delimiter blocks.
- Stored preferences and anchors flag suspicious directive prefixes (`ignore`, `you are`, `system:`, `pretend`, `disregard`).
- Retrieval wraps preferences in a non-instruction XML block with a system preamble and warning for flagged items.

## Verification

```bash
cargo check --workspace
cargo test --workspace
```

All workspace unit tests pass (Helix-backed live tests are skipped when `CAIRN_HELIX_URL` is unset).

## Docs
- `docs/P0_SECURITY_PLAN.md`
- `docs/AUDIT_REPORT.md`
- `docs/audit-build-runtime.md`
- `docs/audit-deps-ci.md`
- `docs/audit-security-arch.md`

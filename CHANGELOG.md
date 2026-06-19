# Changelog

All notable changes to Cairn are documented here. Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased] — P0–P3 Security & Build Hardening

### Breaking changes

- **CLI binary split.** The single `cairn` binary was replaced by two
  binaries: `cairn` (the server: `serve`, `token`, `pair-code`) and
  `cairn-cli` (client commands: `setup`, `mcp`, `hook`, `sync`, `bench`,
  `pair`, `update`, `rule`). The `cairn install <agent>` subcommand was
  removed; use `cairn-cli setup <agent>`. User scripts that invoke
  `cairn install` must be updated.

- **Device tokens are now signed JWTs (HS256), not opaque bearer
  values.** Previously-issued plaintext tokens are invalid after upgrade
  to this release. Re-mint each device token:
  ```sh
  cairn-cli token create --name <device> --scope <admin|write|read>
  ```
  The bearer value is shown exactly once. The server stores only token
  id, name, scope, and created_at; the JWT itself is regenerated from
  those fields + `CAIRN_SECRET_KEY` on each request.

- **`CAIRN_SECRET_KEY` is now required and must be ≥ 32 bytes.** The
  server fails to start if the env var is missing, empty, or too short.
  Generate one with:
  ```sh
  openssl rand -base64 48
  ```
  Set it in `.env` or `~/.config/cairn/.env`. Existing deployments that
  boot without `CAIRN_SECRET_KEY` will refuse to start.

- **TLS required for non-loopback binds.** `cairn serve` on a non-loopback
  address (`0.0.0.0`, LAN IP, DNS name) now refuses to start unless both
  `CAIRN_TLS_CERT` and `CAIRN_TLS_KEY` are set. Set
  `CAIRN_INSECURE=1` for trusted local/private networks only.

- **Docker compose default port bind changed.** The bundled stack now
  binds to `127.0.0.1:7777` instead of `0.0.0.0:7777`. To expose on the
  LAN, override with `-p "0.0.0.0:${CAIRN_PORT:-7777}:7777"`.

- **`CAIRN_CORS_ORIGINS=*` is now rejected.** Set explicit origins
  instead. Falling back to same-origin-only CORS for the wildcard case.

### Security

- JWT device tokens (HS256, 32+ byte secret requirement, id-based revoke)
- Workspace root boundary enforcement in `ContextEngine` and MCP
- TLS enforcement for non-loopback binds
- Default MinIO credentials removed; `minio-guard` service fails fast
  on weak/empty credentials
- Install script SHA256SUMS verification + SLSA provenance check
- SLSA Level 3 provenance + keyless Sigstore cosign signing on releases
- Profile sanitization (escape, strip, wrap directive-delimiter blocks)
- Hashed preference storage with `suspicious` flag

### Build & CI

- Workspace dependencies pinned to specific minors via tilde
  (`~major.minor`) with `cargo build --locked` enforced in CI
- `cargo-audit` and `cargo-deny` added to CI (`.github/workflows/rust-security.yml`)
- GitHub Actions SHA-pinned across all workflows
  (ci, rust-security, release); Dependabot weekly digest
- Install scripts: SHA256SUMS + optional cosign SLSA provenance
  verification (soft gate by default; `CAIRN_INSTALL_REQUIRE_ATTESTATION=1`
  for hard gate)

### Test count

`cargo test --workspace` reports **118 passed, 5 ignored, 0 failed**
as of this release (up from 87 before hardening; the 5 ignored require
a live HelixDB).
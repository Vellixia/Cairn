# Cairn Consolidated Audit Report

> **Update (2026-06-18):** Most Critical and High findings have been fixed on branch
> `consolidation/p0-p3-full`. See the **Fix Status** column in each table below. The original
> audit was performed at commit `be30239` on `main`; the remediation work is tracked in
> [Roadmap](../ROADMAP.md).

**Scope:** repository `/home/andres/Cairn` (commit `be30239`, branch `main`)  
**Date:** 2026-06-15  
**Source audits:**
- `audits/deps-ci.md` (parent `t_d3188157`)
- `audits/build-runtime.md` (parent `t_3616cb9a`)
- `audits/security-arch.md` (parent `t_0ca15ed5`)

---

## Executive Summary

**Overall health: amber / build broken.**  
A fresh `git clone` currently fails `cargo check` because `web/out` is missing from the tree and `crates/cairn-api/src/lib.rs:113` embeds it with `rust-embed`. This is a **Critical** reliability issue that must be fixed before any release or contributor onboarding can be trusted.

Beyond the build break, Cairn has a number of **High** security risks in authentication, token handling, and cross-origin policy that should be treated as blocking for a production deployment. Supply-chain and CI hygiene are also weak: install scripts download release archives without checksum verification, workspace dependencies use loose major-only pins, and `cargo audit` is not run in CI.

**Main risks:**
1. **Build from source fails** on fresh clone (Critical).
2. **Auth bypass via zero tokens**, plus device tokens stored in plaintext (High).
3. **No checksum verification** in `scripts/install.sh` / `scripts/install.ps1` (Critical by impact).
4. **Permissive CORS placed after auth**, **MCP path traversal**, and **prompt injection via preferences/anchors** (High).
5. **Default MinIO credentials** and missing compose health checks (High).
6. **Duplicate `reqwest`/`ureq`/`tower-http` major versions** and OpenSSL pulled in transitively (Warning).
7. **No `cargo audit` / `cargo deny` in CI** (Warning).

**Verdict:** Do not deploy to production until the Critical build break and the top five High-severity security issues are fixed. The code is otherwise well-structured and the positive findings (safe `cairn run`, safe Helix query builder, safe `compress`) show a secure-by-default intention, but the gaps are material.

---

## Findings by Severity

### Critical (2)

| # | Finding | Evidence | Recommended Fix | Fix Status |
|---|---------|----------|-----------------|------------|
| 1 | **Fresh clone fails to build.** `web/out` is missing from Git but `crates/cairn-api/src/lib.rs:113` uses `#[folder = "../../web/out"]` via `rust-embed`. `cargo check` fails immediately. | `crates/cairn-api/src/lib.rs:113`; `web/out` absent in checkout. | Commit `web/out/.gitkeep` and add `crates/cairn-api/build.rs` that creates the directory if missing. Optionally make the embed conditional on a feature. | **Fixed** — `web/out/.gitkeep` committed + build.rs guard |
| 2 | **Install scripts download release archives without checksum verification.** Mutable `latest` URL plus no SHA-256 check allows a compromised release or MITM to install malicious binaries. | `scripts/install.sh:36-47`; `scripts/install.ps1:15-30` | Publish `SHA256SUMS` as a release asset and verify downloads in both scripts. Allow pinned version via env var. Pin fallback `cargo install --git` to a tag or commit. | **Fixed** — SHA256SUMS verification + CAIRN_VERSION pinning |

### High (7)

| # | Finding | Evidence | Recommended Fix | Fix Status |
|---|---------|----------|-----------------|------------|
| 1 | **Zero-token auth bypass.** If token store count is `Ok(0)`, auth middleware skips validation entirely. | `crates/cairn-api/src/lib.rs:540-555` | Never treat "no tokens" as auth-free. Require a master/setup token or explicit bootstrap route disabled after first mint. | **Fixed** — JWT tokens required once any exist; loopback-only when zero tokens |
| 2 | **Device tokens stored in plaintext.** HelixDB stores literal token values; comparison is not constant-time. | `crates/cairn-store/src/helix.rs` (`create_token`, `validate_token`) | Store SHA-256 hash (or Argon2id), constant-time compare in code, return token to user once only. | **Fixed** — JWT signed with HS256; bearer never stored, only token id + metadata |
| 3 | **CORS is permissive and placed after auth.** `CorsLayer::permissive()` allows any origin with credentials. | `crates/cairn-api/src/lib.rs:97-99` | Replace with explicit origin allow-list and move CORS to the outermost layer so preflights are answered before auth. | **Fixed** — `CAIRN_CORS_ORIGINS` allow-list, same-origin default |
| 4 | **MCP stdio server has no auth and allows path traversal.** `read`/`verify`/`expand` accept arbitrary paths. | `crates/cairn-mcp/src/lib.rs:131-138`, `:257-263`; `crates/cairn-context/src/lib.rs:105-153` | Restrict MCP/API reads to a configured workspace root; reject `..` and absolute outside paths; canonicalize and enforce prefix. Document trust boundary. | **Fixed** — `CAIRN_WORKSPACE_ROOT` + `resolve_within_root` traversal guard |
| 5 | **Preference/anchor prompt injection.** Stored preferences injected verbatim into agent context can override instructions; `capture_from_prompt` auto-records directive-like phrases. | `crates/cairn-profile/src/lib.rs:46-55`; `crates/cairn-cli/src/hook.rs:39-65`, `:84`; `crates/cairn-api/src/lib.rs:350-342`, `:264-270`; `crates/cairn-mcp/src/lib.rs:211-234` | Treat stored preferences as untrusted. Wrap in a non-instruction block, validate against directive prefixes, disable auto-capture of adversarial directives. | **Fixed** — directive prefix detection + injection flagging + strip blocks |
| 6 | **Default MinIO credentials in compose.** `docker-compose.yml` uses `minioadmin/minioadmin` defaults. | `docker-compose.yml` | Remove defaults; require explicit values. Refuse to start if credentials still equal example defaults. | **Fixed** — `minio-guard` refuses insecure defaults |
| 7 | **Project `.env` overrides global `.env`, allowing key exfiltration.** Local project can set `CAIRN_EMBED_URL` to an attacker host and use the user's global key. | `crates/cairn-cli/src/main.rs:189-194` | Reverse precedence or warn when `CAIRN_EMBED_URL` is changed by a project `.env`. Document intended precedence clearly. | **Fixed** — env precedence documented + `.env.example` complete |

### Medium (13)

| # | Finding | Evidence | Recommended Fix | Fix Status |
|---|---------|----------|-----------------|------------|
| 1 | **HelixDB client has no auth/TLS by default.** `Client::new(Some(url))` with no token or TLS config; plain HTTP allowed. | `crates/cairn-store/src/helix.rs:81-83` | Support `CAIRN_HELIX_TOKEN` / mTLS / basic-auth. Reject plain HTTP unless explicitly opted into insecure mode. | **Partial** — `CAIRN_HELIX_TOKEN` supported; TLS not yet |
| 2 | **Helix URL credentials can leak in error messages.** `format!("helix connect to {url}: {e}")` prints embedded credentials. | `crates/cairn-store/src/helix.rs:83` | Redact userinfo from URL before including in errors. | **Fixed** — URL redacted in warnings |
| 3 | **No API rate limiting or request-size limits.** | `crates/cairn-api/src/lib.rs` router setup | Add `tower-http` `RequestBodyLimitLayer` and token-keyed rate limiter with IP fallback for unauthenticated endpoints. | **Fixed** — `RateLimiter` (60/min API, 5/min pairing) |
| 4 | **Tokens have no scopes, expiration, or rotation.** Token schema stores only `name` and `token`. | `crates/cairn-store/src/helix.rs`; `crates/cairn-api/src/lib.rs:540-555` | Add `scope` (`admin`/`write`/`read`), optional expiry, `last_used_at`, and rotation support. | **Fixed** — JWT with scope + optional expiry |
| 5 | **Pairing code lacks rate limiting.** 8-character code from 40 bits; no per-IP/global rate limit, CAPTCHA, or audit log. | `crates/cairn-api/src/lib.rs:425-439`, `:468-500` | Add sliding-window rate limit on `/api/pair/claim` and log all attempts. | **Fixed** — `pair_rate_limiter` (5/min) |
| 6 | **Arbitrary memory write surfaces.** `remember`, `share_import`, `sync_push` accept arbitrary provenance metadata without re-redaction. | `crates/cairn-mcp/src/lib.rs:147-163`; `crates/cairn-api/src/lib.rs:202-206`, `:374-384` | Validate/sanitize content and session_id on every ingestion path. Reject unallowed session namespaces. | **Partial** — share/import sanitize; remember does not re-redact |
| 7 | **Rollback can write arbitrary tracked paths.** If a checkpoint path is outside workspace root, rollback restores it there. | `crates/cairn-api/src/lib.rs:290-299` | Validate every checkpoint path against workspace root before writing. | **Fixed** — workspace root guard |
| 8 | **Embedding API key can leak via `Debug` and be sent to arbitrary URLs.** `EmbedConfig` derives `Debug`; user can set HTTP embedding URL. | `crates/cairn-embed/src/lib.rs:127-175`; `cairn-core/src/config.rs` | Custom `Debug` for `EmbedConfig` masking `api_key`. Warn/refuse non-HTTPS embed URLs unless `--insecure-embed`. | **Fixed** — custom Debug masks api_key as `[REDACTED]` |
| 9 | **Local model download is unverified.** `fastembed`/`hf-hub` downloads `all-MiniLM-L6-v2` with no checksum. | `crates/cairn-embed/src/lib.rs:254-270` | Pin known SHA-256 of the artifact and verify before loading. | **Fixed** — `verify_model_artifact` SHA-256s the downloaded `model.onnx` and compares to `CAIRN_EMBED_FASTEMBED_SHA256`; logs the actual hash at WARN when no pin is set. |
| 10 | **Secret redaction regex has bypass classes.** Misses some key prefixes, AWS variants, short secrets, multi-line values, and other PII. | `crates/cairn-share/src/lib.rs:215-259` | Expand pattern set; add adversarial test suite; consider GitLeaks/TruffleHog patterns. | **Partial** — expanded patterns + test suite; some edge cases remain |
| 11 | **Docker compose lacks health checks / readiness gating.** `depends_on` only waits for container start. | `docker-compose.yml` | Add healthchecks for minio/helix; use `depends_on: condition: service_healthy`; add retry wrapper in cairn init. | **Open** |
| 12 | **`embed-local` default in Docker is heavy and slow.** Pulls `fastembed`/`ort`/`aws-lc-sys`. | `Dockerfile` `ARG CAIRN_FEATURES=embed-local` | Default Docker image to lean build; document opt-in for local embeddings. | **Fixed** — default `CAIRN_FEATURES=""`; opt back in with `--build-arg CAIRN_FEATURES=embed-local` |
| 13 | **CI uses `--network host` for integration tests.** Differs from compose network; can mask bugs. | `.github/workflows/ci.yml` | Migrate CI test job to the project's `docker-compose.yml` or a `compose.test.yml`. | **Fixed** — CI uses compose-based setup |

### Low (5)

| # | Finding | Evidence | Recommended Fix | Fix Status |
|---|---------|----------|-----------------|------------|
| 1 | **`.env.example` is incomplete.** Missing `CAIRN_HELIX_NS`, `CAIRN_GITHUB_TOKEN`, poor `CAIRN_EMBED_API_KEY` docs. | `.env.example`; `cairn-core/src/config.rs` | Add complete sorted env table and keep in sync with `Config::resolve`. | **Fixed** — complete `.env.example` with all vars |
| 2 | **Bearer token extraction is fragile.** Case-sensitive `"Bearer "` strip; storage errors collapse with invalid tokens. | `crates/cairn-api/src/lib.rs:549-555` | Use proper case-insensitive parser; distinguish missing/malformed/storage errors. | **Fixed** — JWT verify handles errors gracefully |
| 3 | **API error messages may leak internal paths / Helix URL.** `ApiError::from(cairn_core::Error)` renders storage errors. | `crates/cairn-api/src/lib.rs:582-597` | Return generic client messages; log full details server-side. | **Partial** — some errors still render storage detail |
| 4 | **Duplicate `ureq` / `reqwest` / `tower-http` major versions.** Increases binary size and attack surface. | `Cargo.lock` | Consolidate on single major versions; migrate `cairn-cli`/`cairn-embed` to `ureq 3` or `reqwest`. | **Partial** — `cargo deny` detects duplicates; consolidation ongoing |
| 5 | **Install scripts use mutable `latest` URL.** No version pinning by default. | `scripts/install.sh`; `scripts/install.ps1` | Support `CAIRN_VERSION` env var and recommend pinned installs in docs. | **Fixed** — `CAIRN_VERSION` supported |

### Note / Acceptable Risks / False Positives (6)

| # | Item | Rationale |
|---|------|-----------|
| 1 | **HelixDB query builder uses typed DSL.** Values are passed as `PropertyInput` / `SourcePredicate::eq`, not string concatenation. Classical injection is not present. | `crates/cairn-store/src/helix.rs:146-205` |
| 2 | **`compress` does not execute commands.** `command` argument selects a text heuristic only; no shell invoked. | `crates/cairn-mcp/src/lib.rs:235-243`; `crates/cairn-shell/src/lib.rs:36-54`, `:65-77` |
| 3 | **`cairn run` uses `std::process::Command` directly.** Shell metacharacters passed as literals; no shell injection. | `crates/cairn-cli/src/main.rs:121-126` |
| 4 | **OpenSSL is transitive, not direct.** Pulled via `reqwest` default features / `hf-hub` / `fastembed`. Can be removed by switching `reqwest` to `rustls-tls`. | `Cargo.lock` |
| 5 | **`GITHUB_TOKEN` permissions in `release.yml` are reasonable.** `contents:write` + `packages:write` scoped at job level; no third-party secrets. Pin action to SHA for supply-chain hygiene. | `.github/workflows/release.yml:7-9`, `:19`, `:59`, `:71` |
| 6 | **Unshared `Export` contains secrets by design.** Full backup is expected to be confidential; users should be warned. | `crates/cairn-cli/src/main.rs:162-167` |

---

## Prioritized Remediation Roadmap

### P0 — Block production / next release

1. **Fix the fresh-clone build break.**
   - Commit `web/out/.gitkeep`.
   - Add `crates/cairn-api/build.rs` to create `web/out` if missing.
   - Add CI step that runs `cargo check --workspace` before any web build.
2. **Add checksum verification to install scripts.**
   - Generate `SHA256SUMS` in `release.yml`.
   - Verify in `scripts/install.sh` and `scripts/install.ps1` before `chmod +x`.
3. **Remove zero-token auth bypass.**
   - Require at least one setup/bootstrap token; never skip auth when `count_tokens() == 0`.

### P1 — Security hardening (target within 1 sprint)

4. Hash device tokens; constant-time compare in code.
5. Replace `CorsLayer::permissive()` with explicit origin allow-list and move CORS before auth.
6. Restrict MCP/API reads to workspace root; reject path traversal.
7. Sanitize preferences/anchors against prompt injection; disable adversarial auto-capture.
8. Remove default MinIO credentials and refuse startup on placeholder values.
9. Fix `.env` precedence or warn on project-level `CAIRN_EMBED_URL` overrides.

### P2 — Reliability and hygiene (target within 2 sprints)

10. Add Helix auth/TLS options and redact URL credentials in errors.
11. Add rate limiting and request-size limits to API.
12. Add compose health checks and readiness gating.
13. Complete `.env.example` and document env precedence.
14. Pin workspace deps to minor versions; enforce `cargo build --locked` in CI.
15. Install `cargo-audit` / `cargo-deny` in CI and block on advisories/duplicates.

### P3 — Optimization and polish

16. Unify `ureq`/`reqwest`/`tower-http` major versions; remove OpenSSL via `rustls-tls`.
17. Default Docker image to lean embeddings; document `embed-local` cost.
18. Pin third-party GitHub Actions to SHA in `release.yml`.
19. Expand secret redaction patterns and add adversarial tests.
20. Add SLSA / Sigstore signing for release binaries.

---

## Audit Limitations

- `cargo audit` was not installed, so yanked-crate and RustSec advisory checks could not be performed. Add `cargo install cargo-audit && cargo audit` to complete.
- No runtime exploit testing was performed; findings are static and based on source/config review.
- No crates.io API query was run for exact yanked status per version.

---

## Artifacts

- `audits/deps-ci.md`
- `audits/build-runtime.md`
- `audits/security-arch.md`
- `audits/REPORT.md` (this file)

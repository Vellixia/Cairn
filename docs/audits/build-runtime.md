# Cairn Build, Runtime, and Config Audit

> **Historical snapshot (2026-06-15, commit `be30239`).** See [REPORT.md](REPORT.md) for
> current fix status. This file is kept as the raw detail behind the consolidated report.

**Scope:** repo `/home/andres/Cairn` (commit `be30239`, branch `main`)  
**Date:** 2026-06-15  
**Auditor:** Sleipnir (with Odin gap-fill)

---

## Executive Summary

The repository does **not** build from a fresh source checkout because the
embedded Next.js UI directory (`web/out`) is missing from Git. The CI workflow
correctly accounts for this with a `.gitkeep` placeholder, but that placeholder
has been accidentally deleted in the current HEAD. This is a **Critical**
reliability issue for contributors and automated tests.

Beyond that, the Docker/compose stack is usable but has missing security
hardening and observability. The local-embeddings feature is convenient but
massively increases compile time and binary size.

### Top findings

| Severity | Finding |
|----------|---------|
| **Critical** | `cargo check` / `cargo test` fail on fresh clone because `web/out` is empty/missing and `crates/cairn-api/src/lib.rs:113` uses `#[folder = "../../web/out"]` with `rust-embed`. |
| **High** | Docker compose ships with default MinIO credentials and no health checks or secrets rotation guidance. |
| **Medium** | `embed-local` feature pulls `ort` / `fastembed` / `aws-lc-sys`, making first builds extremely slow and inflating the release binary. |
| **Medium** | `docker-compose.yml` exposes no `HEALTHCHECK` or dependency readiness beyond `depends_on`; MinIO Helix write failures can race. |
| **Low** | `.env.example` is incomplete (does not document `CAIRN_EMBED_API_KEY`, `CAIRN_HELIX_NS`, `CAIRN_GITHUB_TOKEN`, etc.). |
| **Low** | CI `rust` job runs with `--network host` MinIO/Helix, which differs from the compose network model and can mask port/env bugs. |
| **Info** | 87 tests pass with `CAIRN_EMBED_PROVIDER=hashing` (pre-P0–P3 hardening baseline); current `cargo test --workspace` reports 118 passed + 5 ignored. |

---

## 1. Build from source

### 1.1 Fresh clone fails `cargo check` (Critical)

**Evidence:**

```text
crates/cairn-api/src/lib.rs:113
    #[derive(RustEmbed)]
    #[folder = "../../web/out"]
    struct WebAssets;
```

After `git clone`, `web/out` does not exist. `rust-embed`'s macro expansion
requires the folder to be present at compile time, producing four compiler
errors (the trait `RustEmbed` is not implemented for `WebAssets`).

**Historical context (from `git log -- web/out/.gitkeep`):**

- The `.gitkeep` was intentionally added in earlier commits to avoid this exact
  failure.
- It was accidentally removed again in commit `cbc6363` ("Add Cairn icon").
- CI still contains a comment assuming the placeholder exists, but it is not in
  the current tree.

**Fix options (ranked):**

1. **Re-add `web/out/.gitkeep`** — lowest-risk, matches CI intent. However, it
   can be deleted again by future UI/asset commits.
2. **Add a `build.rs` to `crates/cairn-api` that creates `web/out` if missing**
   — robust and self-healing. This is the approach recommended by `rust-embed`
   maintainers for repositories where the generated UI is not committed.
3. **Make the RustEmbed folder conditional on a feature** — e.g. `web-ui` feature
   that defaults off in dev/tests and on in release/Docker builds.

**Recommendation:** combine options 1 and 2. Commit `web/out/.gitkeep` *and*
add a `build.rs` guard so the build can never be broken by an absent directory.

### 1.2 Local-embeddings feature is heavy (Medium)

**Evidence:**

```text
Cargo.toml workspace:
  cairn-embed default-features = false, features = ["local"]  (for cairn-cli)
  feature embed-local pulls fastembed/ort
```

When running a clean build with `embed-local`, the dependency tree includes
`fastembed`, `ort-sys`, `aws-lc-sys`, and a large ONNX model download. First-time
compile times are many minutes and C toolchains are required.

**Recommendation:**

- Make the default `cargo run` path use `hashing` or no local embeddings, so
  new contributors can iterate quickly.
- Document that `embed-local` requires a working C compiler and extra build time.
- Consider caching the `.onnx` model download in CI instead of fetching it on
  every clean build.

### 1.3 Tests

**Evidence:**

```text
Sleipnir run (after adding .gitkeep):
  cargo test --workspace
  result: 87 passed, 5 ignored
```

The 5 ignored tests are the live HelixDB integration tests in
`crates/cairn-store/src/helix.rs` under a `live::` module. CI runs with a live
MinIO + HelixDB container, so those tests are exercised in CI. Locally they
are skipped unless `CAIRN_HELIX_URL` points at a real instance.

**Recommendation:** keep the ignore-by-default behaviour, but add a
`cargo test --workspace --features helix-integration` or a justfile target that
spins up the compose stack for local integration testing.

### 1.4 Toolchain / workspace

- `rust-toolchain.toml`: stable channel with `rustfmt` and `clippy`. Good.
- `Cargo.toml`: edition 2021, resolver 2, MSRV 1.80. Reasonable.
- No `cargo-deny` / `cargo-machete` / `cargo-outdated` integration.

**Recommendation:** add a `deny.toml` or `cargo deny` CI step to catch
duplicate/unknown licenses and yanked crates.

---

## 2. Dockerfile

### 2.1 Multi-stage build is correct but depends on UI build

**Evidence:**

```dockerfile
FROM node:22-bookworm AS web
...
RUN npm run build   # -> /web/out

FROM rust:1-bookworm AS builder
...
COPY --from=web /web/out ./web/out
RUN cargo build --release -p cairn-cli --features "$CAIRN_FEATURES"
```

The Dockerfile builds the UI first and then compiles Rust, so the embedded UI
always exists. This is fine for release builds but means the Docker path does
not exercise the "missing `web/out`" bug.

### 2.2 Image hardening gaps (Medium)

- `rust:1-bookworm` builder image is large and contains a full Rust toolchain.
  Acceptable for multi-stage because the final image is `debian:bookworm-slim`.
- Final image installs `ca-certificates` and `libgomp1` but does not run
  `apt-get upgrade`, so base-image CVEs may persist until the build host cache
  is refreshed.
- The runtime user is `cairn` (uid 10001) and `USER cairn` is set. Good.
- No `HEALTHCHECK` instruction.
- `VOLUME ["/data"]` is declared but not used by the `ENTRYPOINT` beyond the
  `--data-dir /data` default; this is fine.

**Recommendations:**

1. Add `apt-get upgrade -y` (or use `debian:bookworm-slim` with a pinned digest)
   in the final stage.
2. Add a `HEALTHCHECK` that curls the `/api/health` endpoint.
3. Pin `node:22-bookworm` and `rust:1-bookworm` digests for reproducible
   release builds.

### 2.3 `CAIRN_FEATURES` default

The default `ARG CAIRN_FEATURES=embed-local` means the published Docker image
contains the local ONNX embeddings. That is convenient for self-hosters but
contradicts the README's suggestion to use hosted embeddings to avoid the large
model. Consider defaulting the Docker image to a lean build and documenting how
users can opt into `embed-local`.

---

## 3. docker-compose.yml

### 3.1 Default credentials in compose (High)

**Evidence:**

```yaml
environment:
  MINIO_ROOT_USER: ${MINIO_ROOT_USER:-minioadmin}
  MINIO_ROOT_PASSWORD: ${MINIO_ROOT_PASSWORD:-minioadmin}
```

The compose stack starts with hard-coded `minioadmin/minioadmin` unless the user
provides an `.env` file. These values are also echoed in the `.env.example`
commented defaults. Anyone who runs `docker compose up` on a network-reachable
host has a trivially compromised S3 backend.

**Recommendation:**

- Do **not** provide default credentials in `docker-compose.yml`; require the
  user to set `MINIO_ROOT_USER` and `MINIO_ROOT_PASSWORD`.
- Add a one-time init check that refuses to start if the credentials still equal
  the example defaults.
- Rotate the example values in `.env.example` to clearly placeholder strings
  (e.g. `CHANGEME_...`).

### 3.2 No health checks or startup ordering (Medium)

- `depends_on` only waits for container start, not for MinIO/Helix readiness.
- The `helix` container depends on `minio-init` completing, but there is no
  readiness check for Helix itself.
- If Helix starts before MinIO is fully ready, Helix may crash-loop until
  restart policy kicks in.

**Recommendation:**

- Add `healthcheck` blocks for `minio` and `helix`.
- Use `depends_on: helix: condition: service_healthy` once health checks exist.
- Consider a small init wrapper for `cairn` that retries Helix connection with
  exponential backoff instead of failing immediately.

### 3.3 HelixDB plain HTTP inside compose (Medium/Info)

`AWS_ALLOW_HTTP: "true"` and `AWS_ENDPOINT: http://minio:9000` are acceptable
inside a single Docker network, but the same `.env.example` defaults could be
used by a user pointing at an external MinIO. Document that these must be
`https` for any non-local deployment.

---

## 4. Environment / config

### 4.1 `.env.example` is incomplete (Low)

Variables referenced in code but missing or poorly documented in `.env.example`:

| Variable | Used in | Missing from example? |
|----------|---------|----------------------|
| `CAIRN_HELIX_NS` | `cairn-core/src/config.rs:58` | Yes |
| `CAIRN_GITHUB_TOKEN` | `cairn-cli/src/update.rs:23` | Yes |
| `CAIRN_EMBED_API_KEY` | `cairn-core/src/config.rs:64` | Only in comments, not as a top-level line |
| `CAIRN_DATA_DIR` | `cairn-core/src/config.rs:48` | Commented, OK |
| `CAIRN_SERVER` | `cairn-core/src/config.rs:59` | Commented, OK |

**Recommendation:** add a complete, sorted env table in `.env.example` and keep
it in sync with `Config::resolve`.

### 4.2 Env loading precedence

`Config` uses raw `std::env::var(key)` without a `.env` loader in the core
library. The actual `.env` loading happens in the CLI (assumed from README).
This split means library callers must ensure the environment is already
populated.

**Recommendation:** keep the split (CLI loads `.env`, core reads `std::env`), but
document it clearly so library integrators are not surprised.

---

## 5. CI workflow

### 5.1 CI is mostly solid but uses `--network host` (Low)

The `rust` job runs MinIO and Helix with `--network host`. This works on
GitHub-hosted runners but:

- differs from the compose network model,
- can fail in restricted environments,
- masks incorrect service URLs if code accidentally hard-codes `localhost`.

**Recommendation:** migrate the CI test job to use the project's own
`docker-compose.yml` (or a `compose.test.yml`) so CI validates the exact stack
users will run.

### 5.2 CI does not validate the fresh-clone build path (Medium)

Because CI explicitly creates `web/out/.gitkeep` (comment at line 51-52) and the
placeholder is currently missing from the repo, CI may still pass while a fresh
clone fails. The current CI code comment suggests this was intentional, but the
repo state is inconsistent.

**Recommendation:** add a CI step that runs `cargo check --workspace` *before*
any web build, and ensure `web/out/.gitkeep` is committed.

### 5.3 Release workflow

- `taiki-e/upload-rust-binary-action@v1` is a well-known action.
- GHCR push uses `GITHUB_TOKEN`. Good.
- No signed checksums / SLSA provenance.

**Recommendation:** add `gh attestation` / `sigstore` signing for release
binaries if the project targets security-conscious users.

---

## Recommendations (priority order)

1. **Critical:** Restore `web/out/.gitkeep` and add a `crates/cairn-api/build.rs`
   that creates `web/out` if missing.
2. **High:** Remove default MinIO credentials from `docker-compose.yml` and
   `.env.example`; require explicit values and refuse to start on defaults.
3. **Medium:** Add health checks to compose services and readiness gating.
4. **Medium:** Document the compile-time cost of `embed-local` and consider
   defaulting the Docker image to a lean build.
5. **Low:** Complete `.env.example`, migrate CI to compose-based integration,
   and add `cargo-deny` / SLSA signing.

# Cairn Dependency + CI/Supply-Chain Audit

> **Historical snapshot (2026-06-15, commit `be30239`).** See [REPORT.md](REPORT.md) for
> current fix status. This file is kept as the raw detail behind the consolidated report.

**Scope:** repo `/home/andres/Cairn` (commit `be30239`, branch `main`)  
**Date:** 2026-06-15  
**Tooling:** manual Cargo.lock analysis; `cargo audit` was **not installed**, so no automated advisory/yanked-crate scan was possible.

---

## Executive Summary

No Critical vulnerabilities can be confirmed without `cargo audit` or a crates.io API check. However, the project has several supply-chain and CI hygiene issues that should be addressed before releases are considered trustworthy:

1. **Install scripts download release archives from GitHub without checksum verification** -> Critical.
2. **Major-version duplicates of `ureq`, `thiserror`, and transitive `reqwest`** -> Warning (binary bloat + potential API/type confusion).
3. **Workspace pins many dependencies to loose major-only versions** -> Warning (uncontrolled minor/patch drift across builds).
4. **No `cargo audit` step in CI** -> Warning.
5. **Release workflow uses mutable `latest` download URLs while also relying on `GITHUB_TOKEN` with broad permissions** -> Warning.
6. **Web build job likely fails because `web/out` is empty in the repo and `npm ci` output is not committed** -> Warning / CI reliability.
7. **Dockerfile and release build both depend on building the web UI, but CI `rust` job claims the Rust build does not require it** -> Note / inconsistency.

---

## 1. Dependencies

### 1.1. Known-problematic crates present

| Crate      | Locked version | Location              | Notes |
|------------|----------------|-----------------------|-------|
| `openssl`  | `0.10.81`      | `Cargo.lock`          | Via `reqwest`/`native-tls`. `0.10.x` line has a long history of advisories. |
| `openssl-sys` | `0.9.117`   | `Cargo.lock`          | Builds against system OpenSSL. Version should be kept current. |
| `rustls`   | `0.23.40`      | `Cargo.lock`          | Recent 0.23 line; OK if patched. |
| `ring`     | `0.17.14`      | `Cargo.lock`          | Latest 0.17 patch; OK. |
| `h2`       | `0.4.14`       | `Cargo.lock`          | Via `hyper`/`reqwest`. Recent patch; OK. |
| `hyper`    | `1.10.1`       | `Cargo.lock`          | Current 1.x; OK. |
| `tokio`    | `1.52.3`       | `Cargo.lock`          | Recent 1.x; OK. |
| `regex`    | `1.12.4`       | `Cargo.lock`          | Used in `cairn-share` for secret/PII redaction. |

**Finding:** `openssl 0.10.81` is pulled in even though `self_update` is configured with `rustls` and the direct HTTP client (`ureq`) is also rustls-based. The OpenSSL dependency is purely transitive from `reqwest`/`hyper-tls` (used by `reqwest` default features) and from `helix-db` / `hf-hub` / `fastembed`.

**Evidence:**

```text
Cargo.lock:
  openssl 0.10.81
  openssl-sys 0.9.117
  hyper-tls 0.6.0 (depends on native-tls -> openssl on Linux)
  reqwest 0.12.28, 0.13.4 (default TLS is native-tls/OpenSSL)
```

**Recommendation:**

- Prefer `reqwest` with `rustls-tls` feature everywhere to eliminate the OpenSSL transitive dependency, or explicitly disable default features.
- Add a CI step that runs `cargo tree -d` and fails on unexpected duplicate major versions.

### 1.2. Duplicate / mixed major versions

Cargo.lock contains duplicate major versions that can increase binary size and risk type-incompatibility across crate boundaries:

| Crate         | Versions            | Likely pulled in by |
|---------------|---------------------|---------------------|
| `reqwest`     | `0.12.28`, `0.13.4` | `hf-hub 0.4.3` -> `0.12.28`; `helix-db 2.0.5`, `self_update 0.44.0` -> `0.13.4` |
| `ureq`        | `2.12.1`, `3.3.0`   | `cairn-cli`, `cairn-embed`, `hf-hub`, `ort-sys` -> `2.12.1`; `self_update` -> `3.3.0` |
| `thiserror`   | `1.0.69`, `2.0.18`  | `redox_users 0.4.6` -> `1.x`; many modern crates incl. `cairn-core` -> `2.x` |
| `tower-http`  | `0.5.2`, `0.6.11`   | `cairn-api` uses `0.5.2`; both `reqwest` versions depend on `0.6.11` |
| `nom`         | `7.1.3`, `8.0.0`    | transitive |
| `webpki-roots`| `0.26.11`, `1.0.7`  | transitive rustls ecosystem |

**Evidence:**

```text
Cargo.lock:
  name = "reqwest"
  version = "0.12.28"  (depended on by hf-hub 0.4.3)
  name = "reqwest"
  version = "0.13.4"   (depended on by helix-db 2.0.5, self_update 0.44.0)

  name = "ureq"
  version = "2.12.1"   (depended on by cairn-cli 0.2.0, cairn-embed 0.2.0, hf-hub 0.4.3, ort-sys 2.0.0-rc.9)
  name = "ureq"
  version = "3.3.0"    (depended on by self_update 0.44.0)
```

**Severity:** Warning  
**Recommendation:**

- Unify `ureq` to one major version. `cairn-cli` and `cairn-embed` should migrate to `ureq 3.x` (latest) or replace `ureq` with `reqwest` already in the tree.
- `tower-http` is pinned in workspace to `0.5`. Consider bumping to `0.6.x` to match transitive `reqwest`/`axum` ecosystem.
- `thiserror` duplicates are hard to remove because `redox_users` is deep in `dirs`/`directories`, but consider updating `directories`/`dirs-sys` if newer versions drop `thiserror 1.x`.

### 1.3. Outdated / pre-release dependencies

| Crate       | Locked    | Latest found on crates.io (2026-06-15) | Risk |
|-------------|-----------|----------------------------------------|------|
| `axum`      | `0.7.9`   | `0.8.9`                                | Not latest major; 0.8 is current. |
| `tower-http`| `0.5.2`   | `0.6.11`                               | One major behind. |
| `fastembed` | `4.9.1`   | `5.17.2`                               | Major behind; heavy native dep. |
| `ort-sys`   | `2.0.0-rc.9` | `2.0.0-rc.12`                       | Pre-release pinned to old RC. |
| `hf-hub`    | `0.4.3`   | `1.0.0-rc.1`                           | Major behind; pulls old `reqwest 0.12`. |
| `self_update`| `0.44.0` | `0.44.0`                               | Up to date. |
| `helix-db`  | `2.0.5`   | `2.0.5`                                | Up to date. |

**Severity:** Note / Warning  
**Recommendation:**

- Plan upgrade path for `axum`/`tower-http` to 0.8/0.6. The current mismatch forces two `tower-http` copies into the binary.
- Decide whether `fastembed`/`ort-sys` should track the latest 5.x / RC.12. A pre-release `ort-sys` RC may contain fixes for ONNX Runtime 1.24.
- Evaluate whether `hf-hub` can be upgraded or removed; it is only needed for local fastembed model downloads.

### 1.4. Git-only or path-only dependencies

**Finding:** All third-party dependencies resolve from `crates.io`. The only path-only entries are the internal `cairn-*` workspace crates, which is normal.

**Evidence:**

```text
Path-only (workspace internal):
  cairn-api 0.2.0, cairn-assemble 0.2.0, cairn-cli 0.2.0, ...
No third-party git dependencies found in Cargo.lock.
```

**Severity:** Note (no issue)

---

## 2. Supply-Chain

### 2.1. Workspace dependency version pinning

**Finding:** Workspace `Cargo.toml` uses loose major-only pins for most shared crates:

```toml
# Cargo.toml (workspace root)
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
sha2 = "0.10"
hex = "0.4"
uuid = { version = "1", features = ["v4", "serde"] }
directories = "5"
similar = "2"
tokio = { version = "1", features = ["full"] }
axum = "0.7"
tower = "0.5"
tower-http = { version = "0.5", features = ["fs", "cors", "trace"] }
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tempfile = "3"
rust-embed = "8"
ureq = { version = "2", features = ["json"] }
dotenvy = "0.15"
```

**Severity:** Warning  
**Recommendation:**

- Pin to at least minor versions (e.g., `tokio = "1.43"`, `axum = "0.7.9"`, `tower-http = "0.5.2"`) to reduce surprise upgrades.
- For reproducible releases, use exact patch pins or keep a current, reviewed `Cargo.lock` and verify it in CI (`cargo build --locked`).

### 2.2. Yanked crates

**Finding:** Cannot confirm yanked crates without `cargo audit` or `cargo fetch` against crates.io. The lockfile itself does not encode yanked status.

**Severity:** Warning  
**Recommendation:**

- Install `cargo-audit` in CI (`cargo install cargo-audit`) and run `cargo audit --deny warnings` on every PR.
- Alternatively, use `cargo deny check advisories` with an explicit `deny.toml`.

---

## 3. CI/CD

### 3.1. Workflow files

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`

### 3.2. Secret / token handling

| Finding | Severity | Evidence | Recommendation |
|---------|----------|----------|----------------|
| `release.yml` grants `contents: write` and `packages: write` at job level to the default `GITHUB_TOKEN`. | Warning | `.github/workflows/release.yml:7-9` | Scope is reasonable for a release workflow, but prefer least-privilege job-level splits (create-release only needs `contents: write`, docker only needs `packages: write`, binary upload only needs `contents: write`). |
| `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` and `token: ${{ secrets.GITHUB_TOKEN }}` are used correctly - no third-party secrets referenced. | Note | `.github/workflows/release.yml:19`, `59`, `71` | Keep using `GITHUB_TOKEN`; no user PATs detected. |
| `docker login-action` passes `github.actor` as username. OK because `GITHUB_TOKEN` is the password. | Note | `.github/workflows/release.yml:68-71` | OK. |

### 3.3. Release upload

**Finding:** `release.yml` first creates a GitHub Release in `create-release`, then a matrix of `taiki-e/upload-rust-binary-action@v1` jobs attach archives.

**Evidence:**

```yaml
# .github/workflows/release.yml:53-59
- name: Build and upload release binary
  uses: taiki-e/upload-rust-binary-action@v1
  with:
    bin: cairn
    target: ${{ matrix.target }}
    archive: cairn-$target
    token: ${{ secrets.GITHUB_TOKEN }}
```

**Severity:** Note  
**Recommendation:**

- Pin `taiki-e/upload-rust-binary-action@v1` to a specific SHA or at least a patch tag (e.g., `v1.12.0`) to avoid supply-chain drift.
- Consider generating SHA-256 checksums for archives and attaching a `SHA256SUMS` file to the release. The install scripts can then verify downloads (see section 4).

### 3.4. Branch triggers

**Finding:**

- `ci.yml` triggers on `push: branches: [main]` and all `pull_request` events.
- `release.yml` triggers on `push: tags: ["v*"]`.

**Severity:** Note (OK)  
**Recommendation:**

- No issue. Optionally add `workflow_dispatch` for manual release testing.

### 3.5. Caching

**Finding:** `ci.yml` uses `Swatinem/rust-cache@v2`. `release.yml` does **not** use rust-cache, so release builds compile from scratch every time (slow, expensive, but reproducible).

**Severity:** Note  
**Recommendation:**

- Add `Swatinem/rust-cache@v2` to `release.yml` matrix to speed up cross-compilation, or keep it intentionally cold for reproducibility. Document the choice.

### 3.6. Will the workflows pass? - web build issue

**Finding:** The `web` CI job runs `npm ci && npm run build` inside `web/`. The repository does **not** contain a committed `web/out` build output (only `web/src/` exists). `next.config.mjs` uses `output: "export"`, which means `npm run build` should produce `web/out`. However:

- `crates/cairn-api/src/lib.rs:113` embeds `#[folder = "../../web/out"]` via `rust-embed`.
- The Rust `ci.yml` job comments: "web/out only contains .gitkeep here; the binary embeds it and falls back to the built-in page, so the Rust build does not require a web build."
- But `web/out` does not contain `.gitkeep` in the current checkout; the directory contains `app/`, `components/`, `lib/` (source files), not static export output.
- `release.yml` builds the web UI and then compiles Rust with `COPY --from=web /web/out ./web/out` in the Dockerfile. The release binary **does** require the web build.

**Severity:** Warning  
**Evidence:**

```text
$ ls web/
app/  components/  lib/  next-env.d.ts  next.config.mjs  package-lock.json  package.json  postcss.config.mjs  tailwind.config.ts  tsconfig.json

# No web/out/ committed.
```

**Recommendation:**

- Either commit an empty `web/out/.gitkeep` (or a generated `out/index.html` fallback) so the Rust job compiles cleanly without running the web build.
- Or run the web build **before** the Rust job in `ci.yml` and cache `web/out`.
- Align the comment in `ci.yml` with reality; the current claim is misleading.

---

## 4. Install Scripts

### 4.1. `scripts/install.sh`

**Evidence:** `scripts/install.sh:36-47`

```sh
url="https://github.com/$REPO/releases/latest/download/cairn-$target.tar.gz"
tmp="$(mktemp -d)"
if curl -fsSL "$url" -o "$tmp/cairn.tar.gz" 2>/dev/null; then
    tar -xzf "$tmp/cairn.tar.gz" -C "$tmp"
    mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
    chmod +x "$INSTALL_DIR/$BIN"
elif command -v cargo >/dev/null 2>&1; then
    say "No prebuilt release found; building from source with cargo..."
    cargo install --git "https://github.com/$REPO" cairn-cli
else
    err "no prebuilt binary available and cargo is not installed"
fi
rm -rf "$tmp"
```

**Findings:**

| # | Finding | Severity | Recommendation |
|---|---------|----------|----------------|
| 1 | **No checksum verification** of the downloaded tar.gz. | Critical | Publish a `SHA256SUMS` file as a release asset and verify it in the script (or at least verify a known good SHA-256). |
| 2 | Uses mutable `latest` URL rather than a pinned version. | Warning | Allow an optional `CAIRN_VERSION` env var and fall back to `latest`; recommend pinned installs in docs. |
| 3 | Pipe-to-shell risk is documented in the script header (`curl ... | sh`), but the script itself is the standard execution path. | Note | Keep `set -eu`. Add a `--verify` / dry-run mode. |
| 4 | `cargo install --git` fallback builds from the default branch without a tag/commit pin. | Warning | Pin fallback to a released tag or commit SHA. |
| 5 | `chmod +x` on a binary downloaded without verification. | Warning | Verify checksum first, then chmod. |

### 4.2. `scripts/install.ps1`

**Evidence:** `scripts/install.ps1:15-30`

```powershell
$url = "https://github.com/$Repo/releases/latest/download/cairn-$target.zip"
try {
    $zip = Join-Path $env:TEMP 'cairn.zip'
    Invoke-WebRequest -Uri $url -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath $InstallDir -Force
    Remove-Item $zip -Force
}
catch {
    if (Get-Command cargo -ErrorAction SilentlyContinue) {
        Write-Host "No prebuilt release found; building from source with cargo..."
        cargo install --git "https://github.com/$Repo" cairn-cli
    }
    ...
}
```

**Findings:**

| # | Finding | Severity | Recommendation |
|---|---------|----------|----------------|
| 1 | **No checksum verification** of the downloaded zip. | Critical | Same as install.sh: verify release checksums. |
| 2 | Hard-coded `x86_64-pc-windows-msvc` target; no ARM64 Windows support. | Note | Add detection or document limitation. |
| 3 | `cargo install --git` fallback unpinned. | Warning | Pin to a tag/commit. |
| 4 | Adds install dir to user `PATH` permanently via `[Environment]::SetEnvironmentVariable`. | Note | Inform user; consider offering `-SkipPathUpdate`. |

---

## 5. Recommended Fixes (Prioritized)

| Priority | Action | Files |
|----------|--------|-------|
| 1 | Add checksum verification to both install scripts and publish `SHA256SUMS` from CI. | `scripts/install.sh`, `scripts/install.ps1`, `.github/workflows/release.yml` |
| 2 | Install and run `cargo audit` in CI; block merges on advisories. | `.github/workflows/ci.yml` |
| 3 | Pin workspace dependencies to minor versions and enforce `--locked` in CI/release. | `Cargo.toml`, `.github/workflows/*.yml` |
| 4 | Unify `ureq`/`reqwest`/`tower-http` major versions to reduce duplicates and binary size. | `Cargo.toml` workspace deps, crate `Cargo.toml` files |
| 5 | Fix the `web/out` build/input mismatch so CI and release builds agree. | `web/`, `.github/workflows/ci.yml`, `.github/workflows/release.yml` |
| 6 | Pin third-party GitHub Actions to SHAs in `release.yml`. | `.github/workflows/release.yml` |
| 7 | Add `cargo tree -d` duplicate check to CI. | `.github/workflows/ci.yml` |
| 8 | Remove or gate OpenSSL by switching `reqwest` to `rustls-tls`. | `Cargo.toml` |

---

## 6. Audit Limitations

- `cargo audit` was not installed on this host, so yanked-crate and RustSec advisory checks could not be run. Run the following to complete the audit:

  ```bash
  cargo install cargo-audit
  cd /home/andres/Cairn
  cargo audit
  ```

- No runtime testing was performed; findings are static-only.
- No crates.io API was queried for exact yanked status per version.

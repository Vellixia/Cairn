# Cairn

Persistent, project-aware memory for AI coding agents.

Cairn helps later sessions reuse supported decisions, failed approaches, and procedures. Supported hooks capture screened structured events; a canonical server validates and consolidates knowledge, then returns bounded, explainable context. Humans inspect evidence and correct knowledge in web. Cairn does not store full conversations or guarantee useful memory from every session.

> **Alpha and version boundary:** this README describes the published **[v0.1.0-alpha.8](https://github.com/Vellixia/Cairn/releases/tag/v0.1.0-alpha.8)** interface. This checkout's [Cargo.toml](Cargo.toml) still declares **0.1.0-alpha.7** and contains the older CLI. A local build here does not support alpha.8 `cairn setup`. Use a matching released archive or source tag. APIs, schemas, and wire contracts may change before 1.0.

Product target: [PRD](docs/product/prd.md) (**Draft**), [roadmap](docs/product/roadmap.md), and [verification plan](docs/engineering/test-plan.md). Published implementation and proposed corrections are distinct; this documentation does not certify deployment or release gates.

## Published alpha.8 architecture

```text
Git repository + supported agent
  → owned hooks / MCP
  → native cairnd: SQLite delivery spools, receipts, identity, finite-age cache
  → cairn-server + PostgreSQL: canonical knowledge, evidence, governance, retrieval
  → later agent context and human web inspection
```

SQLite keeps edge delivery/binding/ownership metadata. Canonical knowledge lives on the server. The current runtime outage cache is in memory; do not assume it survives daemon restart.

The existing stack uses Rust/Tokio/Axum/SQLx, SQLite/PostgreSQL, and Next.js/React/TypeScript. No additional model, embedding, vector database, broker, or separate graph database is required. Existing server relations support bounded graph/replay views.

**Offline:** agent continues; screened work may queue within explicit bounds. Eligible cached context is labelled with age/identity; fresh search is unavailable without server. Saturation can shed eligible capture rows with counted dispositions or explicitly refuse new work; capture is not lossless. Server retry produces one canonical effect, not exactly-once transport. Observed access denial invalidates matching cache.

## Install a matching release

Archives contain `cairn`, `cairnd`, and `cairn-server`, with checksums/provenance. CLI and daemon must live in the same directory. Choose your actual target.

macOS/Linux, using `curl` and either `sha256sum` or `shasum`:

```bash
VERSION=0.1.0-alpha.8
TARGET=aarch64-apple-darwin
# Other targets: x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
ARCHIVE="cairn-v${VERSION}-${TARGET}.tar.gz"

curl -fsSLO "https://github.com/Vellixia/Cairn/releases/download/v${VERSION}/${ARCHIVE}"
curl -fsSLO "https://github.com/Vellixia/Cairn/releases/download/v${VERSION}/SHA256SUMS"
EXPECTED=$(awk -v file="$ARCHIVE" '$2 == file {print $1}' SHA256SUMS)
test "${#EXPECTED}" -eq 64 || { echo "Missing or invalid archive checksum"; exit 1; }
if command -v sha256sum >/dev/null 2>&1; then
  ACTUAL=$(sha256sum "$ARCHIVE" | awk '{print $1}')
else
  ACTUAL=$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')
fi
test "$ACTUAL" = "$EXPECTED" || { echo "Checksum mismatch"; exit 1; }

tar -xzf "$ARCHIVE"
sudo install -m 0755 "cairn-v${VERSION}-${TARGET}/cairn" "cairn-v${VERSION}-${TARGET}/cairnd" /usr/local/bin/
cairn --version
```

Windows PowerShell:

```powershell
$Version = "0.1.0-alpha.8"
$Target = "x86_64-pc-windows-msvc"
$Archive = "cairn-v$Version-$Target.zip"
Invoke-WebRequest "https://github.com/Vellixia/Cairn/releases/download/v$Version/$Archive" -OutFile $Archive
Invoke-WebRequest "https://github.com/Vellixia/Cairn/releases/download/v$Version/SHA256SUMS" -OutFile SHA256SUMS
$ChecksumRows = @(Get-Content SHA256SUMS | Where-Object { ($_ -split '\s+')[1] -eq $Archive })
if ($ChecksumRows.Count -ne 1) { throw "Missing or ambiguous archive checksum" }
$Expected = ($ChecksumRows[0] -split '\s+')[0]
if ($Expected -notmatch '^[0-9a-fA-F]{64}$') { throw "Invalid archive checksum" }
if ((Get-FileHash $Archive -Algorithm SHA256).Hash -ne $Expected) { throw "Checksum mismatch" }

Expand-Archive $Archive -DestinationPath .
$InstallDir = Join-Path $env:LOCALAPPDATA "Cairn"
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item "cairn-v$Version-$Target/cairn.exe","cairn-v$Version-$Target/cairnd.exe" $InstallDir
$env:Path = "$InstallDir;$env:Path"
cairn --version
```

The PowerShell PATH change applies to the current session; add that folder to your user PATH in Windows Settings for future shells. Native advertised targets: macOS arm64/x86_64, Linux arm64/x86_64, Windows x86_64. Packaged availability and journey verification are separate claims; see roadmap support gates.

## Deploy and establish access

Self-hosted server/PostgreSQL is required for fresh canonical memory, including solo use. Native CLI/daemon runs on developer machine; server/web images run on deployment host.

Use release-matching [alpha.8 deployment files](https://github.com/Vellixia/Cairn/tree/v0.1.0-alpha.8/deploy). In an alpha.8 checkout:

```bash
cp deploy/.env.example deploy/.env
# Edit deploy/.env before starting.
docker compose --env-file deploy/.env -f deploy/docker-compose.yml up -d
```

Set `CAIRN_VERSION=0.1.0-alpha.8`, database credentials, and initial admin credentials explicitly. Example files/version fallbacks must not choose an older image unintentionally.

Published example exposes separate API/web ports. Empty `CAIRN_API_ORIGIN` sends browser calls to web origin, but example does not supply same-origin API routing. Configure a reverse proxy serving web at `/` and API at `/api`, or set browser-reachable `CAIRN_API_ORIGIN` and exact `CAIRN_WEB_ORIGIN` for supported split origin. API/DB exposure, TLS/cookies/CORS, and clean browser login need observed verification; `/api/health` returning `{"ok":true}` proves liveness, not complete readiness.

Admin creates accounts and membership; there is no self-registration or self-join. Accounts are created by an administrator with `cairn user create`, and membership is granted with `cairn project member add`. Tokens come from web **Settings**. Published bootstrap-only admin behavior and old Compose/help restart wording disagree; do not rely on environment edits as routine password rotation. Follow candidate-verified web password/recovery behavior.

For this checkout's older alpha.7 server, `CAIRN_ADMIN_EMAIL` and
`CAIRN_ADMIN_PASSWORD` identify the environment-named break-glass account.
Whoever can set those values and restart the server can always obtain administrator
access; the account is restored to the `admin` role and `active`
status on every start. The published alpha.8 target changes this to
bootstrap/recovery-only behavior, so keep this paragraph scoped to the
alpha.7 checkout and verify the exact release before deployment.

**Repository provisioning gap:** published browser project form omits repository remote, while setup requires a matching registered normalized remote. Authorized `POST /api/projects` supports `repository_remote` and makes creator a member; registered remote must already use setup comparison form, for example:

```json
{"name":"Example repository","repository_remote":"github.com/example/repository"}
```

An operator can provision through this supported API and grant developer membership. Full browser-first remote validation/provisioning remains corrective work, not a completed published journey. Detailed [API/permission contract](docs/product/prd.md#7-api) and [roadmap N1/N2](docs/product/roadmap.md#fix--improve) describe the gap.

## Connect the repository

With matching registered remote, project membership, and token already established, run released alpha.8 CLI inside Git repository. Supply secret through protected environment input or JSON stdin; do not put token in repository files.

```bash
CAIRN_SERVER_URL=https://cairn.example.com \
CAIRN_SERVER_TOKEN="$CAIRN_INSTALL_TOKEN" \
cairn setup
```

`CAIRN_INSTALL_TOKEN` above is a preexisting protected shell value. Setup verifies actor, binds authorized project, installs detected integration, launches daemon, and reports web URL/conflicts. It cannot create account/membership or grant access.

Rerun `cairn setup` to refresh matching Cairn-owned resources. User edits and unrelated config remain untouched. Removal is manual and limited to verified owned resources named by setup. See [alpha.8 ownership guide](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/integrations.md).

## Agent and human interfaces

| Agent/client | Capture | Context |
| --- | --- | --- |
| Claude Code | Supported native hooks | Session/prompt delivery where installed capability/trust is observed |
| Codex CLI | Supported native hooks | Session/prompt delivery where installed capability/trust is observed |
| OpenCode | Supported native capture | Automatic delivery declined under published beta-surface contract |
| Generic MCP | Manual supported tools | Explicit context/search; generic server-session lifecycle unsupported |

Five MCP tools: `cairn_context`, `cairn_search`, `cairn_remember`, `cairn_session`, `cairn_handoff`. Explicit native session/handoff recovery uses exact caller identity. Tool presence does not guarantee every action for generic clients.

Human interface is project selector plus **Overview**, **Memory**, **Sessions**, **Governance**, **Settings**. Alpha.8 removes task scope/`cairn_task` and former human CLI administration/search/repair surfaces. Hooks/MCP remain installed hidden machine adapters.

## Privacy, durability, and upgrade

Safe event union excludes raw prompts/transcripts/diffs/output/credentials/vendor payloads; edge and server independently screen bounded fields, including repository-relative tokens where permitted. Refusals name class without echoing unsafe content. Explicit remembering also obeys domain/privacy/evidence/actor checks.

Personal knowledge is owner-private across projects; team guidance follows proposal/admin governance. Reusable sanitized patterns do not become verified in another project merely because they worked elsewhere. Scope/applicability never grants authorization.

Local queued work can be lost with machine/disk before server acceptance. Accepted canonical data depends on server/database backups. Cache loss on restart does not erase server knowledge. Generated/selected/transmitted/confirmed context states remain distinct and never prove model understood guidance.

Before upgrading, back up PostgreSQL; upgrade server before local agents and install CLI/daemon from same archive. Legacy setup preserves SQLite/WAL, creates fresh edge, and produces import/conservation records. Removed task/local-only/ambiguous records stay offline as `removed_feature`; scope is never widened. Web logical transfer excludes credentials and is not a full disaster-recovery backup. Snapshot consistency, bundle size, and exact upgrade/restore journey remain verification targets.

## Known gaps and development

Published/current evidence includes incomplete browser routing/remote provisioning, memory/session browsing limits, silent action/replay feedback, composed delivery-deadline risk, and live-export snapshot/capacity risk. Corrective recovery work has partial local validation; latest recorded required PostgreSQL gate/final journey rerun is unresolved. Varied usefulness and installed/published-artifact journeys are not certified by this README. See [roadmap](docs/product/roadmap.md) and [verification plan](docs/engineering/test-plan.md).

**This alpha.7 checkout:** use [legacy integration guidance](docs/guides/integrations.md) for its CLI behavior. It still includes task/local-authority-era interfaces; those are not V1 product targets. [SECURITY.md](SECURITY.md) also describes this older source, not alpha.8 bootstrap/storage semantics.

Current-checkout build/check prerequisites: pinned Rust toolchain, Git, Node/npm for web, and PostgreSQL for server suites.

```bash
cargo build --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace --all-targets
```

Configure `CAIRN_TEST_DATABASE_URL` to a disposable PostgreSQL before required server checks. Legacy optional suites may skip absent DB; a green local run without DB is not complete server acceptance. Strict prerequisite mode lives in corrective branch and must be used for candidate release evidence.

Current alpha.7 web scripts:

```bash
cd web
npm ci
npm run typecheck
npm run build
# npm run test:e2e requires running server/DB/web and its fixture environment.
```

Alpha.8 source additionally has `npm run api-contract:check`; this checkout does not. Build alpha.8 from its exact tag/isolated checkout when verifying V1. Follow [testing guidance](docs/engineering/testing.md), with version-specific commands and evidence.

| Path | Responsibility |
| --- | --- |
| `crates/cairn-core` | Domain/wire types, screening, configuration, bounds |
| `crates/cairn-git` | Repository identity and remote matching |
| `crates/cairn-store` | SQLite schemas/storage; alpha.7 canonical-era and alpha.8 thin-edge behavior differ |
| `crates/cairn-integrate` | Native/generic/manager integration and byte ownership |
| `crates/cairnd` | Native edge daemon |
| `crates/cairn` | CLI and hidden machine adapters; visible CLI differs by version |
| `crates/cairn-server` | Axum/PostgreSQL canonical server |
| `web/`, `tests/` | Human application and existing verification harnesses |

Start at [documentation index](docs/README.md). Release history remains in [CHANGELOG](CHANGELOG.md); the [alpha.8 tagged changelog](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/CHANGELOG.md) includes the published V1 reduction.

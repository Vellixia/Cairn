# Changelog

All notable changes to Cairn are documented here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.6.0] - 2026-06-23 - Cleanup sprint

A focused cleanup over the v0.5.0 release. **No new features**, no new
endpoints, no new dependencies. The product surface is smaller, the
install path is unambiguous, and the host install ships one binary
instead of two.

### What's changed

**Workspace (21 crates, down from 22)**
- `cairn-server` crate deleted; the in-container server is now
  `cairn-api::bin::cairn-server` declared as a `[[bin]]` in
  `cairn-api`. See ADR-029.
- `cairn-cli` crate renamed to `cairn-client`. The host binary is
  now just `cairn`. See ADR-030.

**Agents (3 supported, down from 6)**
- Dropped: Cursor, VS Code (Copilot), Windsurf, Cline.
- Added: Codex CLI. Verified against the `openai/codex` source tree
  that the `[mcp_servers.<name>]` stdio transport shape is identical
  to OpenCode's. See ADR-028.
- The TOML serializer uses a `<<CAIRN_SKIP>>` sentinel marker to
  detect unchanged sub-blocks (TOML has no per-key `exists?` check).

**Admin bootstrap (env-only)**
- The admin account is now created at server boot from
  `CAIRN_ADMIN_USERNAME` + `CAIRN_ADMIN_PASSWORD`. The dashboard's
  first-run "set admin password" form is gone - it was a
  v0.4.0 -> v0.5.0 footgun.
- Admin ops (token create / revoke, pair-code generation) live in
  the dashboard under **You -> Tokens** and **You -> Pair**. No new
  HTTP routes were added; the existing `/api/devices/*` routes are
  reused.
- `docker-compose.yml` requires both env vars to be set in `.env`
  (or in the compose file directly) before the `cairn` service
  will start. The startup guard fails fast on
  `CAIRN_ADMIN_PASSWORD=""` or length < 12.

**Dead code removed**
- `cairn update` and `cairn login` subcommands removed (were
  never exercised in v0.5.0).
- `self_update` and `dotenvy` dependencies removed.
- `cairn-api/src/events.rs`: dropped `KIND_STATS`,
  `KIND_CHECKPOINT`, `KIND_VECTOR`, `KIND_GRAPH` constants
  (kept `KIND_AUDIT`, `KIND_MEMORY`, `KIND_DRIFT`).
- `cairn-api/src/metrics.rs`: dropped `source_breakdown` helper.
- `cairn-ingest/src/lib.rs`: dropped `write_tmp` helper.
- `plugins/cairn/` directory deleted (9 files); the modern
  install path is `cairn setup`.

**Distribution**
- Host tarball ships exactly one binary: `cairn` (from
  `cairn-client`). The `release.yml` matrix drops the second
  `bin` entry; the in-container `cairn-server` is built into
  the Docker image only.
- Docker `ENTRYPOINT` is now `["cairn-server"]` (was `["cairn"]`).
- The host install script (`./scripts/install.{sh,ps1}`)
  installs the `cairn` client only; the server is now
  Docker-only.

### Migration from v0.5.0

1. **Re-run `./scripts/install.{sh,ps1}`** to replace
   `cairn-cli` with `cairn` on your `PATH`.
2. **Update MCP configs**: every `command: "cairn-cli"` reference
   becomes `command: "cairn"`. (For Claude Code: `.mcp.json` and
   `.claude/settings.json`.)
3. **Add the admin env vars to `.env`**:
   ```
   CAIRN_ADMIN_USERNAME=<your-username>
   CAIRN_ADMIN_PASSWORD=<at-least-12-chars>
   ```
4. **`docker compose down -v && docker compose up -d`** to pick
   up the new entrypoint.
5. **Test count invariant preserved**: `cargo test --workspace`
   still reports 343 passed, 5 ignored.

### New in the docs

- `docs/ADMIN.md` - env bootstrap, dashboard surface, curl
  equivalents, password rotation.
- `docs/PLAN_v0.6.0.md` - this sprint plan.
- `docs/PLAN_v0.5.0.md` -> `docs/archive/PLAN_v0.5.0.md`.
- ADRs 028 / 029 / 030 in `docs/DECISIONS.md`.

---

## [0.5.0] - 2026-06-21 - Context + Reliability + Distribution + Proactive (Phases 3.5 + 4.0 + 4.1 + 4.2 + 5)

The complete v0.5.0 release - 23 sprints across 5 phases. Cairn is now
self-installable, multi-tenant aware, federated, and proactive.

### What's new

**Memory & confidence (Phase 3.5, Sprints 2--3)**
- `confidence: f32` + `pinned: bool` on every memory; reinforced by the
  agentmemory curve `c' = min(1.0, c + 0.1*(1-c))` on every access.
- Provenance edges on `Memory`: `derived_from`, `contradicts`, `supersedes`,
  `applies_to`. New `/dashboard/memory/graph` page with a pure-SVG force layout.
- `MemoryEngine::crystallize()` promotes a working-tier cluster to a semantic-tier
  crystal (agentmemory's "lesson" pattern).

**Reliability (Phase 3.5, Sprints 4--5)**
- New `cairn-session` crate owns session + drift JSONL storage and
  approve/reject workflow. `/dashboard/sessions` + `/dashboard/reliability/drift`
  pages.
- HMAC-SHA256-signed ledger at `<data_dir>/ledger.jsonl` for every context
  assembly. `/api/ledger` + `/api/ledger/verify` expose the chain.
- `/dashboard/savings` page renders the per-assemble savings breakdown.

**Audit + observability (Phase 3.5, Sprint 1)**
- Audit events are now durable HelixDB records (was in-memory ring); the
  `/api/events` SSE stream uses `Last-Event-ID` replay from durable storage
  instead of 5 s polling. `/api/metrics` exposes the live counters.

**Hybrid search (Phase 3.5, Sprint 7)**
- `MemoryEngine::hybrid_search()` combines lexical (BM25-lite) + semantic
  via Reciprocal Rank Fusion; MMR diversity rerank (`lambda=0.7`) keeps the top-N
  non-redundant. Exposed as `/api/search` and `cairn search`.

**Zero-prompt setup (Phase 4.0, Sprint 8)**
- `cairn onboard` runs `doctor --fix` + provisions the local store + wires
  every detected agent in one shot. `cairn doctor --fix` repairs missing
  data dirs, weak MinIO creds, etc. Non-zero exit when remediation is required.

**CLI surface (Phase 4.0, Sprints 9--10)**
- 25+ new MCP tools (`memory_edit`, `memory_delete`, `memory_pin`,
  `memory_reinforce`, `memory_timeline`, `memory_crystallize`, `memory_graph`,
  `graph`, `search`, `metrics`, `stats`, `proactive_recall`, etc.). Total
  tool count is now 41.
- 6 MCP resources: `cairn://memory/graph`, `cairn://memory/timeline`,
  `cairn://savings/today`, `cairn://drift/pending`, `cairn://audit/recent`,
  `cairn://config/toml`.
- 5 MCP prompts: `summarize-drift`, `remember-decision`, `what-do-we-know`,
  `weekly-savings-report`, `drift-triage`.
- New CLI subcommands: `cairn graph related|impact|callgraph`,
  `memory timeline|crystallize`, `search`, `sessions`, `session`, `metrics`.

**Context packages (Phase 4.0, Sprint 11)**
- `.cairnpkg` format: tarball with `manifest.json` + `memory.jsonl` +
  `profile.jsonl` + `patterns.jsonl` + `graph.jsonl` + `signature.sha256`
  + optional `signature.ed25519`. Per-file SHA-256 + HMAC + optional
  Ed25519 signing; rejects oversized (>16 MiB) and tampered packs.
  `.ctxpkg` is accepted as an import alias.
- New `cairn-pack` crate + `cairn pack` with 9 actions:
  `create | info | install | list | remove | export | import | auto-load |
  publish`.

**Distribution polish (Phase 4.0, Sprint 12)**
- **Homebrew tap** at `Vellixia/homebrew-tap` (`brew install Vellixia/tap/cairn`).
- **Non-root Docker volume init.** New `cairn-init` service chowns `/data` to
  uid 10001 before `cairn` starts as non-root. The pre-0.5.0 `user: "0"`
  workaround is gone.

**Self-hosted registry (Phase 4.1, Sprints 13--14)**
- `cairn-registry` crate with HTTP endpoints under `/registry/*`:
  publish, search, install, manifest, signed download.
- **Ed25519 pack signing** - signers add their public key to `manifest.json`;
  verifiers reject packs whose signature doesn't match.
- **Trust scopes** - Local / Team / Public. Each peer in `TrustGrant` declares
  what scope they allow. Scope mismatch returns `RegistryError::ScopeDenied`.
- **Revocation cascade** - `revoke_if_exists` records the event and pulls
  it across federation; no peer can re-publish a revoked pack.

**Federation + sync (Phase 4.1, Sprint 15)**
- `cairn-sync` crate with offline-first CRDTs:
  - `GCounter` for cumulative counters (memory access counts).
  - `ORSet` for memory sets (concurrent add+remove resolves to present).
- **Vector clocks** per-actor for causal ordering of `MemoryOp::Put/Bump/Tombstone`.
- **End-to-end encryption** - Argon2id (64 MiB / 3 iter) -> ChaCha20-Poly1305
  AEAD with AAD bound to `from->to` actor pair.

**Benchmarks + landing (Phase 4.2, Sprints 16--17)**
- `cairn-bench` crate with three harnesses:
  - `LongMemEval` (synthetic fixtures: `alex_employer_history`,
    `migration_timeline`).
  - `HorizonBenchmark` (recall profile at 10/25/50/100/200-step horizons).
  - `RetentionBenchmark` (Cairn policy preserves ~70% of important memories
    vs ~30% for naive LRU at the same capacity).
- Public landing page at `web/src/app/page.tsx` with hero + savings table +
  honest comparison + install cards + trust signals.
- `docs/BENCHMARKS.md` rewritten with methodology + reproducible numbers.
- `web/src/app/dashboard/registry/page.tsx` - pack registry browser with
  scope chips + provenance panel.

**Proactive recall (Phase 5, Sprint 18)**
- New `cairn-proactive` crate with a local intent classifier:
  - Pure-Rust heuristic - question markers, recall cues, file/path mentions,
    reference pronouns. Sub-millisecond per turn.
  - `ProactiveHook` returns up to 3 relevant memories or a `Skipped { reason }`
    for diagnostics.
- Per-project opt-out: `cairn prefer cairn.proactive_recall=false
  --applies-to <project_root>` disables for a project prefix.
- New MCP tool: `proactive_recall(prompt, project_root?)`.

**Multi-tenant (Phase 5, Sprint 19a)**
- New `OrgId` type on every `Memory`. `Config::multi_tenant` (env
  `CAIRN_MULTI_TENANT`) toggles tenant isolation.
- `MemoryEngine::recall_for_org` filters by `org_id` before any ranking.
- Default org `default` preserves single-tenant behaviour when the flag is off.

**cairn.sh reverse proxy (Phase 5, Sprint 19b)**
- New `cairn-proxy` crate + binary.
- `/registry/packs`, `/registry/search`, `/registry/federation/pull`,
  `/health` endpoints fan out to a configurable peer list.
- Best-effort peer failures don't abort the merge.

**PWA + push (Phase 5, Sprint 20)**
- Service worker (`web/public/sw.js`) with cache-first static + network-first
  `/api/*`. Falls back to cached shell when offline.
- Web App Manifest at `web/public/manifest.json` - installable PWA.
- New `PushStore` + `POST /api/push/subscribe`, `POST /api/push/unsubscribe`,
  `GET /api/push/list`. Each subscription is a JSON file under
  `<data_dir>/push/`.

**Browser extension capture endpoint (Phase 5, Sprint 21)**
- Server endpoint `POST /api/extensions/capture` (loopback-only, 20 KB cap)
  for capturing browser selections and page text as Cairn memories.

**Transcript ingestion (Phase 5, Sprint 22)**
- New `cairn-ingest` crate with VTT/SRT/JSON parsers + speaker-window
  chunking (default 60 s).
- `POST /api/ingest/transcript` - auto-detect format; writes one memory
  per chunk with `applies_to = ["transcript:<source_url>"]`.

**Mobile companion (Phase 5, Sprint 23)**
- `web/src/app/mobile/page.tsx` - standalone PWA surface with biometric
  gate, savings card, drift-approval queue.
- Best-effort WebAuthn probe; falls back to a tap-to-unlock button.

### Security

- Web dashboard ships a **per-request CSP nonce** (random 16 bytes per
  response, injected into `<script>` tags). Closes the static-`script-src`
  gap that would otherwise block the v0.5.0 interactive pages.
- **Setup wizard v2** (`/setup/wizard`) replaces the original `/setup` flow
  with a 4-step admin -> embed -> pair -> health walkthrough. v1 `/setup` is
  retained as a fallback with a deprecation banner.
- **HMAC-SHA256 ledger** detects tamper attempts on the savings record.
- **Ed25519 pack signatures** reject tampered downloads even when the
  registry itself is compromised.
- **Argon2id + ChaCha20-Poly1305 E2E encryption** for federation sync.
- **`SECURITY.md`** rewritten with a 10-row threat model + hardening checklist.

### Test count

`cargo test --workspace` reports **330 passed, 5 ignored, 0 failed** as of
this release (up from 118 in 0.3.0 and 282 in 0.4.0). The 5 ignored tests
require a live HelixDB.

### Docs

- `docs/PLAN_v0.5.0.md` - full 23-sprint plan + success metrics + risks.
- `docs/DECISIONS.md` - 27 ADRs (binary split -> proactive intent classifier
  + multi-tenant + cairn.sh proxy).
- `docs/BENCHMARKS.md` - LongMemEval + horizon + retention numbers + methodology.
- `docs/ROADMAP.md` - verification rows for every Phase 3.5--5 sprint.

---

## [0.4.0] - 2026-06-20 - Context + Reliability Layer (Phase 3.5 + 4.0)

### What's new

**Memory & confidence (Sprint 2--3)**
- `confidence: f32` + `pinned: bool` on every memory; reinforced by the
  agentmemory curve `c' = min(1.0, c + 0.1*(1-c))` on every access.
- Provenance edges on `Memory`: `derived_from`, `contradicts`, `supersedes`,
  `applies_to`. New `/dashboard/memory/graph` page with a pure-SVG force layout.
- `MemoryEngine::crystallize()` promotes a working-tier cluster to a semantic-tier
  crystal (agentmemory's "lesson" pattern).

**Reliability (Sprint 4--5)**
- New `cairn-session` crate owns session + drift JSONL storage and
  approve/reject workflow. `/dashboard/sessions` + `/dashboard/reliability/drift`
  pages.
- HMAC-SHA256-signed ledger at `<data_dir>/ledger.jsonl` for every context
  assembly. `/api/ledger` + `/api/ledger/verify` expose the chain.
- `/dashboard/savings` page renders the per-assemble savings breakdown.

**Audit + observability (Sprint 1)**
- Audit events are now durable HelixDB records (was in-memory ring); the
  `/api/events` SSE stream uses `Last-Event-ID` replay from durable storage
  instead of 5 s polling. `/api/metrics` exposes the live counters.

**Hybrid search (Sprint 7)**
- `MemoryEngine::hybrid_search()` combines lexical (BM25-lite) + semantic
  via Reciprocal Rank Fusion; MMR diversity rerank (`lambda=0.7`) keeps the top-N
  non-redundant. Exposed as `/api/search` and `cairn search`.

**CLI surface (Sprint 9--10)**
- 25 new MCP tools (`memory_edit`, `memory_delete`, `memory_pin`,
  `memory_reinforce`, `memory_timeline`, `memory_crystallize`, `memory_graph`,
  `graph`, `search`, `metrics`, `stats`, etc.). Total tool count is now 40+.
- New CLI subcommands: `cairn graph related|impact|callgraph`,
  `memory timeline|crystallize`, `search`, `sessions`, `session`, `metrics`.

**Zero-prompt setup (Sprint 8)**
- `cairn onboard` runs `doctor --fix` + provisions the local store + wires
  every detected agent in one shot. `cairn doctor --fix` repairs missing
  data dirs, weak MinIO creds, etc. Non-zero exit when remediation is required.

**Context packages (Sprint 11)**
- `.cairnpkg` format: tarball with `manifest.json` + `memory.jsonl` +
  `profile.jsonl` + `patterns.jsonl` + `graph.jsonl` + `signature.sha256`.
  Per-file SHA-256 + HMAC signature; rejects oversized (>16 MiB) and tampered
  packs. `.ctxpkg` is accepted as an import alias.
- New `cairn-pack` crate + `cairn pack` with 9 actions:
  `create | info | install | list | remove | export | import | auto-load |
  publish`.

**Distribution polish (Sprint 12)**
- **Homebrew tap** at `Vellixia/homebrew-tap` (`brew install Vellixia/tap/cairn`).
- **Non-root Docker volume init.** New `cairn-init` service chowns `/data` to
  uid 10001 before `cairn` starts as non-root. The pre-0.5.0 `user: "0"`
  workaround is gone.
- **README OpenCode quickstart** section.

### Security

- Web dashboard ships a **per-request CSP nonce** (random 16 bytes per
  response, injected into `<script>` tags). Closes the static-`script-src`
  gap that would otherwise block the v0.5.0 interactive pages.
- **Setup wizard v2** (`/setup/wizard`) replaces the original `/setup` flow
  with a 4-step admin -> embed -> pair -> health walkthrough. v1 `/setup` is
  retained as a fallback with a deprecation banner.
- **HMAC-SHA256 ledger** detects tamper attempts on the savings record.

### Test count

`cargo test --workspace` reports **225 passed, 5 ignored, 0 failed**
as of this release (up from 118 in 0.3.0). The 5 ignored tests require a
live HelixDB.

See [ADR-010 through ADR-016](docs/reference/decisions.md) for the full reasoning behind
each decision.

---

## [0.3.0] - 2026-06-19 - P0--P3 Security & Build Hardening

### Breaking changes

- **CLI binary split.** The single `cairn` binary was replaced by two
  binaries: `cairn` (the server: `serve`, `token`, `pair-code`) and
  `cairn` (client commands: `setup`, `mcp`, `hook`, `sync`, `bench`,
  `pair`, `update`, `rule`). The `cairn install <agent>` subcommand was
  removed; use `cairn setup <agent>`. User scripts that invoke
  `cairn install` must be updated.

- **Device tokens are now signed JWTs (HS256), not opaque bearer
  values.** Previously-issued plaintext tokens are invalid after upgrade
  to this release. Re-mint each device token:
  ```sh
  cairn token create --name <device> --scope <admin|write|read>
  ```
  The bearer value is shown exactly once. The server stores only token
  id, name, scope, and created_at; the JWT itself is regenerated from
  those fields + `CAIRN_SECRET_KEY` on each request.

- **`CAIRN_SECRET_KEY` is now required and must be >= 32 bytes.** The
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

# Changelog

All notable changes to Cairn are documented here. Versions follow [Semantic Versioning](https://semver.org/).

> **Pre-1.0 notice:** Until 1.0 lands, minor versions may carry breaking changes
> on the upgrade path. Pin the version you install and re-read this file on every bump.

## [Unreleased]

### Fixed — Web UI audit remediation
- **Setup page navigates to non-existent `/dashboard` (W1).** Changed
  `router.push("/dashboard")` to `router.push("/")` — the overview lives at
  the root route, not `/dashboard`.
- **Mobile page raw `fetch()` bypassing typed wrapper (W2).** Replaced raw
  `fetch()` calls with the typed `request()` wrapper from `@/lib/api`. Fixes
  the JSON parse error on non-JSON responses and adds 401 redirect support.
- **Heatmap month labels misaligned (W10).** `buildMonthLabels` incremented
  week on Monday; `generateGrid` on Sunday. Fixed to use Sunday consistently.
- **Duplicate fetch for `/api/devices/audit` (W5).** `ActivityTimeline` used
  `qk.activityAudit` while `useDevicesAuditQuery` used `qk.devicesAudit` —
  same endpoint, two keys, double fetch. Consolidated to `qk.devicesAudit`.
- **SSE invalidation gaps (W6/W14).** Added `session`, `metrics`, `ledger`,
  `config`, `profile` to `INVALIDATION_MAP` — those views now auto-refresh
  on server events.
- **Duplicated `Me` interface (W7).** `stores/me.ts` had its own `Me` type;
  now imports from `api.ts` (single source of truth).
- **`sonner.tsx` dead `useTheme()` call (W13).** No `ThemeProvider` mounted,
  so `useTheme()` always returned `"system"`. Removed the call, hardcoded
  `theme="dark"` (matching the app's dark-only design).

### Fixed — Rust engine audit remediation
- **Promotion scoring ceiling unreachable (R1).** `fast_promotion_score`
  weights were `kind_prior * 0.4 + cross_score * 0.6` — with
  `cross_project_hits` always 0 (scope isolation), the ceiling was 0.36,
  below every threshold. Changed to `kind_prior * 0.8 + cross_score * 0.2`
  so a high `kind_prior` alone reaches the candidate band (Fact = 0.72).
- **Drift log filter ignored (R3).** `list_drift` handler hardcoded `None`
  for the status filter. Now extracts `Query<DriftFilter>` with optional
  `status` and `limit` params.
- **Static handler missing `<path>/index.html` lookup (R4).** Added the
  Next.js nested-folder convention to the lookup chain. Also made
  `find_shell_in` prefer `[param]`-style placeholders over plain listing
  pages so dynamic routes get the right client component shell.

### Fixed — Audit remediation (Bugs #1–#9 vs reference implementations)
- **Shared engines in `tools_call` (Bug #1).** The `/api/tools/call` handler
  created a fresh `McpServer::new()` per request, opening a new `Store` +
  `ContextEngine` + `Guard` — bypassing `AppState`'s shared `Arc` engines.
  Every read went through an empty cache (always `Full`, never `Cached` or
  `Diff`), every checkpoint saw stale file versions, and verify-against-baseline
  could never find the baseline. Now uses `McpServer::from_engines()` with
  `AppState`'s shared engines. Fixes drift from context and checkpoint problems.
- **RemoteProxy `LocalReader` (Bug #2).** The old `read_file_local` was
  stateless — always returned `Full`, ignored `mode`, no mtime cache, no diff.
  Replaced with `LocalReader` that tracks mtime (re-read killer: `Cached` for
  unchanged files), produces `Diff` on change (with 60% auto-delta fallback),
  and respects `mode` (auto/full; signatures/map fall back to full on proxy path
  since tree-sitter isn't available without `engine`).
- **PostToolUse verify-against-baseline (Bug #3).** `Guard::verify_against_baseline`
  was never called by any hook or tool — post-edit corruption detection was dead
  code. Now `PostToolUse` hook calls `POST /api/guard/verify-baseline` for each
  edited file (fire-and-forget, spooled). New MCP tool `verify_baseline` added.
- **Working tier cap demotes instead of deletes (Bug #7).** `run_working_tier_cap`
  previously deleted oldest excess working memories. Now demotes them to
  `Episodic` tier (agentmemory's auto-page pattern) — content and edges preserved.
- **Cross-project corruption guard (Bug #9).** `run_contradiction_detection`'s
  auto-resolve supersedes path could supersede a memory from a different project.
  Now refuses to supersede across project boundaries (same-scope or Global-only).

### Added — Audit remediation
- **`verify_baseline` MCP tool.** Compare current on-disk file against the
  version recorded when the agent last read it (PostToolUse corruption check).
- **`POST /api/guard/verify-baseline` API endpoint.** Same as the MCP tool,
  for direct API consumers.
- **Semantic graph edges (Bug #6).** Two new edge types: `related_to` (auto-derived
  from shared `concepts` at write time — bidirectional between memories sharing ≥1
  concept) and `depends_on` (auto-derived from simple import analysis of `files` —
  scans for `use`/`import`/`require`/`#include` statements). `EdgeKind` enum extended.
  `Memory` struct gains `related_to` and `depends_on` fields.

### Changed — Audit remediation
- **`McpServer::dispatch` signature** now requires `&ScopeCtx` (from previous
  release). `tools_call` now passes `AppState`'s shared scope.
- **`McpServer::from_engines`** new constructor accepting pre-built `Arc` handles.
- **`Memory` struct** gains `related_to: Vec<String>` and `depends_on: Vec<String>`
  fields (both `#[serde(default)]`, backward compatible).

### Fixed
- **Project memory scoping end-to-end.** The MCP `remember` tool never set
  `scope_type`, so every memory written by an agent defaulted to `Global` —
  project memory counts were structurally 0, and the promotion pipeline had
  nothing to act on. `dispatch` now takes a `ScopeCtx` parameter, threaded
  from the `tools_call` HTTP handler's `X-Cairn-Project` header. `remember`
  defaults to `Project` scope when a project is detected; explicit
  `scope_type` overrides the default. The client hook's `UserPromptSubmit`
  write also sets `scope_type: "project"` when a project is active.
- **Memory graph feeds from `files`.** `MemoryEngine::remember` now
  auto-derives `applies_to` edges from the `files` field, so every remember
  with file paths populates the provenance graph without needing
  crystallize/consolidate.

### Added
- **`scope_type`, `scope_id`, `concepts`, `files` args on the `remember`
  MCP tool.** Agents can now explicitly control scope and feed graph edges.
- **`document_ingest` MCP tool.** Ingest a file or URL into the document
  store for semantic search. Reads locally when `content` is omitted;
  project-scoped via the same `ScopeCtx`.
- **`document_search` MCP tool.** Search ingested document chunks by
  semantic similarity. Project-scoped via `ScopeCtx`.

### Changed
- **`McpServer::dispatch` signature** now requires a `&ScopeCtx` third
  parameter. The `tools_call` HTTP handler extracts it from request
  extensions; stdio-only paths pass `ScopeCtx::default()`.

## [0.8.3] - 2026-07-08

### Changed
- Bumped workspace version to 0.8.3.

## [0.8.2] - 2026-07-07 - Hardening: expand short handles, error clarity, hook UX

Follow-up to v0.8.1 addressing the external audit report (E001-E003, W001-W005).
No new features; behavior fixes and developer-facing clarity.

### Fixed
- **`expand` now resolves short handles** (E001). `read` advertises
  `handle = hash.short()` (12 chars) as the value to pass to `expand`, but
  `expand` previously only accepted the full 64-char content hash - any caller
  that followed the advertised contract got `unknown handle`. The blob store
  now resolves short prefixes to the full hash via a shard-directory scan, and
  `ContextEngine::expand` retries with the resolved full hash on a miss.
  Handles were never TTL'd or session-bound (they're content-addressed blobs in
  the store); the failure was a contract mismatch, not expiration.
- **Workspace escape error message** (E002) now names the configured
  `CAIRN_WORKSPACE_ROOT` and points at the env var, instead of the bare
  `path escapes workspace root: <path>`.
- **`POST /api/memory/:id/pin` accepts an empty body** (W004), defaulting
  `pinned=true`. Previously the Axum `Json` extractor required a
  `Content-Type: application/json` header and a parseable body even though pin
  is the common case. Send `{"pinned": false}` to unpin; empty body pins.

### Changed
- **MCP `initialize` response now includes `workspaceRoot`** in `capabilities`
  when a workspace root is configured, so MCP clients can discover the
  confinement boundary without a separate round-trip.
- **`proactive_recall` MCP tool returns `{matches, reason}`** instead of a bare
  array. The `reason` string distinguishes "no relevant memories" from "the
  classifier skipped this prompt" - previously the skip reason was only visible
  in `tracing::debug`.
- **Claude Code `health()` now checks `settings.json`** (E003 follow-up). Flags
  a missing or unparseable `<project>/.claude/settings.json` when Claude Code
  is detected, since without that file the lifecycle hooks silently stop
  firing. The install path itself was already correct (project-scoped); this
  adds a doctor-level guard against the file going missing or getting
  clobbered.

### Docs
- **`cairn hook --help`** now explains the stdin JSON protocol and lists every
  event name, so `cairn hook UserPromptSubmit --prompt "..."` failures
  (W002/W003) point at the right answer instead of just rejecting the flag.
- **`AGENTS.md`** has a new "Hook stdin format" table mapping each event to its
  JSON shape.
- **`expand` and `sanitize` MCP tool schemas** carry the missing notes:
  `expand` clarifies that handles never expire and short/full are both
  accepted; `sanitize` documents the min-length thresholds that deliberately
  skip short fragments like `sk-abc123` (W001).
- **README + docs/README** now carry a pre-1.0 disclaimer: Cairn is under active
  development, interfaces may change between minor versions without prior
  notice, and there is no stability guarantee until 1.0.

## [0.8.0] - 2026-07-05 - Intelligent memory + observability-first dashboard

Two threads landed together: nine backend sprints deepening the memory
engine's autonomy, and a ground-up rebuild of the web dashboard around
"humans watch, agents do."

### Engine (Sprints 1-9)

- **Storage: HelixDB -> SurrealDB** (Sprint 1) - full `StoreBackend` rewrite
  against `surrealdb/surrealdb:v3`; SDK and server major versions must
  match. Drops HelixDB, MinIO, and the `quinn-proto` RUSTSEC patch HelixDB
  required - the Docker Compose stack is now just `cairn` + `surreal`.
- **Scope model** (Sprint 2) - Global/Project/Session recall isolation;
  memories (and, from Sprint 6 onward, documents) are scoped and blended
  rather than siloed.
- **Auto project detection + registry** (Sprint 3) - `cairn-client` detects
  the current project and threads `X-Cairn-Project` automatically.
- **In-process cron scheduler** (Sprint 4) - session TTL expiry and memory
  decay run on a schedule instead of on-demand.
- **LLM-driven concept extraction** (Sprint 5) - contradiction detection,
  promotion scoring.
- **RAG document ingest + search** (Sprint 6) - merged into `cairn-assemble`.
- **MMR in base recall + promotion/intelligence dashboard** (Sprint 7).
- **Full-auto promotion, drift & anchor autopilot** (Sprint 8) - the manual
  approve/reject workflow is gone; autopilot decides at verify time, and
  danger-tier findings never auto-approve.
- **Resilience & self-tuning** (Sprint 9, final).

### Web: observability-first redesign

- **New IA**: Now / Memory (browser) / Projects / Documents / Automation +
  You. The `?tab=` hub pattern is gone from monitor pages.
- **Memory Browser** (`/memory`) - every memory, filterable (scope/tier/
  kind/pinned/suspicious/query) + sortable + paginated, with a full-detail
  drawer showing all 22 fields and clickable provenance edges.
- **"basalt & ember" visual identity** - full theme rewrite: near-black
  graphite base, ember accent, Inter + JetBrains Mono (self-hosted,
  offline-safe), dot-grid background.
- **Registry and Trust removed from the web UI** (still fully supported
  server/CLI/MCP-side - `.cairnpkg` packs via `cairn pack`, the
  `registry_search` MCP tool); Automation absorbs what Trust used to show.
- **Memories gained structure**: optional `title` + `reasoning` fields,
  threaded through `remember`/`memory_edit` end to end; the browser falls
  back to the first line of content for older memories that predate this.
- **Documents are now project-scoped**: `project_id` on ingest/list/search,
  same "project's own + global blend" policy as memory recall.
- **Preferences page rewritten** - explanation, add form, per-row delete.
  Previously a dead end: "No preferences yet, use `cairn prefer`" with no
  way to act on it from the web.
- **Dead code removed**: 2 unused UI components, 8 unused API types, 6
  unused Tailwind aliases, `DoctorOptions::interactive`, and assorted dead
  Rust helpers.

### Fixed

- `/api/setup/health`'s Database pill was permanently stuck on "warn" with
  a literal "undefined" in its tooltip - the web client had the response
  shape typed wrong.
- `projects/[id]` and `you/sessions/[id]` never worked for a real id, on
  hard reload or client-side navigation - Next's static export bakes one
  placeholder shell, and `useParams()`/`params.id` return that placeholder
  forever with no real Next.js server to reconcile against. Fixed with a
  `useUrlId()` hook that reads `window.location.pathname` directly; the
  same pattern applies to any future `[id]` route in this codebase.
- rustfmt/clippy drift accumulated from CI's floating `stable` toolchain
  picking up newer lint/format rules (191 formatting diffs, 6 clippy
  lints) across pre-existing code, unrelated to this release's own changes.
- Two quick-xml advisories (RUSTSEC-2026-0194, RUSTSEC-2026-0195) via
  `self_update`; verified neither vulnerable API path is reachable through
  Cairn's usage and documented as ignores in `deny.toml`/`.cargo/audit.toml`.

### Breaking

- Registry and Trust web pages are gone (backend/CLI/MCP unaffected).
- Manual drift approve/reject API routes are removed in favor of autopilot.
- `edit_memory` (store trait, engine, API, MCP) gained two new trailing
  parameters (`title`, `reasoning`) - a source-level break for any direct
  caller; existing callers passing `None` see no behavior change.

Stats: `cargo test --workspace` passes clean; clippy and rustfmt clean; all
CI advisory scans pass with documented, verified-unreachable ignores.

## [0.7.1] - 2026-07-02 - Live-e2e coverage + dashboard fixes + docs reorg

A verification-driven patch release: walked every documented surface (30
docs covering auth, memory, compression, tiers, profile, guard, share,
ingest, registry, and MCP transport) against a real Docker stack, fixed
what broke, and reorganized the docs around reader intent.

### Found and fixed by the live walk

- 3 dashboard crashes surfaced only by real navigation, not unit tests.
- `static_handler` returned the wrong response for missing assets instead
  of a 404, and didn't percent-decode paths.
- `parse_vtt` accepted files missing the `WEBVTT` header instead of
  rejecting them.
- The OpenAPI spec was missing registry routes.
- The memory tracker could panic on a fresh boot - the cutoff wasn't
  clamped.
- Setup wiring bugs across agents: OpenCode double-loading its config, the
  Codex TOML shape, the Claude Code matcher.

### Dashboard

- Registry surfaced in the sidebar nav; hidden dashboard pages surfaced via
  the command palette, HubTabs, and topbar.
- `/api/metrics/savings` added; registry nested under `/api/registry`.
- `HelpButton` tolerant of missing `helpCopy` entries.

### Docs

- Reorganized `docs/` by reader intent, with authoring conventions and
  templates.
- New 30-document live-e2e test suite under `docs/testing/`, each doc
  walked and marked PASS/SKIP against a live stack.

Also fixed while greening CI: a stale VTT test fixture and an `anyhow`
advisory.

## [0.7.0] - 2026-06-29 - Engine intelligence + dashboard UX + new crates

20 plan items shipped end to end, grouped in four tracks.

### Engine intelligence

- Anti-inflation cap: structural/diff context views fall back to Full when
  they wouldn't actually be cheaper.
- 60% diff threshold: auto-delta falls back to Full once the delta exceeds
  60% of the original.
- Triple-stream search: BM25 + HNSW vector + graph proximity, fused with
  RRF (dynamic renormalization, session diversification) - completes the
  "graph leg" of hybrid search.
- LLM consolidation: `LlmConsolidator` (`consolidate_semantic`,
  `extract_procedures`, `synthesize_insights`) + `apply_decay`.
- Contradiction detection via Jaccard similarity + `auto_forget`.
- Follow-up tracker (rolling-window recall-quality metric) and bounce
  tracker (rolling-window compressed-read -> full-read detection).
- Context injection now defaults **off**, gated by `CAIRN_INJECT_CONTEXT`
  on `UserPromptSubmit` and the proactive hook.

### Dashboard UX

- Live updates over WebSocket; USD savings surfaced on the overview.
- Compression Lab: side-by-side 4-mode comparison.
- Architecture report, context pressure gauge with eviction candidates,
  activity heatmap, knowledge graph enhancements, memory browser with
  search + filters.

### Developer experience

- Structured REST error envelope (`error` + `error_code`).
- `/api/openapi.json` + `/api/capabilities`.
- LLM-driven query expansion, reformulations merged by max score.

### Engine depth

- Gotcha tracker: in-memory failure clusterer, auto-promotes to a Gotcha
  memory past a threshold.
- Shell compression refactored to a 9-category registry, 12 new patterns
  (npm, pnpm, pip, eslint, tsc, docker, kubectl, curl, rg, etc).

### New crate

- `cairn-rerank`: cross-encoder reranking via fastembed, gated by the
  `local` feature - completes the "Rerank + MMR diversity" backlog item.

Stats: 555 tests passing, 0 failed. Clippy clean, fmt clean.

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

# Cairn — Improvement Recommendations

**Date:** 2026-06-24
**Scope:** Forward-looking, advisory. Written after the v6.1.0 test & security audit
(see [`AUDIT_v6.1.0.md`](AUDIT_v6.1.0.md)).
**Status:** None of the items below are implemented — this is a prioritized backlog, not a
changelog. Each entry is independently actionable in a later sprint.

Every finding was **verified against current code** during review (file/line references are
real, not inferred from summaries). Items the audit already settled are cross-linked under
[Out of scope](#out-of-scope--already-accepted) rather than restated.

---

## How to read this

Each item carries an **effort** and an **impact** tag.

| Effort | Meaning |
|---|---|
| `S` | A focused change — hours, contained to one or two files. |
| `M` | A feature-sized change — a day or two, new code paths + tests. |
| `L` | Multi-sprint; design decision required. |

| Impact | Meaning |
|---|---|
| 🔴 | High — correctness, availability, or security exposure under realistic conditions. |
| 🟡 | Medium — operability, maintainability, or a latent footgun. |
| ⚪ | Polish — maturity / DX; valuable before a 1.0 but not urgent. |

**Baseline (already strong, not repeated below):** clean acyclic 21-crate dependency graph, no
`unsafe`, Argon2id password hashing, TLS gating with a loopback exemption, Ed25519-signed packs,
per-request CSP nonce, SLSA-3 + cosign release provenance, 30 ADRs, and 464 passing tests after
v6.1.0. The recommendations target the gaps around that solid core.

---

## Tier A — quick, high-value wins

### A1. Close the CI coverage gaps — `S` / 🔴

**What.** The PR-gating workflow tests less than the project promises.

**Why.** [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) runs a single job on
`ubuntu-latest` with the `stable` toolchain only:
- The **declared MSRV `1.85`** (`Cargo.toml` `rust-version`) is never compiled, so an
  accidental use of a newer-than-1.85 API would not be caught until a user on 1.85 hits it.
- The `web` job runs only `npm run build` — it never runs `npm run lint` or `npm run typecheck`,
  even though both scripts exist in [`web/package.json`](../web/package.json). TypeScript errors
  and lint regressions land silently. (The `<div>`-in-`<p>` hydration bug fixed in v6.1.0 is
  exactly the class of issue a typecheck/lint gate surfaces early.)
- `cargo audit` / `cargo deny` live in a **separate** workflow
  ([`rust-security.yml`](../.github/workflows/rust-security.yml)) that does not block merges, so a
  newly-disclosed advisory can merge unnoticed.

**Suggested approach.** Add a second toolchain job pinned to `1.85` (`cargo build --workspace
--locked`); add `npm run lint` and `npm run typecheck` steps to the `web` job; and either move the
audit/deny checks into `ci.yml` or mark the security workflow as a required status check. No
production code changes.

---

### A2. Add a timeout to HelixDB queries — `S` / 🔴

**What.** Store calls can hang forever if HelixDB stalls.

**Why.** Every store operation funnels through `HelixBackend::block()` at
[`crates/cairn-store/src/helix.rs:138`](../crates/cairn-store/src/helix.rs), which runs
`rt.block_on(fut)` with **no deadline**; `run()` at `helix.rs:148` dispatches all dynamic queries
through it. If the HelixDB server is slow, overloaded, or wedged, the calling Axum worker thread
blocks indefinitely with no error returned to the client and no upper bound on resource use.

**Suggested approach.** Wrap the future in `tokio::time::timeout(...)` inside `block()` (or `run()`),
with a configurable duration (e.g. `CAIRN_HELIX_TIMEOUT_SECS`, default ~5 s). Map elapse to
`Error::Storage("helix query timed out")` so it surfaces as a clean 5xx instead of a hang.

---

### A3. Make `/api/health` a real probe — `S` / 🟡

**What.** The health endpoint always reports OK, even when dependencies are down.

**Why.** `health()` at [`crates/cairn-api/src/lib.rs:511`](../crates/cairn-api/src/lib.rs) returns
a static `{"status":"ok","name":"cairn","version":...}` regardless of whether HelixDB or the
embedder are reachable — so a load balancer or uptime check can't distinguish "serving" from
"process up but store unreachable." The logic for a real check **already exists**:
`setup_health()` at [`crates/cairn-api/src/setup_wizard.rs:34`](../crates/cairn-api/src/setup_wizard.rs)
probes `helix_reachable` (via `store.count_memories()`) and `embedder_loaded`.

**Suggested approach.** Add a `/api/health/deep` route (or a `?deep=1` mode on the existing one)
that reuses the `setup_health` probes and returns per-component status with an appropriate status
code (200 healthy / 503 degraded). Keep the cheap static `/api/health` for liveness.

---

## Tier B — operability hardening

### B1. Request-logging middleware + document `RUST_LOG` — `M` / 🟡

**What.** There is no per-request log line, so production traffic is invisible.

**Why.** The router in `crates/cairn-api/src/lib.rs` has no tower layer that logs method, path,
status, or latency. `tracing_subscriber` is initialized in the server binary so `RUST_LOG` works,
but it is not documented in [`.env.example`](../.env.example) and the default emits almost nothing
per request. An operator debugging a slow or failing call has no trail to follow.

**Suggested approach.** Add a lightweight request-logging layer (e.g. `tower_http::trace::TraceLayer`
or a small custom middleware) emitting structured lines to stderr; add a documented
`RUST_LOG=cairn_api=info,cairn_store=warn` example to `.env.example`. Pair naturally with a
correlation/request-id header for error triage.

---

### B2. Rate-limit the auth and write surface — `M` / 🟡

**What.** No throttling anywhere; login and write endpoints are brute-force / DoS exposed.

**Why.** A repository-wide search for `governor` / `rate limit` / throttling returns **zero hits** —
there is no per-IP or per-token limiter. `/api/auth/login` accepts unlimited password attempts, and
write endpoints accept unbounded request rates (the only guard is the 1 MiB body limit). This is
already noted as deferred in [`AUDIT_v6.1.0.md`](AUDIT_v6.1.0.md) ("Login rate limiting"); it is
called out here as the one deferred item worth **promoting** to active work.

**Suggested approach.** Add a per-IP limiter (e.g. `tower-governor`) scoped to the auth surface and
write routes, with conservative defaults and a `429` + `Retry-After` response. Alternatively,
document that a reverse proxy (nginx/Caddy) is expected to provide this and state it in
`SECURITY.md` — but in-process is friendlier for the self-hosted single-binary story.

---

### B3. Audit-counter scan + non-durable ledger — `M` / 🟡

**What.** Two append-only data paths that don't scale and don't survive restart.

**Why.**
- `bump_audit_counter()` at [`crates/cairn-store/src/helix.rs:751`](../crates/cairn-store/src/helix.rs)
  reads **all** `AuditCounter` rows and takes `.max()` on every append (likewise
  `max_audit_event_id()` at `helix.rs:740`), then inserts a new row — so the row set grows
  unbounded and each append is O(n) in the number of prior audit events. (The `.max()` is correct
  post-v6.1.0; the cost is the issue, not correctness.)
- The savings `Ledger` at [`crates/cairn-api/src/ledger.rs:57`](../crates/cairn-api/src/ledger.rs)
  is an in-memory `VecDeque` only — entries are lost on restart. The struct carries a
  self-acknowledged FIXME at `ledger.rs:51` (sign `price_usd_per_million_tokens_at_sign_time` so
  historical USD is reproducible, "tracked for v0.6") that slipped past the v0.6 release.

**Suggested approach.** Replace the audit counter with a single upserted row (O(1) per append).
Mirror ledger entries to the store on append and re-seed the in-memory ring on startup; while
touching the signed payload, close the `ledger.rs:51` FIXME (it is already flagged as a breaking
schema change, so version it deliberately).

---

## Tier C — product & maturity

### C1. Memory expiry / garbage collection — `M` / 🟡

**What.** Memories decay in confidence but are never reclaimed.

**Why.** The memory model applies Ebbinghaus-style confidence decay, but there is no `expires_at`
field and no GC job — low-confidence, unpinned memories accumulate in HelixDB indefinitely. A
long-running instance's store only grows, slowly diluting recall quality and increasing storage.

**Suggested approach.** Add an optional retention policy (e.g. `CAIRN_MEMORY_MAX_AGE_DAYS`) that
tombstones unpinned memories below a confidence threshold past an age bound, with pinned memories
always exempt. Surface "expiring soon" in the dashboard so deletion is never silent.

---

### C2. OpenAPI spec + an API-versioning decision — `M` / ⚪

**What.** The 45+ HTTP routes have no machine-readable contract and no version namespace.

**Why.** Routes are hand-registered in `crates/cairn-api/src/lib.rs` (`router()` at `lib.rs:190`),
all under a flat `/api/...` prefix with no `/api/v1`. There is no OpenAPI/Swagger document, so
client SDKs are hand-written and a breaking change has no negotiated upgrade path.

**Suggested approach.** Generate a spec from the handlers (e.g. `utoipa`) and serve it at
`/api/openapi.json`; decide and document a versioning policy (path prefix or a version header in
`/api/health`) before committing to a 1.0 compatibility promise.

---

### C3. Resolve the graph-analysis stubs — `S` / ⚪

**What.** Two CLI subcommands are advertised but unimplemented.

**Why.** `cairn impact` and `cairn callgraph` are stubs in
[`crates/cairn-client/src/extra.rs:54`](../crates/cairn-client/src/extra.rs) — `callgraph` prints
"not yet implemented in v0.5.0" and `impact` redirects users to `cairn graph related`. The module
header (`extra.rs:7`) documents this, and a test (`graph_impact_and_callgraph_are_stubs`,
`extra.rs:353`) pins the stub behavior. They over-promise on the command surface.

**Suggested approach.** Either implement them against the tree-sitter outline data the project
already produces, or remove the subcommands (and the test) until the codebase-graph backing exists,
so the CLI doesn't ship dead-ends.

---

### C4. Broaden the test surface (web + portable E2E) — `M` / ⚪

**What.** Gaps in automated coverage outside the Rust unit suite.

**Why.**
- The dashboard has **no component tests** — [`web/package.json`](../web/package.json) has no
  `test` script, so React regressions (like the v6.1.0 hydration bug) rely on manual QA.
- The end-to-end harness [`scripts/e2e.ps1`](../scripts/e2e.ps1) is PowerShell-only, so it cannot
  run on the Linux CI runners and is effectively never exercised in automation.
- No coverage tool is configured, so cold spots are unknown (qualitative only, per the v6.1.0
  audit).

**Suggested approach.** Add a minimal Vitest + React Testing Library setup covering the highest-risk
components (auth gate, memory list, settings) and wire it into the `web` CI job (ties into A1).
Provide a portable E2E path — port the harness to bash or run the existing scenarios from a
container job — so the 20 documented scenarios actually gate releases. Optionally add
`cargo-llvm-cov` to quantify Rust coverage.

---

## Out of scope / already accepted

These were evaluated during the v6.1.0 audit and are **intentionally deferred** — see the
"Accepted / Deferred Items" table in [`AUDIT_v6.1.0.md`](AUDIT_v6.1.0.md). They are listed here only
so this document doesn't appear to contradict that decision:

- Running `cargo-audit` / `cargo-deny` locally (they run in CI).
- `cargo-llvm-cov` coverage measurement (qualitative coverage documented instead).
- The `local_embeds_*` ONNX test that downloads a ~90 MB model (kept `#[ignore]`).

The one previously-deferred item this document actively **re-raises** is login rate limiting,
expanded into [B2](#b2-rate-limit-the-auth-and-write-surface--m--) above.

---

## Suggested sequencing

1. **Tier A** first — small, safe, and each removes a real failure mode (silent CI gaps, indefinite
   hangs, misleading health). Good single PR.
2. **Tier B** next — operability you'll want before exposing Cairn beyond a trusted LAN.
3. **Tier C** as maturity work ahead of a 1.0, where the API contract and data lifecycle matter most.

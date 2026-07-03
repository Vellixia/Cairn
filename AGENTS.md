You are an AI agent working in the Cairn repo. This file routes you: dev commands and
architecture below, a docs map + task playbooks further down, and your standing tool
preferences in the managed block at the end.

## Dev commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

- CI runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo build --workspace` on every PR via `.github/workflows/ci.yml`. Run the same commands locally before pushing.
- Dependencies use tilde constraints (`~major.minor`) - build with `--locked` to catch drift.
- Run a single crate's tests: `cargo test -p cairn-core` (substitute any crate name).
- `cargo build --workspace` does **not** require the web UI; `crates/cairn-api/build.rs` creates `web/out/` at compile time when missing so the binary falls back to its built-in page.

**Server (requires SurrealDB):**
```sh
docker compose up -d
```

**Web UI (separate from Rust):**
```sh
cd web && npm install && npm run dev   # :3000 -> API on :7777
```

## Local testing (dev + AI verification, NOT in CI)

Two hermetic test buckets live alongside the production code. Both are **local-only**; the
CI does not run them.

### Rust integration tests - `crates/cairn-tests/`

A single workspace member (`cairn-tests`) that hosts `tests/<NN>_<topic>.rs` files - one
integration test binary per file. Every test calls a real Cairn crate function against a
real `Store::open_in_memory()` instance (no network, no live database). The hermetic boundary is
maintained by a per-test in-memory `cairn_store::Store`, exercising every engine without a
running backend.

```sh
cargo test -p cairn-tests                 # 26 files, 204 tests (as of v0.8.0)
cargo test -p cairn-tests --test 18_context_engine  # just one
```

Coverage: memory tiers, followup + gotcha trackers, activity heatmap, architecture
report, **real `MemoryEngine` end-to-end (remember / recall / hybrid_search / consolidate /
crystallize / gotcha promotion)**, **real `ContextEngine` (Full / Cached / Diff / Outline +
anti-inflation + auto-delta fallbacks)**, **real `Assembler` (budget + dropped items)**,
**real `Guard` (verify_edit risk + anchor round-trip + suspicious-anchor prefix)**,
**real `McpServer::dispatch` (remember / recall / assemble / sanitize round-trip)**,
**real `cairn_api` router mounted in-process via `tower::ServiceExt::oneshot`**, shell+profiles,
share sanitization, pack+registry crypto, session persistence, sync CRDTs, proactive intent,
transcript ingest, config env precedence, and workspace invariants.

Add a new flow by dropping a `tests/<NN>_<topic>.rs` file - cargo discovers it. Tests must
exercise a real Cairn crate API, not hand-coded literals or re-implementations of functions
already in the crate.

### Web dashboard flow tests - `web/test/`

The dashboard is driven by an AI agent using the **chrome-devtools** MCP server. No
PowerShell, no agent-browser, no scripted assertions. The agent drives Chrome and asserts
on real DOM state via accessibility snapshots + console messages.

Read `docs/testing/flows.md` for the 13 flow checklists (login, recall, anchor, compression,
tokens, audit, palette, etc.). Read `docs/testing/run-agent-tests.md` for the meta-instruction.

When a flow fails for a real-product reason (a TypeError, a 404, a JSON parse error), write
a finding to `docs/testing/findings/open/<slug>.md` using `docs/_templates/finding-template.md`.
The findings folder is the durable artifact — bugs surface here, they are never silently fixed.

Screenshots land in `web/test/screenshots/<NN>-<flow>/*.png`. The registry of every open and
resolved finding is `docs/testing/findings/README.md`.

**Hard rules:**

- A step that times out, returns no snapshot, or returns an identical-looking screenshot
  to the previous step is a **failure**. Write a finding. Never "PASS" the flow.
- Two findings are confirmed real bugs from previous runs: `/memory/architecture` Next.js
  client-side crash, `/mobile` JSON parse error. Both surface when the agent actually
  inspects the page; they were missed by the old PowerShell harness because URL pattern +
  exit code 0 was the only "assertion".
- **No fake passes.** If you can't confirm, write a finding.

## Architecture

23-crate Rust workspace (MSRV 1.89) + Next.js static-export web UI. Two binaries:

| Binary | Lives in | Purpose |
|--------|----------|---------|
| `cairn-server` (in-container) | Docker image (`cairn-api` bin) | Long-lived server: binds :7777, serves the API + web UI, runs env-only admin bootstrap |
| `cairn` (host) | release tarball (`cairn-client` crate) | Client: `mcp`, `setup`, `onboard`, `doctor`, `hook`, `status`, `reset`, `upgrade` |

**Dep graph:** `cairn-core` -> `cairn-store` -> domain crates (`context`, `memory`, `guard`, `shell`, `profile`, `embed`, `share`, `assemble`, `document`) -> `cairn-mcp` -> `cairn-api`. `cairn-client` is a thin remote-only HTTP wrapper (no local engines).

**Config precedence:** CLI flag > env var > project `.env` > `~/.config/cairn/.env` > built-in default.

**Web UI:** Next.js static export (`output: "export"`), embedded via `rust-embed` in `cairn-api`.

## Docs map for agents

| To... | Go to |
|-------|-------|
| Understand a subsystem | [`docs/reference/architecture.md`](docs/reference/architecture.md) - full crate graph, MCP tools, API endpoints, Docker, config, CLI |
| Learn *why* something is so | [`docs/reference/decisions.md`](docs/reference/decisions.md) - ADR log |
| See product direction / active plan | [`docs/planning/roadmap.md`](docs/planning/roadmap.md) - status; [`docs/planning/plans/`](docs/planning/plans/) - version implementation plans |
| Understand or add tests | [`docs/testing/overview.md`](docs/testing/overview.md) - live-suite coverage; [`docs/testing/live-e2e/`](docs/testing/live-e2e/) - walk docs; [`docs/testing/findings/`](docs/testing/findings/) - bug registry |
| Set up or upgrade an install | [`docs/guides/admin.md`](docs/guides/admin.md), [`docs/guides/upgrading.md`](docs/guides/upgrading.md) |
| **Write a new doc** | [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md) - pick a template from `docs/_templates/` |
| Browse everything | [`docs/README.md`](docs/README.md) - full docs catalog |

## Task playbooks

- **Add a feature:** read `docs/reference/architecture.md` + the active plan in
  `docs/planning/plans/` -> implement -> add tests (`crates/cairn-tests/` or `web/test/`) ->
  append an ADR to `docs/reference/decisions.md` if the design is non-obvious -> update `CHANGELOG.md`.
- **File a dashboard bug:** copy `docs/_templates/finding-template.md` into
  `docs/testing/findings/open/` -> when fixed, move it to `resolved/` and update
  `docs/testing/findings/README.md`.
- **Start a new version's plan:** copy `docs/_templates/plan-template.md` into
  `docs/planning/plans/vX.Y.Z.md`.
- **Write any other doc:** check `docs/CONVENTIONS.md` for which folder and template apply
  before creating the file - don't guess at structure.

## House rules

- Keep PRs focused; one logical change per PR. Match the surrounding style.
- Never hand-edit inside `<!-- BEGIN CAIRN -->` / `<!-- END CAIRN -->` markers (this file,
  `CLAUDE.md`) - they're rewritten by `cairn rules` on every run.
- Doc authoring conventions (folder structure, templates, naming, frontmatter) live in
  [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md) - read it before adding a new doc.

## Runtime prerequisites

- **SurrealDB required.** Set `CAIRN_DB_URL` (default `ws://localhost:8000`) or use `docker compose up -d surreal`.
- **Production:** set `CAIRN_SECRET_KEY` (32+ bytes), `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`.
- **Docker compose:** requires `.env` with a non-default `SURREAL_PASS` (startup guard refuses `root`/`secret`/`changeme`/`password`).

## Key files

- `Cargo.toml` - workspace manifest, dep versions, `[profile.release]` (lto = "thin", strip = true)
- `deny.toml` - cargo-deny config (bans multiple-versions, yanked crates)
- `rust-toolchain.toml` - pins `stable` with `rustfmt` + `clippy` components
- `.mcp.json` - MCP config for OpenCode (Claude Code + Codex use their own configs)
- `.claude/settings.json` - Claude Code lifecycle hooks via `cairn hook`

<!-- BEGIN CAIRN (managed by `cairn rules`) -->
## Cairn --- prefer these tools

You have **Cairn** (MCP server `cairn`): persistent memory, lean context, and edit safety. Use it.

- **Reading code/files:** use `read` instead of your default file read - unchanged re-reads are
  nearly free, and `mode:"signatures"` returns a large file as just its structure (huge token
  saving). Recover any full original with `expand`.
- **Verbose tool output:** run `compress` to shrink cargo/build/log output into a compact view,
  retaining the exact original (recover with `expand`).
- **Memory:** at the start of a task, `wakeup` auto-injects your highest-priority memories so
  you never start cold. Use `recall` (quick) or `search` (hybrid BM25+semantic) to find relevant
  past decisions and context; `remember` decisions, gotchas, and rationale as you make them.
  Record standing user preferences with `prefer`. Call `proactive_recall` at the start of each
  turn to get context automatically injected. Use `assemble` to build a context block under a
  token budget.
- **Before sharing, logging, or committing text:** run `sanitize` to redact secrets/PII.
- **Risky edits:** `checkpoint` before large changes; `verify` a proposed file against its retained
  original to catch silent corruption; `rollback` to undo damage.
- **Stay on task:** keep the current goal in `anchor`.
- **End of session:** run `memory_crystallize` then `consolidate` to promote working notes into
  durable knowledge. Curate with `memory_pin` (keep), `memory_reinforce` (bump confidence),
  `memory_delete` (remove stale). On self-hosted servers use `registry_search` to browse
  the local pack registry.
- **Dashboard is observability-only:** the web UI shows what exists and progress --- you are the one
  who writes, curates, and maintains; humans watch.

Everything Cairn shows is lossless --- the full original is always one `expand` away.
<!-- END CAIRN -->

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

**Server (requires HelixDB):**
```sh
docker compose up -d
```

**Web UI (separate from Rust):**
```sh
cd web && npm install && npm run dev   # :3000 -> API on :7777
```

## Architecture

21-crate Rust workspace (MSRV 1.85) + Next.js static-export web UI. Two binaries:

| Binary | Lives in | Purpose |
|--------|----------|---------|
| `cairn-server` (in-container) | Docker image (`cairn-api` bin) | Long-lived server: binds :7777, serves the API + web UI, runs env-only admin bootstrap |
| `cairn` (host) | release tarball (`cairn-client` crate) | Client: `mcp`, `setup`, `rules`, `run`, `hook`, `remember`, `recall`, `sync`, `pair`, `bench`, `pack`, `graph`, `memory`, `search`, `doctor`, `onboard` |

**Dep graph:** `cairn-core` -> `cairn-store` -> domain crates (`context`, `memory`, `guard`, `shell`, `profile`, `embed`, `share`, `assemble`) -> `cairn-mcp` -> `cairn-api` -> `cairn-client`.

**Config precedence:** CLI flag > env var > project `.env` > `~/.config/cairn/.env` > built-in default.

**Web UI:** Next.js static export (`output: "export"`), embedded via `rust-embed` in `cairn-api`.

## Documentation

> For detailed architecture, MCP tool surface, API endpoints, Docker topology, config reference, and CLI commands, read:
> - [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

| Doc | What |
|-----|------|
| `CONTRIBUTING.md` | Dev setup, PR checklist, workspace layout |
| `docs/ARCHITECTURE.md` | Full crate graph, MCP tools, API endpoints, Docker, config, CLI |
| `docs/DECISIONS.md` | Architecture decision records |
| `docs/TESTING.md` | End-to-end live-suite coverage (20 e2e scenarios; cargo test --workspace reports 330 passed + 5 ignored) |
| `docs/ROADMAP.md` | Development status and phases |
| `docs/BENCHMARKS.md` | Token savings benchmarks |

## Runtime prerequisites

- **HelixDB required.** Set `CAIRN_HELIX_URL` or use `docker compose up -d helix`.
- **Production:** set `CAIRN_SECRET_KEY` (32+ bytes), `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`.
- **Docker compose:** requires `.env` with non-default `MINIO_ROOT_USER` + `MINIO_ROOT_PASSWORD` (startup guard refuses `minioadmin`).

## Key files

- `Cargo.toml` - workspace manifest, dep versions, `[profile.release]` (lto = "thin", strip = true)
- `deny.toml` - cargo-deny config (bans multiple-versions, yanked crates)
- `rust-toolchain.toml` - pins `stable` with `rustfmt` + `clippy` components
- `.mcp.json` - MCP config for OpenCode (Claude Code + Codex use their own configs)
- `.claude/settings.json` - Claude Code lifecycle hooks via `cairn hook`

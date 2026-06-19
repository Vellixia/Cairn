<!-- BEGIN CAIRN (managed by `cairn rules`) -->
## Cairn — prefer these tools

You have **Cairn** (MCP server `cairn`): persistent memory, lean context, and edit safety. Use it.

- **Reading code/files:** use `read` instead of your default file read — unchanged re-reads are
  nearly free, and `mode:"signatures"` returns a large file as just its structure (huge token
  saving). Recover any full original with `expand`.
- **Memory:** at the start of a task, `recall` (or `assemble`) relevant past decisions and context;
  `remember` decisions, gotchas, and rationale as you make them so the next session never starts
  cold. Record standing user preferences with `prefer`.
- **Before sharing, logging, or committing text:** run `sanitize` to redact secrets/PII.
- **Risky edits:** `checkpoint` before large changes; `verify` a proposed file against its retained
  original to catch silent corruption; `rollback` to undo damage.
- **Stay on task:** keep the current goal in `anchor`.

Everything Cairn shows is lossless — the full original is always one `expand` away.
<!-- END CAIRN -->

## Dev commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

- No CI workflows in repo yet — run all three locally before PRs.
- Dependencies use tilde constraints (`~major.minor`) — build with `--locked` to catch drift.
- Run a single crate's tests: `cargo test -p cairn-core` (substitute any crate name).
- `cargo build --workspace` does **not** require the web UI; `web/out/.gitkeep` ships so the binary falls back to a built-in page.

**Server (requires HelixDB):**
```sh
docker compose up -d helix
cargo run -p cairn-server -- serve
```

**Web UI (separate from Rust):**
```sh
cd web && npm install && npm run dev   # :3000 → API on :7777
```

## Architecture

14-crate Rust workspace (MSRV 1.80) + Next.js static-export web UI. Two binaries:

| Binary | Crate | Purpose |
|--------|-------|---------|
| `cairn` | `cairn-server` | Server: `serve`, `token`, `pair-code` |
| `cairn-cli` | `cairn-cli` | Client: `mcp`, `setup`, `hook`, `sync`, `bench` |

**Dep graph:** `cairn-core` → `cairn-store` → domain crates (`context`, `memory`, `guard`, `shell`, `profile`, `embed`, `share`, `assemble`) → `cairn-mcp` → `cairn-api` → `cairn-server` / `cairn-cli`.

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
| `docs/TESTING.md` | End-to-end live-suite coverage (54 integration tests; cargo test --workspace reports 118 passed + 5 ignored) |
| `docs/ROADMAP.md` | Development status and phases |
| `docs/BENCHMARKS.md` | Token savings benchmarks |

## Runtime prerequisites

- **HelixDB required.** Set `CAIRN_HELIX_URL` or use `docker compose up -d helix`.
- **Production:** set `CAIRN_SECRET_KEY` (32+ bytes), `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`.
- **Docker compose:** requires `.env` with non-default `MINIO_ROOT_USER` + `MINIO_ROOT_PASSWORD` (startup guard refuses `minioadmin`).

## Key files

- `Cargo.toml` — workspace manifest, dep versions, `[profile.release]` (lto = "thin", strip = true)
- `deny.toml` — cargo-deny config (bans multiple-versions, yanked crates)
- `rust-toolchain.toml` — pins `stable` with `rustfmt` + `clippy` components
- `.mcp.json` / `.cursor/mcp.json` — MCP config for OpenCode / Cursor
- `.claude/settings.json` — Claude Code lifecycle hooks via `cairn-cli hook`

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

- CI runs the first three (above). No CI workflows in repo yet — run them locally.
- Dependencies use tilde constraints (`~major.minor`) — build with `--locked` to catch version drift.
- `cargo build --workspace` does **not** require the web UI; `web/out/.gitkeep` ships so the binary falls back to a built-in page.

**Run a single crate's tests:** `cargo test -p cairn-core` (substitute any crate name).

**Run the server locally (requires HelixDB):**
```sh
docker compose up -d helix        # or point CAIRN_HELIX_URL at any HelixDB
cargo run -p cairn-server -- serve
```

**Web UI dev (separate from Rust):**
```sh
cd web && npm install && npm run dev   # :3000 → talks to API on :7777
```

## Architecture

14-crate Rust workspace (MSRV 1.80) + Next.js static-export web UI. Two binaries:

| Binary | Crate | Build |
|--------|-------|-------|
| `cairn` | `cairn-server` | `cargo build -p cairn-server` |
| `cairn-cli` | `cairn-cli` | `cargo build -p cairn-cli` |

The dep graph: `cairn-core` → `cairn-store` → domain crates (`context`, `memory`, `guard`, `shell`, `profile`, `embed`, `share`, `assemble`) → `cairn-mcp` → `cairn-api` → `cairn-server` / `cairn-cli`.

Config precedence: CLI flag > env var > project `.env` > `~/.config/cairn/.env` > built-in default.

## Runtime prerequisites

- **HelixDB is required.** Set `CAIRN_HELIX_URL`. Docker compose bundle includes it.
- For production: set `CAIRN_SECRET_KEY` (32+ bytes for JWT signing), `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY` (required when binding non-loopback).
- Docker compose requires `.env` with non-default `MINIO_ROOT_USER` + `MINIO_ROOT_PASSWORD` (startup guard refuses `minioadmin`).

## Key files

- `Cargo.toml` — workspace manifest, dep versions, `[profile.release]` (lto = "thin", strip = true)
- `CONTRIBUTING.md` — dev setup and PR checklist
- `docs/ARCHITECTURE.md` — full crate graph, MCP tool surface, API endpoints, Docker topology, config reference
- `docs/DECISIONS.md` — ADRs
- `deny.toml` — cargo-deny config (bans multiple-versions, yanked crates)
- `rust-toolchain.toml` — pins `stable` with `rustfmt` + `clippy` components
- `web/` — Next.js static export (`output: "export"`), embedded via `rust-embed` in `cairn-api`
- `.mcp.json` / `.cursor/mcp.json` — MCP config for OpenCode / Cursor
- `.claude/settings.json` — Claude Code lifecycle hooks via `cairn-cli hook`

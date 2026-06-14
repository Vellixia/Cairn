# Contributing to Cairn

Thanks for your interest! Cairn is an early-stage, open-source project and contributions are
welcome.

## Development setup

You'll need a recent **Rust** toolchain and **Node 20+** (for the web UI).

```sh
# engine
cargo build --workspace
cargo test --workspace

# run the server (+ embedded UI / built-in fallback page) on http://127.0.0.1:7777
cargo run -p cairn-cli -- serve

# web control plane (dev server on http://localhost:3000, talks to the API on :7777)
cd web && npm install && npm run dev
```

## Before you open a PR

CI runs these — please run them locally first:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep changes focused; one logical change per PR.
- Match the surrounding style. New behavior gets a test.
- The Rust build does **not** require building the web UI — `web/out` ships a `.gitkeep` and the
  binary falls back to the built-in page when no export is present.

## Workspace layout

| Crate | Role |
|---|---|
| `cairn-core` | domain types, hashing, config |
| `cairn-store` | pluggable backend (SQLite today) + content-hash blob store |
| `cairn-context` | cached reads · AST signature outlines · byte-identical `expand` |
| `cairn-memory` | remember · BM25 recall · wakeup · decay · 4-tier consolidation |
| `cairn-assemble` | token-budgeted, edge-ordered context assembler |
| `cairn-guard` | verify edits vs originals · task anchor · checkpoint/rollback |
| `cairn-shell` | RTK-style command-output compression (lossless via `expand`) |
| `cairn-profile` | preference learning |
| `cairn-api` | axum REST API + embedded web UI |
| `cairn-mcp` | MCP server (stdio) |
| `cairn-cli` | the `cairn` binary (serve, mcp, run, hook, install, …) |

## License

By contributing, you agree your contributions are licensed under the project's
[Apache-2.0](LICENSE) license.

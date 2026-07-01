# Contributing to Cairn

Thanks for your interest! Cairn is an early-stage, open-source project and contributions are
welcome.

## Development setup

You'll need a recent **Rust** toolchain (stable, MSRV 1.85) and **Node 20+** (for the web UI).
A running **HelixDB** instance is required for live integration tests - the simplest path is
`docker compose up -d helix`.

```sh
# engine
cargo build --workspace
cargo test --workspace

# run the in-container server (+ embedded UI / built-in fallback page) on http://127.0.0.1:7777
docker compose up -d cairn
# or for local-dev with the host-stdout binary:
cargo run -p cairn-api --bin cairn-server -- serve

# web control plane (dev server on http://localhost:3000, talks to the API on :7777)
cd web && npm install && npm run dev
```

The Rust build does **not** require building the web UI - `crates/cairn-api/build.rs`
creates `web/out/` at compile time when missing, so the binary falls back to a
built-in page when no export is present. Build artifacts are never committed.

## Before you open a PR

CI runs these - please run them locally first:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Keep changes focused; one logical change per PR.
- Match the surrounding style. New behavior gets a test.
- Dependencies use tilde constraints (`~major.minor`); build with `--locked` to catch drift.
- Adding or moving a doc? Read [`docs/CONVENTIONS.md`](docs/CONVENTIONS.md) first - it says
  which folder and template to use.
- CI runs `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo build --workspace` on every PR via `.github/workflows/ci.yml`. Run the same commands locally before pushing.

## Workspace layout

21 crates. Dep graph: `cairn-core` -> `cairn-store` -> domain crates -> `cairn-mcp` -> `cairn-api`
-> `cairn-client`. The in-container server entrypoint lives at `cairn-api::bin::cairn-server`.

| Crate | Role |
|---|---|
| `cairn-core` | domain types, hashing, config, `OrgId` tenant scope |
| `cairn-store` | pluggable backend (HelixDB + in-memory) + content-hash blob store |
| `cairn-context` | cached reads - AST signature outlines - byte-identical `expand` |
| `cairn-memory` | remember - BM25/semantic recall - wakeup - decay - 4-tier consolidation - MMR hybrid |
| `cairn-assemble` | token-budgeted, edge-ordered context assembler with CSP nonce support |
| `cairn-guard` | verify edits vs originals - task anchor - checkpoint/rollback - reliability score |
| `cairn-shell` | RTK-style command-output compression (lossless via `expand`) |
| `cairn-profile` | preference learning |
| `cairn-share` | privacy-first sanitization (redact secrets/PII before sharing) |
| `cairn-session` | on-disk session store (JSON), replay + resume |
| `cairn-pack` | `.cairnpkg` archive format (ustar + SHA-256 + HMAC + Ed25519) |
| `cairn-registry` | Ed25519-signed pack registry - trust scopes - revocation cascade |
| `cairn-sync` | CRDTs (GCounter + ORSet) - vector clocks - E2E encryption (Argon2id + ChaCha20-Poly1305) |
| `cairn-bench` | LongMemEval / horizon / retention benchmarks |
| `cairn-proactive` | intent classifier - auto-inject - opt-out (`PROJECT_OPT_OUT`) |
| `cairn-proxy` | `cairn.sh` reverse proxy - fanout - claim tokens |
| `cairn-ingest` | VTT/SRT/JSON transcript parsers - speaker-window chunking |
| `cairn-embed` | embedding providers (hashing default, ONNX opt-in) |
| `cairn-api` | axum REST API - embedded web UI - PWA service worker - push subscriptions |
| `cairn-mcp` | MCP server (stdio) . 29 tools + 10 graph actions = 39 total . 6 resources . 5 prompts |
| `cairn-client` | the host `cairn` binary (mcp, setup, hook, sync, pair, bench, pack, doctor, ...) |

The two binaries shipped are `cairn-server` (in-container, from `cairn-api`) and `cairn` (host, from `cairn-client`).

## License

By contributing, you agree your contributions are licensed under the project's
[Apache-2.0](LICENSE) license.

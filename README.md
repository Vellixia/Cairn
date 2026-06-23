<div align="center">

<img src="assets/cairn-logo.svg" alt="Cairn" width="116" />

# Cairn

### The open-source context & reliability layer for AI agents

**Make any model smart.** Remember everything · feed less, not more · stay reliable on long
tasks · get smarter together — self-hosted, with no context ever lost.

</div>

---

> A cairn is a stack of trail-marker stones. Travelers each add a stone, and everyone who follows
> benefits. Each coding session leaves a marker the next one follows (**memory**); a cairn is
> minimal — only the stones you need to navigate (**lean, no-loss context**).

Cairn sits between your AI coding agents (Claude Code, OpenCode, Cursor, …) and your code.
It runs as one small server you self-host once, and every device + agent connects to it through a
single MCP endpoint plus lifecycle hooks.

```mermaid
flowchart LR
    subgraph Agents["Your agents"]
        CC["Claude Code"]
        OC["OpenCode"]
        CUR["Cursor"]
        VS["VS Code"]
        WS["Windsurf"]
    end

    CLI["cairn-cli<br/>MCP + hooks + setup"]
    Server["cairn server<br/>REST API + web UI"]
    Store["HelixDB<br/>graph + vectors<br/>+ blob store"]

    Agents -->|"MCP stdio"| CLI
    CLI -->|"local or<br/>remote proxy"| Server
    Server --> Store

    CLI -->|"remember / recall<br/>read / expand<br/>verify / checkpoint"| Store
```

## Why

AI agents fail on long, multi-session work in ways bigger context windows don't fix:

- They **forget everything** between sessions.
- They **re-read files** they already read, burning tokens.
- Quality **decays over long tasks** (context rot, reasoning drift, silent corruption).
- Memory is **siloed** per machine and per tool.

The bottleneck usually isn't the model's IQ — it's the **context fed to it** and the **drift over
time**. Cairn fixes that.

## Features

### Memory

- **Cross-session recall** — decisions, findings, and rationale from last week are visible today, ranked by confidence × relevance.
- **4-tier memory** — working → episodic → semantic → procedural. Memories consolidate and crystallize over time.
- **Provenance graph** — every memory tracks `derived_from`, `contradicts`, `supersedes`, and `applies_to` edges. The dashboard renders the full graph.
- **Confidence reinforcement** — the agentmemory curve `c' = min(1.0, c + 0.1*(1-c))` on every successful recall. Pinned memories bypass decay.
- **Proactive recall** — an intent classifier runs before each agent turn and auto-injects up to 3 relevant memories when the prompt has recall cues. Per-project opt-out.
- **Hybrid search** — BM25 lexical + semantic vector recall fused via Reciprocal Rank Fusion, with MMR diversity reranking (λ=0.7).
- **Multi-tenant** — every memory carries an `OrgId`. Tenant isolation enforced before any ranking work. Single-tenant installs see no change.

### Context compression

- **AST-aware reads** — tree-sitter outlines for 11 languages (rust, python, javascript, typescript, go, c, cpp, java, c#, ruby, bash). A 3,200-token file becomes ~210 tokens. The full original is one `expand` away.
- **Cache-aware re-reads** — unchanged files cost ~19 tokens (just the handle). No context ever lost.
- **Shell compression** — verbose command output (153 lines) compresses to 1 line, fully recoverable.
- **Token-budget assembly** — edge-ordered context assembly under a budget. Anti-context-rot.

### Reliability

- **Edit verification** — compares proposed edits against the retained original. Flags large unreplaced deletions (silent corruption).
- **Checkpoint / rollback** — snapshot tracked files before risky edits. One command undoes damage.
- **Task anchor** — the current goal is re-injected at session start so the model doesn't drift.
- **Drift detection** — sessions record checkpoints; the dashboard surfaces drift for review + approval.
- **HMAC-signed savings ledger** — every context assembly is signed. `/api/ledger/verify` detects tampering.

### Collaboration

- **`.cairnpkg` format** — share memory packs as signed tarballs. Ed25519 signatures. Per-file SHA-256 integrity.
- **Self-hosted registry** — publish, search, install, revoke packs via `/registry/*` HTTP endpoints. Trust scopes (Local / Team / Public).
- **Federation** — pull-based revocation propagation. Offline-first CRDT sync (GCounter + ORSet + vector clocks).
- **E2E encrypted sync** — Argon2id → ChaCha20-Poly1305 AEAD. The server never sees plaintext.

### Platforms

- **PWA** — service worker for offline dashboard. Push notifications for drift events.
- **Transcript ingestion** — VTT / SRT / JSON parsers. Chunk by speaker + time window. Each chunk becomes a memory.
- **Browser extension capture** — `POST /api/extensions/capture` turns any selection into a Cairn memory.
- **Mobile companion** — `/mobile` PWA with biometric gate, savings card, drift-approval queue.

## Proof

Run **`cairn-cli bench`** on your own repo. Measured on Cairn's own `crates/` (25 files):

| Mechanism | Before | After | Saved |
|---|---|---|---|
| AST outline reads | ~59,052 tok | ~5,894 tok | **90%** |
| Re-reading an unchanged file | ~6,506 tok | ~19 tok | **99.7%** |
| Shell output (verbose test log) | 153 lines | 1 line | **99%** |

All lossless — the full original is retained and one `expand` away. See [Benchmarks](docs/BENCHMARKS.md).

## Getting started

### 1. Install

```sh
# macOS / Linux — one-liner (recommended)
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.ps1 | iex
```

```sh
# Docker — the full stack (Cairn + HelixDB + MinIO), the easiest path
cp .env.example .env          # set MinIO credentials (see .env.example)
docker compose up -d          # builds Cairn, pulls HelixDB + MinIO, wires them together
# → http://localhost:7777
```

```sh
# From source
cargo install --git https://github.com/Vellixia/Cairn cairn-server cairn-cli
```

### 2. Start the server

Cairn stores data in **HelixDB** — `docker compose` starts one for you, or point
`CAIRN_HELIX_URL` at an existing server. Then run `cairn serve` and open
<http://127.0.0.1:7777>.

### 3. Connect an agent

```sh
cairn-cli setup --all         # auto-detect every installed agent and wire up MCP
# or target one:
cairn-cli setup opencode --server http://localhost:7777 --token <token>
```

Supports Claude Code (MCP + lifecycle hooks), OpenCode, Cursor, VS Code, Windsurf.
See [Architecture — Connecting an agent](docs/ARCHITECTURE.md#connecting-an-agent-by-hand) for manual setup.

### 4. Verify

```sh
cairn-cli doctor              # checks the local setup
cairn-cli remember "we use rust and helixdb"
cairn-cli recall "rust"       # should return the memory you just saved
```

## OpenCode quickstart

OpenCode is a first-class citizen — `cairn-cli` is one of its built-in providers.
The fastest path from `git clone` to a Cairn-aware session:

```sh
# 1. Install the CLI
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# 2. Start the server stack
docker compose up -d                     # HelixDB + MinIO + Cairn on :7777

# 3. Wire OpenCode (creates ~/.config/opencode/opencode.json with the MCP entry)
cairn-cli setup opencode --server http://localhost:7777

# 4. Generate a token (one-time, copy it)
cairn-cli token create --name laptop --scope write

# 5. Restart OpenCode so the MCP entry picks up. You'll see `cairn` in the tool list.
```

After that, OpenCode's tool palette includes `cairn_recall`, `cairn_remember`, `cairn_read`,
`cairn_verify`, `cairn_assemble`, `proactive_recall`, `memory_graph`, `memory_crystallize`,
`search`, `metrics`, and 30+ more — see [MCP tools](docs/ARCHITECTURE.md#mcp-tool-surface).

### What Cairn gives OpenCode out of the box

- **Cross-session memory.** Decisions from last week are visible today via `cairn_recall`
  at session start, ranked by `confidence × applies_to`.
- **Lean file reads.** `cairn_read` returns the AST outline (~90% smaller than the full
  file); the original is one `cairn_expand` away. No context lost.
- **Drift detection.** Each session records checkpoints; `cairn-cli doctor --fix`
  re-anchors the model on long tasks.
- **One-line rules.** `cairn-cli prefer always use ripgrep` becomes a memory that
  re-fires on every session until contradicted.
- **Proactive recall.** The intent classifier fires before each turn — if the prompt
  has recall cues, relevant memories are auto-injected. No manual `recall` needed.

## Status

🚧 Active development — v0.5.0 is feature-complete. See [Roadmap](docs/ROADMAP.md) for
what's done and what's next.

## Documentation

| Doc | Description |
|---|---|
| [Architecture](docs/ARCHITECTURE.md) | Crate graph, MCP tools, API endpoints, Docker, config, CLI commands |
| [Plan v0.5.0](docs/PLAN_v0.5.0.md) | 23-sprint plan, success metrics, risks |
| [Roadmap](docs/ROADMAP.md) | Development status — done, in progress, next |
| [Benchmarks](docs/BENCHMARKS.md) | Token savings methodology + measured results |
| [Decisions](docs/DECISIONS.md) | 26 ADRs covering every architecture decision |
| [Security](docs/SECURITY.md) | Threat model + hardening checklist |
| [E2E Tests](docs/E2E.md) | 20-scenario end-to-end test harness |
| [Changelog](CHANGELOG.md) | Release notes for every version |
| [Contributing](CONTRIBUTING.md) | Dev setup, PR checklist, workspace layout |

## License

Apache-2.0. See [LICENSE](LICENSE).
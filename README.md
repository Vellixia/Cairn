<div align="center">

<img src="assets/cairn-logo.svg" alt="Cairn" width="116" />

# Cairn

### The open-source context & reliability layer for AI agents

**Make any model smart.** Remember everything Â· feed less, not more Â· stay reliable on long
tasks Â· get smarter together â€” self-hosted, with no context ever lost.

</div>

---

> A cairn is a stack of trail-marker stones. Travelers each add a stone, and everyone who follows
> benefits. Each coding session leaves a marker the next one follows (**memory**); a cairn is
> minimal â€” only the stones you need to navigate (**lean, no-loss context**).

Cairn sits between your AI coding agents (Claude Code, Codex CLI, OpenCode) and your code.
It runs as one small server you self-host once via Docker, and every device + agent connects to
it through a single MCP endpoint plus lifecycle hooks.

```mermaid
flowchart LR
    subgraph Agents["Your agents"]
        CC["Claude Code"]
        CX["Codex CLI"]
        OC["OpenCode"]
    end

    CLI["cairn<br/>MCP + hooks + setup"]
    Server["cairn-server<br/>in-container<br/>REST API + web UI"]
    Store["HelixDB<br/>graph + vectors<br/>+ blob store"]

    Agents -->|"MCP stdio"| CLI
    CLI -->|"HTTP"| Server
    Server --> Store

    CLI -->|"remember / recall<br/>read / expand<br/>verify / checkpoint"| Server
```

## Why

AI agents fail on long, multi-session work in ways bigger context windows don't fix:

- They **forget everything** between sessions.
- They **re-read files** they already read, burning tokens.
- Quality **decays over long tasks** (context rot, reasoning drift, silent corruption).
- Memory is **siloed** per machine and per tool.

The bottleneck usually isn't the model's IQ â€” it's the **context fed to it** and the **drift over
time**. Cairn fixes that.

## Features

### Memory

- **Cross-session recall** â€” decisions, findings, and rationale from last week are visible today, ranked by confidence Ã— relevance.
- **4-tier memory** â€” working â†’ episodic â†’ semantic â†’ procedural. Memories consolidate and crystallize over time.
- **Provenance graph** â€” every memory tracks `derived_from`, `contradicts`, `supersedes`, and `applies_to` edges. The dashboard renders the full graph.
- **Confidence reinforcement** â€” the agentmemory curve `c' = min(1.0, c + 0.1*(1-c))` on every successful recall. Pinned memories bypass decay.
- **Proactive recall** â€” an intent classifier runs before each agent turn and auto-injects up to 3 relevant memories when the prompt has recall cues. Per-project opt-out.
- **Hybrid search** â€” BM25 lexical + semantic vector recall fused via Reciprocal Rank Fusion, with MMR diversity reranking (Î»=0.7).
- **Multi-tenant** â€” every memory carries an `OrgId`. Tenant isolation enforced before any ranking work. Single-tenant installs see no change.

### Context compression

- **AST-aware reads** â€” tree-sitter outlines for 11 languages (rust, python, javascript, typescript, go, c, cpp, java, c#, ruby, bash). A 3,200-token file becomes ~210 tokens. The full original is one `expand` away.
- **Cache-aware re-reads** â€” unchanged files cost ~19 tokens (just the handle). No context ever lost.
- **Shell compression** â€” verbose command output (153 lines) compresses to 1 line, fully recoverable.
- **Token-budget assembly** â€” edge-ordered context assembly under a budget. Anti-context-rot.

### Reliability

- **Edit verification** â€” compares proposed edits against the retained original. Flags large unreplaced deletions (silent corruption).
- **Checkpoint / rollback** â€” snapshot tracked files before risky edits. One command undoes damage.
- **Task anchor** â€” the current goal is re-injected at session start so the model doesn't drift.
- **Drift detection** â€” sessions record checkpoints; the dashboard surfaces drift for review + approval.
- **HMAC-signed savings ledger** â€” every context assembly is signed. `/api/ledger/verify` detects tampering.

### Collaboration

- **`.cairnpkg` format** â€” share memory packs as signed tarballs. Ed25519 signatures. Per-file SHA-256 integrity.
- **Self-hosted registry** â€” publish, search, install, revoke packs via `/registry/*` HTTP endpoints. Trust scopes (Local / Team / Public).
- **Federation** â€” pull-based revocation propagation. Offline-first CRDT sync (GCounter + ORSet + vector clocks).
- **E2E encrypted sync** â€” Argon2id â†’ ChaCha20-Poly1305 AEAD. The server never sees plaintext.

### Platforms

- **PWA** â€” service worker for offline dashboard. Push notifications for drift events.
- **Transcript ingestion** â€” VTT / SRT / JSON parsers. Chunk by speaker + time window. Each chunk becomes a memory.
- **Browser extension capture** â€” `POST /api/extensions/capture` turns any selection into a Cairn memory.
- **Mobile companion** â€” `/mobile` PWA with biometric gate, savings card, drift-approval queue.

## Proof

Run **`cairn bench`** on your own repo. Measured on Cairn's own `crates/` (25 files):

| Mechanism | Before | After | Saved |
|---|---|---|---|
| AST outline reads | ~59,052 tok | ~5,894 tok | **90%** |
| Re-reading an unchanged file | ~6,506 tok | ~19 tok | **99.7%** |
| Shell output (verbose test log) | 153 lines | 1 line | **99%** |

All lossless â€” the full original is retained and one `expand` away. See [Benchmarks](docs/BENCHMARKS.md).

## Getting started

### 1. Install

```sh
# macOS / Linux â€” one-liner (recommended)
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.ps1 | iex
```

```sh
# Docker â€” the full stack (Cairn + HelixDB + MinIO), the easiest path
cp .env.example .env          # set MinIO + admin credentials (see .env.example)
docker compose up -d          # builds Cairn, pulls HelixDB + MinIO, wires them together
# â†’ http://localhost:7777
# First-boot admin is bootstrapped from CAIRN_ADMIN_USERNAME + CAIRN_ADMIN_PASSWORD.
# Comment out CAIRN_ADMIN_PASSWORD to fall back to the /setup wizard on first visit.
```

```sh
# From source
cargo install --git https://github.com/Vellixia/Cairn cairn-server cairn
```

### 2. Start the server

`docker compose up -d` brings up Cairn + HelixDB + MinIO. The admin record
is bootstrapped from `CAIRN_ADMIN_USERNAME` + `CAIRN_ADMIN_PASSWORD` in
`.env` on first boot. See [docs/ADMIN.md](docs/ADMIN.md) for the full
admin surface (mint tokens, pair codes, password rotation).

### 3. Connect an agent

```sh
cairn setup --all         # auto-detect every installed agent and wire up MCP
# or target one:
cairn setup opencode --server http://localhost:7777 --token <token>
```

Supports Claude Code (MCP + lifecycle hooks), Codex CLI, and OpenCode.
See [Architecture — Connecting an agent](docs/ARCHITECTURE.md#connecting-an-agent-by-hand) for manual setup.

### 4. Verify

```sh
cairn doctor              # checks the local setup
cairn remember "we use rust and helixdb"
cairn recall "rust"       # should return the memory you just saved
```

## OpenCode quickstart

OpenCode is a first-class citizen â€” `cairn` is one of its built-in providers.
The fastest path from `git clone` to a Cairn-aware session:

```sh
# 1. Install the CLI
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# 2. Start the server stack
docker compose up -d                     # HelixDB + MinIO + Cairn on :7777

# 3. Wire OpenCode (creates ~/.config/opencode/opencode.json with the MCP entry)
cairn setup opencode --server http://localhost:7777

# 4. Generate a token (one-time, copy it)
cairn token create --name laptop --scope write

# 5. Restart OpenCode so the MCP entry picks up. You'll see `cairn` in the tool list.
```

After that, OpenCode's tool palette includes `cairn_recall`, `cairn_remember`, `cairn_read`,
`cairn_verify`, `cairn_assemble`, `proactive_recall`, `memory_graph`, `memory_crystallize`,
`search`, `metrics`, and 30+ more â€” see [MCP tools](docs/ARCHITECTURE.md#mcp-tool-surface).

### What Cairn gives OpenCode out of the box

- **Cross-session memory.** Decisions from last week are visible today via `cairn_recall`
  at session start, ranked by `confidence Ã— applies_to`.
- **Lean file reads.** `cairn_read` returns the AST outline (~90% smaller than the full
  file); the original is one `cairn_expand` away. No context lost.
- **Drift detection.** Each session records checkpoints; `cairn doctor --fix`
  re-anchors the model on long tasks.
- **One-line rules.** `cairn prefer always use ripgrep` becomes a memory that
  re-fires on every session until contradicted.
- **Proactive recall.** The intent classifier fires before each turn â€” if the prompt
  has recall cues, relevant memories are auto-injected. No manual `recall` needed.

## Status

ðŸš§ Active development â€” v0.5.0 is feature-complete. See [Roadmap](docs/ROADMAP.md) for
what's done and what's next.

## Documentation

| Doc | Description |
|---|---|
| [Architecture](docs/ARCHITECTURE.md) | Crate graph, MCP tools, API endpoints, Docker, config, CLI commands |
| [Plan v0.5.0](docs/PLAN_v0.5.0.md) | 23-sprint plan, success metrics, risks |
| [Roadmap](docs/ROADMAP.md) | Development status â€” done, in progress, next |
| [Benchmarks](docs/BENCHMARKS.md) | Token savings methodology + measured results |
| [Decisions](docs/DECISIONS.md) | 26 ADRs covering every architecture decision |
| [Security](docs/SECURITY.md) | Threat model + hardening checklist |
| [E2E Tests](docs/E2E.md) | 20-scenario end-to-end test harness |
| [Changelog](CHANGELOG.md) | Release notes for every version |
| [Contributing](CONTRIBUTING.md) | Dev setup, PR checklist, workspace layout |

## License

Apache-2.0. See [LICENSE](LICENSE).
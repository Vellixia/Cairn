<div align="center">

<img src="assets/cairn-logo.svg" alt="Cairn" width="116" />

# Cairn

### The open-source context & reliability layer for AI agents

**Make any model smart.** Remember everything - feed less, not more - stay reliable on long
tasks - get smarter together - self-hosted, with no context ever lost.

</div>

---

> A cairn is a stack of trail-marker stones. Travelers each add a stone, and everyone who follows
> benefits. Each coding session leaves a marker the next one follows (**memory**); a cairn is
> minimal - only the stones you need to navigate (**lean, no-loss context**).

Cairn sits between your AI coding agents (Claude Code, Codex CLI, OpenCode) and your code.
It runs as one small server you self-host once via Docker, and every device + agent connects to
it through a single MCP endpoint plus lifecycle hooks.

> **Pre-1.0 software.** Cairn has not shipped a 1.0. The current release is **v0.8.x** and is
> **under active development** - interfaces (MCP tool names/shapes, HTTP API, CLI flags, config
> keys, hook payloads) may change between minor versions without prior notice, and there is no
> stability guarantee until 1.0 lands. It is usable today (the project dogfoods itself - every
> Cairn agent session runs against a live Cairn server), but if you adopt it for production work,
> expect breaking changes on the upgrade path and pin the version you install. See
> [Roadmap](docs/planning/roadmap.md) for what's done and what's next toward 1.0.

## Architecture Flow

```mermaid
flowchart TB
    subgraph Host["Host machine (your laptop)"]
        subgraph Agents["Your agents"]
            CC["Claude Code"]
            CX["Codex CLI"]
            OC["OpenCode"]
        end
        CLI["cairn (host binary)<br/>RemoteProxy + hooks<br/>setup / doctor / status"]
    end

    subgraph Container["Docker container"]
        Server["cairn-server<br/>REST API + web UI<br/>shared ContextEngine<br/>+ MemoryEngine + Guard"]
        Store["cairn-store<br/>SurrealDB backend"]
    end

    subgraph Backend["SurrealDB"]
        Graph["graph + HNSW vectors<br/>+ blob store<br/>+ cron scheduler"]
    end

    Agents -->|"MCP stdio"| CLI
    CLI -->|"HTTP + JWT<br/>X-Cairn-Project header"| Server
    Server --> Store
    Store --> Graph
```

### 24-crate workspace

```mermaid
graph BT
    core["cairn-core<br/>types, config, OrgId, errors"]
    store["cairn-store<br/>SurrealDB + BlobStore"]
    context["cairn-context<br/>read, cache, AST, diff"]
    memory["cairn-memory<br/>4-tier, RRF, MMR, graph"]
    guard["cairn-guard<br/>verify, anchor, checkpoint"]
    shell["cairn-shell<br/>compress, recover"]
    profile["cairn-profile<br/>preferences"]
    assemble["cairn-assemble<br/>token-budget assembly"]
    share["cairn-share<br/>sanitization"]
    embed["cairn-embed<br/>embeddings"]
    document["cairn-document<br/>RAG chunking"]
    rerank["cairn-rerank<br/>cross-encoder rerank"]
    session["cairn-session<br/>CCP + drift log"]
    pack["cairn-pack<br/>.cairnpkg + Ed25519"]
    registry["cairn-registry<br/>pack registry"]
    sync["cairn-sync<br/>CRDTs + E2E"]
    bench["cairn-bench<br/>LongMemEval + horizon"]
    proactive["cairn-proactive<br/>intent classifier"]
    proxy["cairn-proxy<br/>reverse proxy"]
    ingest["cairn-ingest<br/>transcript parsers"]
    mcp["cairn-mcp<br/>32 MCP tools"]
    api["cairn-api<br/>REST API + web UI"]
    client["cairn-client<br/>host binary"]
    tests["cairn-tests<br/>27 integration test files"]

    core --> store
    store --> context
    store --> memory
    store --> guard
    store --> shell
    store --> session
    memory --> assemble
    context --> mcp
    memory --> mcp
    guard --> mcp
    assemble --> mcp
    proactive --> mcp
    mcp --> api
    api --> client
    mcp --> client
```

**Data flow:** Agent calls `cairn_read` via MCP stdio → `RemoteProxy` forwards to
`/api/tools/call` → `McpServer::from_engines()` dispatches through `AppState`'s shared
`Arc<ContextEngine>` (mtime cache persists) → `ContextEngine::read()` returns
`Cached`/`Diff`/`Full` → file version recorded as edit baseline → result returned to agent.

**Scope model:** Every memory carries `scope_type` (Global/Project/Session). The
`X-Cairn-Project` header (auto-detected from git remote/root) scopes reads and writes.
Project memories promote to Global via the `llm-intelligence` cron job's scoring pipeline.

## Workflow — Session Lifecycle

```mermaid
sequenceDiagram
    participant Agent
    participant Hook as cairn hook
    participant Server as cairn-server
    participant Store as SurrealDB

    Note over Agent,Store: SessionStart
    Agent->>Hook: SessionStart {cwd, session_id}
    Hook->>Server: GET /api/memory/wakeup (top 12)
    Hook->>Server: GET /api/memory/autopilot-digest
    Hook->>Server: PATCH /api/projects/upsert (auto-detect project)
    Hook-->>Agent: context injection (wakeup + anchor + autopilot)

    Note over Agent,Store: Agent reads a file
    Agent->>Server: read(path, mode=auto) via MCP
    Server->>Store: ContextEngine::read (mtime cache)
    Store-->>Server: Cached (~19 tok) | Diff | Full
    Server->>Store: record_file_version (edit baseline)
    Server-->>Agent: ReadResult (handle + view)

    Note over Agent,Store: Agent edits a file
    Agent->>Server: verify(path, new_content) via MCP
    Server->>Store: Guard::verify_edit (diff vs original)
    Store-->>Server: VerifyReport (ok / warn / danger)
    Server-->>Agent: risk assessment

    Note over Agent,Store: PostToolUse (after edit lands)
    Agent->>Hook: PostToolUse {tool_name: Edit, tool_input}
    Hook->>Server: POST /api/guard/verify-baseline (corruption check)
    Server->>Store: Guard::verify_against_baseline (vs read-time version)
    Store-->>Server: None (clean) | VerifyReport (corruption detected)

    Note over Agent,Store: UserPromptSubmit
    Agent->>Hook: UserPromptSubmit {prompt}
    Hook->>Server: POST /api/guard/anchor/auto (derive anchor)
    Hook->>Server: POST /api/memory (save prompt, scope=project)
    Hook-->>Agent: proactive_recall (up to 3 memories if intent matches)

    Note over Agent,Store: SessionEnd / PreCompact
    Agent->>Hook: SessionEnd
    Hook->>Server: POST /api/memory/consolidate (tier promotion)
    Hook->>Server: POST /api/memory/session-summary (LLM synthesis)
    Hook->>Server: POST /api/memory (touched files list, scope=session)
```

### What happens at each step

| Event | What Cairn does |
|---|---|
| **SessionStart** | Injects wakeup memories + task anchor + autopilot digest. Auto-detects project from git remote/root, registers it via `PATCH /api/projects/upsert`. Spools offline if server unreachable. |
| **Agent reads** | `read` checks mtime cache → `Cached` (~19 tok if unchanged), `Diff` (only changed lines), or `Full`. Records file version as edit baseline. AST outlines for 11 languages (`signatures`/`map` mode). Original retained in blob store — `expand` recovers it byte-identical. |
| **Agent edits** | `verify` compares proposed content vs on-disk original. Flags large unreplaced deletions (silent corruption). Retains pre-edit version in blob store. |
| **PostToolUse** | `verify_baseline` compares current file vs the version recorded at read time. Catches corruption introduced *after* the read but *before* the next read. Fire-and-forget (spooled). |
| **UserPromptSubmit** | Auto-derives task anchor from first substantive prompt. Saves prompt as project-scoped episodic memory. Proactive recall classifier auto-injects up to 3 relevant memories. |
| **SessionEnd** | Consolidates memory across tiers (working → episodic → semantic → procedural). LLM synthesizes session summary (if enabled). Flushes touched-files list as session-scoped memory. |
| **Cron (nightly)** | `llm-intelligence` job: concept extraction, contradiction detection, promotion scoring, auto-promote Project→Global, auto-demote stale Global→Project, session GC, memory decay. Self-tuning promotion threshold. |

## Why

AI agents fail on long, multi-session work in ways bigger context windows don't fix:

- They **forget everything** between sessions.
- They **re-read files** they already read, burning tokens.
- Quality **decays over long tasks** (context rot, reasoning drift, silent corruption).
- Memory is **siloed** per machine and per tool.

The bottleneck usually isn't the model's IQ - it's the **context fed to it** and the **drift over
time**. Cairn fixes that.

## Features

### Memory

- **Cross-session recall** - decisions, findings, and rationale from last week are visible today, ranked by confidence x relevance.
- **4-tier memory** - working -> episodic -> semantic -> procedural. Memories consolidate and crystallize over time. Working-tier overflow auto-pages to episodic instead of deleting.
- **Project→Global scoping** - memories default to Project scope when a project is detected (via `X-Cairn-Project` header from git remote/root). The `llm-intelligence` cron job scores Project memories and auto-promotes worthy ones to Global. Demote reverts stale Global memories back to their origin project.
- **Provenance graph** - 6 edge types: `derived_from`, `contradicts`, `supersedes`, `applies_to` (file relevance), `related_to` (shared concepts, auto-derived), `depends_on` (import analysis, auto-derived). The dashboard renders the full graph.
- **Confidence reinforcement** - the agentmemory curve `c' = min(1.0, c + 0.1*(1-c))` on every successful recall. Pinned memories bypass decay.
- **Proactive recall** - an intent classifier runs before each agent turn and auto-injects up to 3 relevant memories when the prompt has recall cues. Per-project opt-out.
- **Hybrid search** - BM25 lexical + semantic vector recall fused via Reciprocal Rank Fusion, with MMR diversity reranking (λ>=0.7). Optional LLM query expansion.
- **Document RAG** - `document_ingest` + `document_search` MCP tools. Paragraph-aware chunking, project-scoped document store, blended into `assemble` context blocks.
- **Cross-project corruption guard** - consolidation and contradiction auto-resolve refuse to supersede memories from different project scopes.
- **Multi-tenant** - every memory carries an `OrgId`. Tenant isolation enforced before any ranking work. Single-tenant installs see no change.

### Context compression

- **AST-aware reads** - tree-sitter outlines for 11 languages (rust, python, javascript, typescript, go, c, cpp, java, c#, ruby, bash). A 3,200-token file becomes ~210 tokens. The full original is one `expand` away.
- **Cache-aware re-reads** - unchanged files cost ~19 tokens (just the handle). No context ever lost. Works on both the server path (shared `ContextEngine`) and the proxy path (`LocalReader` with mtime cache + diff).
- **Stateful proxy reads** - `RemoteProxy`'s `LocalReader` tracks file mtime, returns `Cached` for unchanged files, `Diff` for changed files (with 60% auto-delta fallback to `Full` when the diff would be noisier than the original).
- **Shared engines** - `/api/tools/call` shares `AppState`'s long-lived `ContextEngine`, `Guard`, and `MemoryEngine` across all tool calls. The read cache, file version baselines, and guard workspace scoping persist.
- **Shell compression** - verbose command output (153 lines) compresses to 1 line, fully recoverable.
- **Token-budget assembly** - edge-ordered context assembly under a budget. Merges memories + document chunks. Anti-context-rot.

### Reliability

- **Edit verification** - `verify` compares proposed edits against the retained original. Flags large unreplaced deletions (silent corruption).
- **Post-edit corruption detection** - `verify_baseline` compares the current file against the version Cairn recorded when the agent last read it. The `PostToolUse` hook fires it automatically for every edited file.
- **Checkpoint / rollback** - snapshot tracked files before risky edits. One command undoes damage. Checkpoints see the same file versions that reads recorded (shared engine state).
- **Task anchor** - the current goal is re-injected at session start so the model doesn't drift. Auto-derived from the first substantive prompt.
- **Drift detection** - sessions record checkpoints; the dashboard surfaces drift for review + autopilot approval. Filter by status (`?status=pending`).
- **HMAC-signed savings ledger** - every context assembly is signed. `/api/ledger/verify` detects tampering.

### Collaboration

- **`.cairnpkg` format** - share memory packs as signed tarballs. Ed25519 signatures. Per-file SHA-256 integrity.
- **Self-hosted registry** - publish, search, install, revoke packs via `/api/registry/*` HTTP endpoints. Trust scopes (Local / Team / Public).
- **Federation** - pull-based revocation propagation. Offline-first CRDT sync (GCounter + ORSet + vector clocks).
- **E2E encrypted sync** - Argon2id -> ChaCha20-Poly1305 AEAD. The server never sees plaintext.

### Platforms

- **PWA** - service worker for offline dashboard. Push notifications for drift events.
- **Transcript ingestion** - VTT / SRT / JSON parsers. Chunk by speaker + time window. Each chunk becomes a memory.
- **Browser extension capture** - `POST /api/extensions/capture` turns any selection into a Cairn memory.
- **Mobile companion** - `/mobile` PWA with biometric gate, savings card, drift-approval queue.

## Proof

Measured on Cairn's own `crates/` (24 crates, 27 integration test files):

| Mechanism | Before | After | Saved |
|---|---|---|---|
| AST outline reads | ~59,052 tok | ~5,894 tok | **90%** |
| Re-reading an unchanged file | ~6,506 tok | ~19 tok | **99.7%** |
| Shell output (verbose test log) | 153 lines | 1 line | **99%** |

All lossless - the full original is retained and one `expand` away. See [Benchmarks](docs/testing/benchmarks.md).

## Getting started

### 1. Install

```sh
# macOS / Linux - one-liner (recommended)
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.ps1 | iex
```

```sh
# Docker - the full stack (Cairn + SurrealDB), the easiest path
cp .env.example .env          # set database + admin credentials (see .env.example)
docker compose up -d          # builds Cairn, pulls SurrealDB, wires them together
# -> http://localhost:7777
# First-boot admin is bootstrapped from CAIRN_ADMIN_USERNAME + CAIRN_ADMIN_PASSWORD.
# Comment out CAIRN_ADMIN_PASSWORD to fall back to the /setup wizard on first visit.
```

```sh
# From source (host binary only - the in-container server bin ships in the Docker image)
cargo install --git https://github.com/Vellixia/Cairn cairn
```

### 2. Start the server

`docker compose up -d` brings up Cairn + SurrealDB. The admin record
is bootstrapped from `CAIRN_ADMIN_USERNAME` + `CAIRN_ADMIN_PASSWORD` in
`.env` on first boot. See [docs/guides/admin.md](docs/guides/admin.md) for the full
admin surface (mint tokens, password rotation).

### 3. Connect an agent

Fastest path: log in to the dashboard, open **You -> Tokens**, and click "Mint token" to get a
JWT. Then run:

```sh
cairn setup --all --server <url> --token <jwt>   # writes ~/.cairn/config.toml, wires detected agents
```

Against a local dev server, `--server` can be omitted (`cairn setup --all` probes
`localhost:7777` automatically) - you still need `--token`:

```sh
cairn setup --all --token <jwt>
```

Or wire agents by hand:

```sh
cairn setup --all         # auto-detect every installed agent and wire up MCP
# or target one:
cairn setup opencode --server http://localhost:7777 --token <token>
```

Supports Claude Code (MCP + lifecycle hooks + an on-demand `cairn` skill), Codex CLI, and
OpenCode. See [Architecture - Connecting an agent](docs/reference/architecture.md#connecting-an-agent-by-hand)
for manual setup.

### 4. Verify

```sh
cairn doctor --json | jq   # checks server connectivity + agent config, machine-readable
cairn status               # shows server, token, and agent status, with their sources
cairn statusline           # one ambient line: memories, anchor, tokens saved, offline backlog
```

## OpenCode quickstart

OpenCode is a first-class citizen - `cairn` is one of its built-in providers.
The fastest path from `git clone` to a Cairn-aware session:

```sh
# 1. Install the CLI
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# 2. Start the server stack
docker compose up -d                     # SurrealDB + Cairn on :7777

# 3. Mint a token and onboard (wires every detected agent, including OpenCode):
#    open http://127.0.0.1:7777/you/tokens, click "Mint token", copy the JWT, then:
cairn setup --all --server http://localhost:7777 --token <jwt>

# Or wire OpenCode by hand with the same token:
cairn setup opencode --server http://localhost:7777 --token <jwt>

# 4. Restart OpenCode so the MCP entry picks up. You'll see `cairn` in the tool list.
```

After that, OpenCode's tool palette includes `cairn_recall`, `cairn_remember`, `cairn_read`,
`cairn_verify`, `cairn_verify_baseline`, `cairn_assemble`, `proactive_recall`, `memory_graph`,
`memory_crystallize`, `document_ingest`, `document_search`, `search`, `metrics`, and 20+ more -
see [MCP tools](docs/reference/architecture.md#mcp-tool-surface).

### What Cairn gives OpenCode out of the box

- **Cross-session memory.** Decisions from last week are visible today via `cairn_recall`
  at session start, ranked by `confidence x applies_to`.
- **Project-scoped memory.** Memories default to Project scope when a project is detected.
  Good ones promote to Global via the nightly scoring pipeline.
- **Lean file reads.** `cairn_read` returns the AST outline (~90% smaller than the full
  file); the original is one `cairn_expand` away. No context lost.
- **Post-edit corruption detection.** `cairn_verify_baseline` compares the current file
  against the version recorded at read time. The PostToolUse hook fires it automatically.
- **Drift detection.** Each session records checkpoints; `cairn doctor --fix`
  re-anchors the model on long tasks.
- **One-line rules.** The MCP `prefer` tool turns "always use ripgrep" into a memory that
  re-fires on every session until contradicted.
- **Proactive recall.** The intent classifier fires before each turn - if the prompt
  has recall cues, relevant memories are auto-injected. No manual recall needed.
- **Document RAG.** `document_ingest` reads a file or URL, chunks it, and stores it for
  semantic search. `document_search` queries the chunks. Project-scoped.

## Status

**Active development - pre-1.0.** v0.8.x is feature-complete for the
memory/context/reliability surface but is **not stable** - breaking changes
on the upgrade path to 1.0 are expected. Dogfooded daily; not yet a
"install and forget" product. See [Roadmap](docs/planning/roadmap.md) for
what's done and what's next.

## Where to go next

| I want to... | Read |
|---|---|
| Install & operate the server | [docs/guides/admin.md](docs/guides/admin.md) |
| Upgrade an existing install | [docs/guides/upgrading.md](docs/guides/upgrading.md) |
| Understand how it works | [docs/reference/architecture.md](docs/reference/architecture.md) |
| Connect my AI tool / IDE | [docs/guides/ide-integration.md](docs/guides/ide-integration.md) |
| Web dashboard & auth | [docs/guides/web-auth.md](docs/guides/web-auth.md) |
| See the roadmap / vision | [Roadmap](docs/planning/roadmap.md) / [Vision](docs/reference/vision.md) |
| Why decisions were made | [docs/reference/decisions.md](docs/reference/decisions.md) (ADRs) |
| Measured token savings | [docs/testing/benchmarks.md](docs/testing/benchmarks.md) |
| Security policy | [SECURITY.md](SECURITY.md) |
| Browse the full docs library | [docs/README.md](docs/README.md) |

Release notes: [CHANGELOG.md](CHANGELOG.md). Contributing: [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
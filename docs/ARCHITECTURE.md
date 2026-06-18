# Architecture

How Cairn is structured today — crate graph, data flow, MCP tool surface, API endpoints,
and Docker topology.

---

## System Overview

```mermaid
graph TD
    Agent["AI Agent<br/>(Claude Code, OpenCode, Cursor, VS Code, Windsurf)"]
    CLI["cairn-cli (client binary)<br/>McpServer / RemoteProxy<br/>setup · run · hook · sync · pair"]
    Server["cairn (server binary)<br/>cairn-api: axum REST + web UI<br/>auth: JWT + rate limiting + CORS"]
    Store["cairn-store<br/>HelixBackend + BlobStore"]
    Helix["HelixDB<br/>graph + HNSW vectors"]
    MinIO["MinIO<br/>S3 persistence"]

    Agent -->|"MCP stdio<br/>+ lifecycle hooks"| CLI
    CLI -->|"local store (HelixDB)"| Store
    CLI -->|"HTTP proxy<br/>CAIRN_SERVER + CAIRN_TOKEN"| Server
    Server --> Store
    Store --> Helix
    Helix --> MinIO
```

---

## Two Binaries

| Binary | Crate | Role |
|---|---|---|
| `cairn` | `cairn-server` | Server: `serve`, `token create/list/revoke`, `pair-code` |
| `cairn-cli` | `cairn-cli` | Client: `mcp`, `setup`, `rules`, `run`, `hook`, `remember`, `recall`, `prefer`, `anchor`, `checkpoint`, `rollback`, `sync`, `pair`, `export`, `import`, `contribute`, `pull`, `bench`, `update`, `doctor` |

---

## Cargo Workspace — 14 Crates

### Dependency Graph

```mermaid
graph BT
    core["cairn-core<br/>types · config · errors"]
    store["cairn-store<br/>HelixDB + BlobStore"]
    context["cairn-context<br/>read · cache · AST · assemble"]
    memory["cairn-memory<br/>4-tier · recall · decay"]
    guard["cairn-guard<br/>verify · anchor · checkpoint"]
    shell["cairn-shell<br/>compress · recover"]
    profile["cairn-profile<br/>preferences"]
    assemble["cairn-assemble<br/>token-budget assembly"]
    share["cairn-share<br/>sanitization"]
    embed["cairn-embed<br/>embeddings"]
    mcp["cairn-mcp<br/>MCP server (stdio)"]
    api["cairn-api<br/>REST API + web UI"]
    server["cairn-server<br/>cairn binary"]
    cli["cairn-cli<br/>cairn-cli binary"]

    core --> store
    core --> embed
    core --> share
    store --> context
    store --> memory
    store --> guard
    store --> shell
    store --> profile
    memory --> assemble
    context --> mcp
    memory --> mcp
    guard --> mcp
    shell --> mcp
    profile --> mcp
    share --> mcp
    embed --> mcp
    assemble --> mcp
    mcp --> api
    core --> api
    api --> server
    mcp --> cli
    api --> cli
```

### Crate Roles

| Crate | Role |
|---|---|
| `cairn-core` | Domain types, config resolution, errors, hashing. No deps on other cairn crates. |
| `cairn-store` | HelixDB backend (graph + vector) + content-hash `BlobStore`. Token/memory/checkpoint persistence. |
| `cairn-context` | Read modes (full/signatures/map/auto), content-hash + mtime cache (~13-tok re-reads), tree-sitter AST outlines (11 languages), `expand` recovery, `Assembler` (token-budgeted context). |
| `cairn-memory` | 4-tier memory (working/episodic/semantic/procedural), consolidation, Ebbinghaus decay, SHA-256 dedup, BM25 lexical recall. |
| `cairn-assemble` | Edge-ordered context assembly under a token budget. Anti-context-rot. |
| `cairn-guard` | Verify edits vs originals, task anchor, checkpoint/rollback, reliability scoring. |
| `cairn-shell` | RTK-style command-output compression (filter/group/dedup), lossless via blob store. |
| `cairn-profile` | Preference/behavior learning, injected at session start. |
| `cairn-share` | Privacy-first sanitization: secret/PII detection, redaction, classification (shareable/review/private). |
| `cairn-embed` | Pluggable embeddings: local (fastembed/ONNX all-MiniLM-L6-v2), OpenAI, Ollama, hashing fallback. |
| `cairn-mcp` | MCP server over stdio. Local mode (opens HelixDB store) or remote proxy mode (forwards to `cairn-api`). 16 tools. |
| `cairn-api` | Axum REST API + embedded web UI (rust-embed). Auth middleware (JWT device tokens), rate limiting, CORS, TLS. |
| `cairn-server` | The `cairn` binary: `serve`, `token`, `pair-code`. |
| `cairn-cli` | The `cairn-cli` binary: `mcp`, `setup`, `run`, `hook`, `sync`, `pair`, `bench`, `update`, `doctor`, etc. |

---

## MCP Tool Surface (16 tools)

All tools are exposed via `cairn-cli mcp` (stdio) and mirrored at `/api/tools/list` + `/api/tools/call`.

```mermaid
mindmap
  root((Cairn MCP<br/>16 tools))
    Context
      read
      expand
    Memory
      remember
      recall
      wakeup
      consolidate
    Assembly
      assemble
    Guardrails
      checkpoint
      rollback
      checkpoints
      verify
      anchor
    Profile
      prefer
      profile
    Shell
      compress
    Sanitization
      sanitize
```

### Context (file operations)

| Tool | Description |
|---|---|
| `read` | Read a file through Cairn — cache-aware (auto mode), AST signatures, or full. Returns a compressed view + handle. |
| `expand` | Recover the byte-identical original for a handle/content hash returned by `read`. |

### Memory

| Tool | Description |
|---|---|
| `remember` | Save a durable memory (content, kind, tier, importance). |
| `recall` | Search memories by query, ranked by relevance + recency + importance. |
| `wakeup` | Session-start bootstrap: highest-value memories (decisions, tasks, preferences). |
| `consolidate` | Promote memories across tiers (working → episodic → semantic → procedural). |

### Context Assembly

| Tool | Description |
|---|---|
| `assemble` | Build a lean, edge-ordered working set under a token budget. Reports what's in/dropped. |

### Guardrails

| Tool | Description |
|---|---|
| `checkpoint` | Snapshot tracked files for rollback. Optional label. |
| `rollback` | Restore tracked files to a checkpoint's state. |
| `checkpoints` | List checkpoints (newest first) with their IDs. |
| `verify` | Compare proposed file content against the current version. Flags large unreplaced deletions. |
| `anchor` | Set or read the current task goal (re-injected at session start). |

### Profile

| Tool | Description |
|---|---|
| `prefer` | Record a standing user preference (stack, style, do/don'ts). |
| `profile` | Show recorded preferences (the profile block). |

### Shell

| Tool | Description |
|---|---|
| `compress` | Compress verbose command/tool output (cargo, git, build logs). Original retained via `expand`. |

### Sanitization

| Tool | Description |
|---|---|
| `sanitize` | Check text for secrets/PII before sharing/logging/committing. Redacts and classifies. |

---

## MCP Modes

```mermaid
flowchart LR
    subgraph Local["Local mode"]
        Agent1["Agent"] -->|"stdio"| CLI1["cairn-cli mcp<br/>(McpServer)"]
        CLI1 -->|"Store::open"| Helix1["Local HelixDB"]
    end

    subgraph Remote["Remote proxy mode"]
        Agent2["Agent"] -->|"stdio"| CLI2["cairn-cli mcp<br/>(RemoteProxy)"]
        CLI2 -->|"path rewrite<br/>+ HTTP"| API["cairn-api<br/>/api/tools/call"]
        API --> ServerHelix["Server HelixDB"]
        CLI2 -.->|"CAIRN_SERVER<br/>CAIRN_TOKEN"| Config["env vars"]
    end
```

### Local mode (default)
`cairn-cli mcp` opens the local store (`Store::open` → HelixDB). Requires `CAIRN_HELIX_URL`.
All tools run locally.

### Remote proxy mode
When `CAIRN_SERVER` is set, `cairn-cli mcp` runs `RemoteProxy` — forwards tool calls to the
remote Cairn server's HTTP API. No local HelixDB needed on the client device.

**Path rewriting:** For file tools (`read`, `verify`, `checkpoint`, `rollback`), the proxy
rewrites absolute host paths to workspace-relative paths before forwarding. The server has
the project mounted at `CAIRN_WORKSPACE_ROOT=/workspace`, so relative paths resolve correctly
inside the container.

---

## API Endpoints

All `/api/*` routes (except `/api/health` and `/api/pair/claim`) require `Authorization: Bearer
<jwt>` once any device token exists.

| Method | Path | Description | Auth |
|---|---|---|---|
| GET | `/api/health` | Health check | Open |
| GET | `/api/stats` | Server stats (memory count, reliability) | Required |
| GET | `/api/context/read` | Read a file (same as MCP `read`) | Required |
| GET | `/api/context/expand` | Expand a content hash | Required |
| GET | `/api/context/assemble` | Assemble context for a query | Required |
| POST | `/api/memory` | Remember a memory | Required |
| GET | `/api/memory/recall` | Recall memories | Required |
| GET | `/api/memory/wakeup` | Session-start bootstrap | Required |
| POST | `/api/memory/consolidate` | Consolidate tiers | Required |
| POST | `/api/guard/verify` | Verify file vs original | Required |
| GET/POST | `/api/guard/anchor` | Get/set task anchor | Required |
| POST | `/api/guard/checkpoint` | Create checkpoint | Required |
| GET | `/api/guard/checkpoints` | List checkpoints | Required |
| POST | `/api/guard/rollback` | Rollback to checkpoint | Required |
| POST | `/api/shell/compress` | Compress shell output | Required |
| GET/POST | `/api/profile` | Get/set preferences | Required |
| POST | `/api/share/sanitize` | Sanitize text | Required |
| GET | `/api/share/export` | Export memory bundle | Required |
| POST | `/api/share/import` | Import memory bundle | Required |
| POST | `/api/pool/contribute` | Contribute to shared pool | Required |
| GET | `/api/pool` | List shared pool | Required |
| GET | `/api/tools/list` | MCP tool surface (JSON) | Required |
| POST | `/api/tools/call` | Call an MCP tool via HTTP | Required |
| POST | `/api/pair/new` | Generate pairing code | Open |
| POST | `/api/pair/claim` | Claim a pairing code | Open |
| GET | `/api/sync/pull` | Pull remote changes | Required |
| POST | `/api/sync/push` | Push local changes | Required |

---

## Docker Topology

```mermaid
graph TD
    subgraph Compose["docker compose up -d"]
        Guard["minio-guard<br/>(one-shot)"]
        MinIOInit["minio-init<br/>(one-shot: create bucket)"]
        MinIO["minio<br/>S3 storage<br/>:9000 internal"]
        Helix["helix<br/>HelixDB graph + vectors<br/>:6969 → :8080"]
        Cairn["cairn<br/>Cairn server + web UI<br/>:7777"]

        Guard -->|"refuses insecure creds"| MinIO
        MinIOInit -->|"creates helix-db bucket"| MinIO
        MinIO --> Helix
        Helix --> Cairn
    end

    HostProject["Host project dir<br/>(read-only mount)"]
    HostProject -.->|"/workspace:ro"| Cairn

    Host["Host machine"]
    Host -->|":7777"| Cairn
    Host -->|":6969"| Helix
```

| Service | Image | Role | Port |
|---|---|---|---|
| `cairn` | `cairn:dev` (built) | Cairn server + web UI | 7777 |
| `helix` | `ghcr.io/helixdb/enterprise-dev` | HelixDB graph + vector datastore | 6969 → 8080 |
| `minio` | `minio/minio:latest` | S3 storage for HelixDB persistence | 9000 (internal) |
| `minio-init` | `minio/mc:latest` | One-shot: creates `helix-db` bucket | — |
| `minio-guard` | `alpine:3.19` | One-shot: refuses to boot with insecure MinIO creds | — |

### Key environment variables (compose)

| Variable | Value | Purpose |
|---|---|---|
| `CAIRN_HOST` | `0.0.0.0` | Bind on all interfaces (container) |
| `CAIRN_HELIX_URL` | `http://helix:8080` | HelixDB over compose network |
| `CAIRN_INSECURE` | `1` | Allow plain HTTP on non-loopback (local dev) |
| `CAIRN_WORKSPACE_ROOT` | `/workspace` | Project mount root for file tools |
| `CAIRN_TLS_CERT` / `CAIRN_TLS_KEY` | (not set in dev) | PEM cert+key for HTTPS |

### Volumes

| Volume | Mount | Purpose |
|---|---|---|
| `cairn-data` | `/data` | Cairn data dir (blobs, etc.) |
| `helix-minio` | `/data` (minio) | MinIO S3 data |

### Host project mount

The host project directory is mounted read-only at `/workspace` so `read`/`verify`/`checkpoint`
tools can access host files:

```yaml
volumes:
  - "${CAIRN_WORKSPACE_HOST:-.}:/workspace:ro"
```

---

## Security Boundaries

```mermaid
flowchart TD
    Request["Incoming request"]
    TLSGate{"TLS gate<br/>non-loopback?"}
    Auth{"Auth<br/>JWT valid?"}
    RateLimit{"Rate limit<br/>60/min?"}
    Workspace{"Workspace root<br/>path inside?"}
    Sanitize{"Sanitization<br/>on share/export?"}
    Tool["Tool executes"]
    Rejected["Rejected"]

    Request --> TLSGate
    TLSGate -->|"no TLS + no INSECURE"| Rejected
    TLSGate -->|"TLS or loopback or INSECURE=1"| Auth
    Auth -->|"missing/invalid"| Rejected
    Auth -->|"valid JWT"| RateLimit
    RateLimit -->|"over limit"| Rejected
    RateLimit -->|"under limit"| Workspace
    Workspace -->|"outside root"| Rejected
    Workspace -->|"inside root"| Sanitize
    Sanitize -->|"share/export path"| Tool
    Sanitize -->|"normal path"| Tool
```

| Boundary | Mechanism |
|---|---|
| Workspace root | `CAIRN_WORKSPACE_ROOT` — `ContextEngine::resolve_path` rejects paths outside the root |
| Auth | JWT device tokens (HS256, signed with `CAIRN_SECRET_KEY`). Required once any token exists. |
| TLS gate | Refuses to serve HTTP on non-loopback unless `CAIRN_INSECURE=1` or TLS cert+key set |
| CORS | `CAIRN_CORS_ORIGINS` allow-list (default: same-origin only) |
| Rate limiting | 60 req/min per IP for API; 5 req/min for pairing claim |
| Sanitization | `cairn-share` redacts secrets/PII before any share/export/contribute |

---

## Config Resolution

Precedence (highest → lowest):

1. CLI flag (e.g. `--host`, `--port`, `--data-dir`)
2. Real environment variable
3. Project `.env` (repo root)
4. Global `.env` (`~/.config/cairn/.env`)
5. Built-in default

Key variables: `CAIRN_DATA_DIR`, `CAIRN_HOST`, `CAIRN_PORT`, `CAIRN_HELIX_URL`, `CAIRN_SECRET_KEY`,
`CAIRN_TLS_CERT`, `CAIRN_TLS_KEY`, `CAIRN_INSECURE`, `CAIRN_WORKSPACE_ROOT`, `CAIRN_CORS_ORIGINS`,
`CAIRN_EMBED_PROVIDER`, `CAIRN_EMBED_MODEL`, `CAIRN_EMBED_URL`, `CAIRN_EMBED_API_KEY`,
`CAIRN_SERVER`, `CAIRN_TOKEN`, `CAIRN_HELIX_NS`.

### Full `.env` variable reference

| Variable | What |
|---|---|
| `CAIRN_DATA_DIR` | data directory (default: OS data dir; `/data` in Docker) |
| `CAIRN_HOST` · `CAIRN_PORT` | serve bind address (default `127.0.0.1:7777`) |
| `CAIRN_SERVER` | default server URL for `sync` / `pull` / `contribute` |
| `CAIRN_HELIX_URL` | HelixDB server URL — **required** (the `docker compose` stack sets it for you) |
| `CAIRN_HELIX_NS` | label-namespace prefix on the Helix backend; isolates multiple Cairn instances (default `cairn_`) |
| `CAIRN_SECRET_KEY` | 32+ byte HS256 key for signing device-token JWTs — **required** for production |
| `CAIRN_TLS_CERT` · `CAIRN_TLS_KEY` | PEM cert+key for HTTPS — **required** when binding to a non-loopback address |
| `CAIRN_INSECURE` | set `1` to allow plain HTTP on non-loopback (local dev only) |
| `CAIRN_WORKSPACE_ROOT` | restrict file reads/writes to this directory (path traversal guard) |
| `CAIRN_CORS_ORIGINS` | comma-separated allowed CORS origins (default: same-origin only) |
| `CAIRN_EMBED_PROVIDER` · `_MODEL` · `_URL` · `_API_KEY` | embedding model (default: local `all-MiniLM-L6-v2`) |
| `GITHUB_TOKEN` · `CAIRN_GITHUB_TOKEN` | optional. Lifts the GitHub API rate limit for `cairn update` |
| `HELIX_PORT` · `MINIO_ROOT_USER` · `MINIO_ROOT_PASSWORD` | (compose only) host Helix port and MinIO credentials |
| `CAIRN_REPO` · `CAIRN_INSTALL_DIR` | (install script only) override the GitHub repo and install location |

---

## CLI Commands

### `cairn` (server binary)

| Command | What it does |
|---|---|
| `cairn serve` | start the server + embedded web UI (`http://127.0.0.1:7777`) |
| `cairn token create <name>` | create a signed JWT device token (requires `CAIRN_SECRET_KEY`) |
| `cairn token list` · `cairn token revoke <token>` | manage device tokens |
| `cairn pair-code [name]` | generate a short, single-use pairing code for a new device |
| `cairn doctor` | verify the server-side setup |

### `cairn-cli` (client binary)

| Command | What it does |
|---|---|
| `cairn-cli mcp` | run the MCP server over stdio (local HelixDB or remote proxy via `CAIRN_SERVER`) |
| `cairn-cli setup [agent]` · `cairn-cli setup --all` | wire up MCP + instructions file (+ hooks for Claude Code); `--all` auto-detects |
| `cairn-cli setup --server <url> --token <t>` | configure agents to talk to a remote Cairn server |
| `cairn-cli rules [agent]` · `cairn-cli rules --all` | (re)write per-agent instructions that tell the model to use Cairn's tools |
| `cairn-cli run -- <cmd>` | run a command, print **compressed** output (full output retained) |
| `cairn-cli remember <text>` · `cairn-cli recall <query>` | store / search memory |
| `cairn-cli prefer <rule>` | record a standing preference (e.g. `cairn-cli prefer always use ripgrep`) |
| `cairn-cli anchor <goal>` | set the current task goal (re-injected at session start) |
| `cairn-cli checkpoint [label]` · `cairn-cli rollback <id>` · `cairn-cli checkpoints` | snapshot / restore tracked files |
| `cairn-cli sync --server <url> --token <t>` | multi-device sync (last-write-wins) |
| `cairn-cli pair <code> --server <url>` | onboard this device with a short code (no token copying) |
| `cairn-cli export <file>` · `cairn-cli import <file>` | move memory between machines offline |
| `cairn-cli export --share <file>` | export a sanitized, shareable bundle |
| `cairn-cli import --share <file>` | ingest a shared bundle |
| `cairn-cli contribute --server <url>` · `cairn-cli pull --server <url>` | federate sanitized knowledge with a shared pool |
| `cairn-cli bench [path]` | measure the token savings on a codebase |
| `cairn-cli update` | self-update the binaries to the latest GitHub release |
| `cairn-cli doctor` | verify the local setup |

---

## Multi-device & Sync

```mermaid
graph TD
    Server["Cairn Server<br/>cairn serve --host 0.0.0.0<br/>:7777 + web UI"]
    Helix["HelixDB<br/>graph + vectors"]
    MinIO["MinIO<br/>S3 persistence"]

    Server --> Helix
    Helix --> MinIO

    Device1["Laptop<br/>cairn-cli mcp<br/>+ Claude Code hooks"]
    Device2["Desktop<br/>cairn-cli mcp<br/>+ OpenCode MCP"]
    Device3["Server / NAS<br/>cairn-cli mcp<br/>+ Cursor MCP"]

    Device1 -.->|"pair + sync<br/>HTTP :7777"| Server
    Device2 -.->|"pair + sync<br/>HTTP :7777"| Server
    Device3 -.->|"pair + sync<br/>HTTP :7777"| Server
```

Run one Cairn server for all your devices, or keep a server per device and sync between them.

```sh
# On the server — expose it on the network and note its URL, e.g. http://192.168.1.10:7777
cairn serve --host 0.0.0.0           # or set CAIRN_HOST=0.0.0.0 in .env
cairn pair-code                      # prints a short, single-use pairing code

# On a personal device — point it at the server once
cairn-cli pair <code> --server http://192.168.1.10:7777
# now `cairn-cli sync --server http://192.168.1.10:7777` (or just `cairn-cli sync` if CAIRN_SERVER is set)
```

The **dashboard works at the server's URL out of the box** — open `http://192.168.1.10:7777` and the
UI talks to that same origin (no rebuild, no hardcoded localhost).

- **Tokens:** `cairn token create <name>` prints a signed JWT device token (requires `CAIRN_SECRET_KEY`).
  Once any token exists, `/api/*` requires `Authorization: Bearer <token>` (the web UI and `/api/health` stay open).
  The bearer value is never stored — only the token id and metadata are persisted. Local-only
  setups on loopback need no tokens.
- **Pairing:** on the host run `cairn pair-code` (or click *Generate pairing code* in the
  dashboard) for a short, single-use code; on the new device run
  `cairn-cli pair <code> --server http://host:7777`. It claims a device token (no long secret to copy),
  stores it, and runs the first sync. The claim endpoint is the only open `/api/*` route — the
  short-lived code is the credential.
- **Sync:** `cairn-cli sync --server http://host:7777 --token <token>` pulls remote changes then
  pushes local ones (last-write-wins on `updated_at`). After pairing, the token is remembered, so
  `cairn-cli sync --server http://host:7777` alone works.
- **Offline move:** `cairn-cli export dump.json` / `cairn-cli import dump.json` copies memory between
  machines with no network.

---

## Development

```sh
# Cairn needs a HelixDB — start just that service, or point at any HelixDB server.
docker compose up -d helix
CAIRN_HELIX_URL=http://localhost:6969 cargo run -p cairn-server -- serve
# server + API on http://127.0.0.1:7777
```

The landing page + operational control plane live in `web/` (Next.js, static-exported so the
binary can embed it):

```sh
cd web && npm install && npm run dev   # http://localhost:3000 (talks to the API on :7777)
```

During dev, use `cargo run -p cairn-cli -- mcp` as the MCP command.

---

## Connecting an agent by hand

If you prefer not to use `cairn-cli setup`, you can wire up MCP manually:

```json
{
  "mcpServers": {
    "cairn": { "command": "cairn-cli", "args": ["mcp"] }
  }
}
```

For a remote server:

```json
{
  "mcpServers": {
    "cairn": {
      "command": "cairn-cli",
      "args": ["mcp"],
      "env": { "CAIRN_SERVER": "http://192.168.1.10:7777", "CAIRN_TOKEN": "<token>" }
    }
  }
}
```

Or with Claude Code: `claude mcp add cairn -- cairn-cli mcp`.

**Claude Code plugin:** run `/plugin marketplace add Vellixia/Cairn` then `/plugin install cairn@cairn`
to bundle the MCP server, all four lifecycle hooks, slash commands (`/cairn:recall`,
`/cairn:remember`, `/cairn:sanitize`, `/cairn:bench`), and usage guidance in a single install.

---

## See also

- [Plan](PLAN.md) — product vision and problem analysis
- [Roadmap](ROADMAP.md) — what's done, what's next
- [Decisions](DECISIONS.md) — key architecture decisions
- [Benchmarks](BENCHMARKS.md) — measured token savings
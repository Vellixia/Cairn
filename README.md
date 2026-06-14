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

Cairn sits between your AI coding agents (Claude Code, Codex, OpenCode, Cursor, …) and your code.
It runs as one small server you self-host once, and every device + agent connects to it through a
single MCP endpoint plus lifecycle hooks.

## Why

AI agents fail on long, multi-session work in ways bigger context windows don't fix:

- They **forget everything** between sessions.
- They **re-read files** they already read, burning tokens.
- Quality **decays over long tasks** (context rot, reasoning drift, silent corruption).
- Memory is **siloed** per machine and per tool.

The bottleneck usually isn't the model's IQ — it's the **context fed to it** and the **drift over
time**. Cairn fixes that.

## The five pillars

1. **Remember** — decisions, tasks, and rationale persist across sessions, devices, and agents.
2. **Compress without loss** — files, shell output, and responses shrink in the window but stay
   fully recoverable (`expand`/`recover`). Cairn keeps the full-fidelity original; the agent gets
   a compact view + a handle.
3. **Assemble lean context** — fight context rot by feeding *less*, higher-signal, well-ordered
   context under a token budget.
4. **Stay reliable** — verify agent edits against retained originals, snapshot/rollback tracked
   files, and keep a task anchor on long tasks (active guardrails).
5. **Get smarter together** — learn your preferences and opt into a sanitized, federated
   **collective knowledge** pool so cheap/small models behave like senior, personalized engineers.

## Proof

Run **`cairn bench`** on your own repo to see the savings. Measured on Cairn's own `crates/` (25 files):

| Mechanism | Before | After | Saved |
|---|---|---|---|
| AST outline reads (feed code as structure) | ~59,052 tok | ~5,894 tok | **90%** |
| Re-reading an unchanged file | ~6,506 tok | ~19 tok | **99.7%** |
| Shell output (a verbose test log) | 153 lines | 1 line | **99%** |

All of it is lossless — the full original is retained and one `expand` away.

## Status

🚧 Active development — the engine is functional today (memory, no-loss compression, context
assembly, edit guardrails + reliability score, shell compression, preference learning,
privacy-first sanitization, a federated collective-knowledge pool, and multi-device sync). Vectors
+ graph (HelixDB) are next; see [the design plan](docs/PLAN.md).

This repo is a Cargo workspace:

| Crate | Role |
|---|---|
| `cairn-core` | shared domain types, hashing, config |
| `cairn-store` | pluggable backend (SQLite today) + content-hash blob store |
| `cairn-context` | cached reads · AST signature outlines (11 languages) · byte-identical `expand` |
| `cairn-memory` | remember · BM25 recall · wakeup · Ebbinghaus decay · 4-tier consolidation |
| `cairn-assemble` | token-budgeted, edge-ordered context assembler (anti-rot) |
| `cairn-guard` | verify edits vs originals · task anchor · checkpoint/rollback · reliability score |
| `cairn-shell` | RTK-style command-output compression (lossless via `expand`) |
| `cairn-profile` | preference learning — inject how you work |
| `cairn-share` | privacy-first sanitization — redact secrets/PII, classify shareable/review/private |
| `cairn-mcp` | MCP server (stdio) |
| `cairn-api` | axum REST API + embedded web UI |
| `cairn-cli` | the `cairn` binary (serve, mcp, run, hook, install, …) |

## Install

```sh
# Linux / macOS — one-liner (downloads the latest release binary)
curl -fsSL https://raw.githubusercontent.com/Vellixia/cairn/main/scripts/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/Vellixia/cairn/main/scripts/install.ps1 | iex

# Docker
docker run -p 7777:7777 -v cairn:/data ghcr.io/vellixia/cairn    # or: docker compose up

# From source
cargo install --git https://github.com/Vellixia/cairn cairn-cli
```

Then run `cairn serve` and open <http://127.0.0.1:7777>.

## Quickstart (dev)

```sh
cargo run -p cairn-cli -- serve
# server + API on http://127.0.0.1:7777
```

The landing page + operational control plane live in `web/` (Next.js, static-exported so the
binary can embed it):

```sh
cd web && npm install && npm run dev   # http://localhost:3000 (talks to the API on :7777)
```

## Connect an agent (MCP)

Cairn speaks the Model Context Protocol over stdio — point any MCP-capable agent at `cairn mcp`.

The fastest path is **`cairn install claude-code`**, which non-destructively wires up the MCP
server **and** the lifecycle hooks into `.mcp.json` and `.claude/settings.json`:
`SessionStart` injects your preferences + memory + current task; `UserPromptSubmit` assembles
relevant context and learns preferences; `PostToolUse` guards edits against silent corruption;
`SessionEnd` consolidates memory.

Using another editor? `cairn install cursor`, `cairn install vscode`, and `cairn install windsurf`
each wire up the MCP server in that agent's own config format (MCP only — they have no hook
system). Or run **`cairn install --all`** to auto-detect every agent present and configure each.
Every install is non-destructive and idempotent.

To do it by hand: run `claude mcp add cairn -- cairn mcp`, or add an `.mcp.json`:

```json
{
  "mcpServers": {
    "cairn": { "command": "cairn", "args": ["mcp"] }
  }
}
```

Tools exposed: `read`, `expand`, `remember`, `recall`, `wakeup`, `consolidate`, `assemble`,
`prefer`, `profile`, `anchor`, `checkpoint`, `rollback`, `checkpoints`, `verify`, `compress`,
`sanitize`.
During dev, use `cargo run -p cairn-cli -- mcp` as the command.

## Commands

The `cairn` binary:

| Command | What it does |
|---|---|
| `cairn serve` | start the server + embedded web UI (`http://127.0.0.1:7777`) |
| `cairn mcp` | run the MCP server over stdio (for agents) |
| `cairn install [agent]` · `cairn install --all` | wire up MCP (+ hooks for Claude Code); `--all` auto-detects |
| `cairn run -- <cmd>` | run a command, print **compressed** output (full output retained) |
| `cairn remember <text>` · `cairn recall <query>` | store / search memory |
| `cairn prefer <rule>` | record a standing preference (e.g. `cairn prefer always use ripgrep`) |
| `cairn anchor <goal>` | set the current task goal (re-injected at session start) |
| `cairn checkpoint [label]` · `cairn rollback <id>` · `cairn checkpoints` | snapshot / restore tracked files |
| `cairn token create <name>` · `cairn sync --server <url> --token <t>` | device tokens · multi-device sync |
| `cairn pair-code [name]` · `cairn pair <code> --server <url>` | onboard a new device with a short code (no token copying) |
| `cairn export <file>` · `cairn import <file>` | move memory between machines offline |
| `cairn export --share <file>` | export a sanitized, shareable bundle (secrets/PII redacted, private memories withheld) |
| `cairn import --share <file>` | ingest a shared bundle (tagged `shared`, deduplicated against existing) |
| `cairn contribute --server <url>` · `cairn pull --server <url>` | federate sanitized knowledge with a shared pool |
| `cairn bench [path]` | measure the token savings on a codebase (outlines, re-reads, shell) |
| `cairn update` | self-update the binary to the latest GitHub release |
| `cairn doctor` | verify the local setup |

## Multi-device & sync

Run one Cairn server for all your devices, or keep a server per device and sync between them.

- **Tokens:** `cairn token create <name>` prints a device token. Once any token exists, `/api/*`
  requires `Authorization: Bearer <token>` (the web UI and `/api/health` stay open). Local-only
  setups need no tokens.
- **Pairing:** on the host run `cairn pair-code` (or click *Generate pairing code* in the
  dashboard) for a short, single-use code; on the new device run
  `cairn pair <code> --server http://host:7777`. It claims a device token (no long secret to copy),
  stores it, and runs the first sync. The claim endpoint is the only open `/api/*` route — the
  short-lived code is the credential.
- **Sync:** `cairn sync --server http://host:7777 --token <token>` pulls remote changes then
  pushes local ones (last-write-wins on `updated_at`). After pairing, the token is remembered, so
  `cairn sync --server http://host:7777` alone works.
- **Offline move:** `cairn export dump.json` / `cairn import dump.json` copies memory between
  machines with no network.

## License

Apache-2.0. See [LICENSE](LICENSE).

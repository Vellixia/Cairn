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

## Five pillars

```mermaid
graph TD
    Root["Cairn"]

    Remember["Remember"]
    Compress["Compress"]
    Assemble["Assemble"]
    Reliable["Reliable"]
    Smarter["Smarter together"]

    Root --> Remember
    Root --> Compress
    Root --> Assemble
    Root --> Reliable
    Root --> Smarter

    Remember --> R1["cross-session"]
    Remember --> R2["cross-device"]
    Remember --> R3["cross-agent"]

    Compress --> C1["file reads"]
    Compress --> C2["shell output"]
    Compress --> C3["lossless"]

    Assemble --> A1["token budget"]
    Assemble --> A2["anti-rot"]

    Reliable --> Re1["verify edits"]
    Reliable --> Re2["checkpoint"]
    Reliable --> Re3["rollback"]

    Smarter --> S1["preferences"]
    Smarter --> S2["collective"]
    Smarter --> S3["federation"]
```

1. **Remember** — decisions and rationale persist across sessions, devices, and agents.
2. **Compress without loss** — files and shell output shrink in the window, stay fully recoverable.
3. **Assemble lean context** — feed *less*, higher-signal, well-ordered context under a token budget.
4. **Stay reliable** — verify edits vs originals, checkpoint/rollback, re-anchor on drift.
5. **Get smarter together** — learn preferences + opt-in sanitized collective knowledge.

## Proof

Run **`cairn-cli bench`** on your own repo. Measured on Cairn's own `crates/` (25 files):

| Mechanism | Before | After | Saved |
|---|---|---|---|
| AST outline reads | ~59,052 tok | ~5,894 tok | **90%** |
| Re-reading an unchanged file | ~6,506 tok | ~19 tok | **99.7%** |
| Shell output (verbose test log) | 153 lines | 1 line | **99%** |

All lossless — the full original is retained and one `expand` away. See [Benchmarks](docs/BENCHMARKS.md).

## Quick start

```sh
# macOS / Linux — the recommended install path
brew install Vellixia/tap/cairn       # ships both `cairn` (server) and `cairn-cli`
```

```sh
# Linux / macOS — one-liner (alternative to Homebrew)
curl -fsSL https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/Vellixia/Cairn/main/scripts/install.ps1 | iex
```

```sh
# From source
cargo install --git https://github.com/Vellixia/Cairn cairn-server cairn-cli
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

Cairn stores data in **HelixDB** — `docker compose` starts one for you, or point
`CAIRN_HELIX_URL` at an existing server. Then run `cairn serve` and open
<http://127.0.0.1:7777>.

## Connect an agent

```sh
cairn-cli setup --all         # auto-detect every installed agent and wire up MCP
# or target one:
cairn-cli setup opencode --server http://localhost:7777 --token <token>
```

Supports Claude Code (MCP + lifecycle hooks), OpenCode, Cursor, VS Code, Windsurf.
See [Architecture — Connecting an agent](docs/ARCHITECTURE.md#connecting-an-agent-by-hand) for manual setup.

## OpenCode quickstart

OpenCode is a first-class citizen — `cairn-cli` is one of its built-in providers.
The fastest path from `git clone` to a Cairn-aware session:

```sh
# 1. Install the CLI (one of these)
brew install Vellixia/tap/cairn         # macOS / Linux + Homebrew
# or
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
`cairn_verify`, `cairn_assemble`, and the v0.5.0 additions (`memory_graph`, `memory_crystallize`,
`search`, `metrics`, etc.) — see [MCP tools](docs/ARCHITECTURE.md#mcp-tool-surface).

### What Cairn gives OpenCode out of the box

- **Cross-session memory.** Decisions from last week are visible today via `cairn_recall`
  at session start, ranked by `confidence × applies_to`.
- **Lean file reads.** `cairn_read` returns the AST outline (~90% smaller than the full
  file); the original is one `cairn_expand` away. No context lost.
- **Drift detection.** Each session records checkpoints; `cairn-cli doctor --fix`
  re-anchors the model on long tasks.
- **One-line rules.** `cairn-cli prefer always use ripgrep` becomes a memory that
  re-fires on every session until contradicted.

### Upgrading OpenCode wiring

After a Cairn upgrade, re-run `cairn-cli setup opencode` to refresh the MCP entry
(the binary path + tool list may have changed). The setup command is idempotent —
it preserves unrelated entries in `opencode.json`.

See [Connect OpenCode by hand](docs/ARCHITECTURE.md#opencode) for the JSON shape
that `cairn-cli setup opencode` writes, in case you need to merge it into a managed
dotfile repo.

## Status

🚧 Active development — the engine is functional today. See [Roadmap](docs/ROADMAP.md) for
what's done and what's next.

## Upgrading from a pre-P0–P3 build

The hardening release includes several breaking changes. If you're upgrading from a build that predates the JWT / TLS / SHA-pin work, do this in order:

1. **Generate a `CAIRN_SECRET_KEY`** (≥ 32 bytes):
   ```sh
   openssl rand -base64 48
   ```
   Add it to `.env` (or `~/.config/cairn/.env`). The server refuses to start without it.

2. **Re-mint device tokens.** Old plaintext tokens are invalid under the new auth path. For each existing device:
   ```sh
   cairn-cli token create --name <device> --scope <admin|write|read>
   ```
   The bearer value is shown once; the server stores only metadata. Update each agent to use the new token.

3. **Update CLI invocations.** `cairn install` was renamed to `cairn-cli setup`. If you have scripts calling `cairn install`, switch to `cairn-cli setup`.

4. **TLS for network binds.** If you bind `cairn serve` to a non-loopback address, set `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`. `CAIRN_INSECURE=1` is allowed only for trusted local networks.

5. **Docker compose.** The bundled stack now binds to `127.0.0.1:7777` by default. Override with `-p "0.0.0.0:${CAIRN_PORT:-7777}:7777"` for LAN exposure.

See [CHANGELOG.md](CHANGELOG.md) for the full list of breaking changes and security hardening.

## Documentation

| Doc | Description |
|---|---|
| [Plan](docs/PLAN.md) | Product vision, problem analysis, core principles |
| [Architecture](docs/ARCHITECTURE.md) | Crate graph, MCP tools, API endpoints, Docker, config, CLI commands, multi-device sync |
| [Roadmap](docs/ROADMAP.md) | Development status — done, in progress, next |
| [Benchmarks](docs/BENCHMARKS.md) | Token savings methodology + measured results |
| [Audit Report](docs/audits/REPORT.md) | Security audit with fix-status tracking |

## License

Apache-2.0. See [LICENSE](LICENSE).
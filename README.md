<div align="center">

# 🪨 Cairn

### The open-source context & reliability layer for AI agents

**Make any model smart.** Remember everything · feed less, not more · stay reliable on long
tasks · get smarter together — self-hosted, one Rust binary, with no context ever lost.

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
4. **Stay reliable** — verify agent edits against retained originals, detect drift, and re-anchor
   long tasks (active guardrails).
5. **Get smarter together** — learn your preferences and opt into a sanitized, federated
   **collective knowledge** pool so cheap/small models behave like senior, personalized engineers.

## Status

🚧 Early development. See [the design plan](docs/PLAN.md) for the full architecture and roadmap.

This repo is a Cargo workspace:

| Crate | Role |
|---|---|
| `cairn-core` | shared domain types, hashing, config |
| `cairn-store` | SQLite + content-hash blob store (full-fidelity originals) |
| `cairn-context` | file reads with cache + byte-identical `expand` (the re-read killer) |
| `cairn-memory` | remember / recall / wakeup across sessions |
| `cairn-api` | axum REST API + (soon) the web control plane |
| `cairn-cli` | the `cairn` binary (`serve`, …) |

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

For Claude Code, run `claude mcp add cairn -- cairn mcp`, or add an `.mcp.json`:

```json
{
  "mcpServers": {
    "cairn": { "command": "cairn", "args": ["mcp"] }
  }
}
```

Tools exposed: `read`, `expand`, `remember`, `recall`, `wakeup`. During dev, use
`cargo run -p cairn-cli -- mcp` as the command.

## License

Apache-2.0. See [LICENSE](LICENSE).

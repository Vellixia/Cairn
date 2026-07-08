---
title: Connect an AI Agent to Cairn
type: guide
status: living
updated: 2026-07-04
---

# Connect an AI Agent to Cairn

## What this covers

How to connect Claude Code, Codex CLI, or OpenCode to a Cairn server: minting a token and
running `cairn setup --all`, the manual `cairn setup <agent>` fallback, exactly what each
install writes to disk, how to verify the connection, and the optional real-time edit/command
guard.

## Prerequisites

- A reachable Cairn server (self-hosted or hosted) and its URL.
- The `cairn` client binary on `PATH` (`cairn --version`).
- Claude Code, Codex CLI, and/or OpenCode already installed — `cairn` detects whichever is
  present.

## Steps

### 1. Connect (primary path)

Mint a token from the dashboard (**You > Tokens** → "Mint token"), then run:

```sh
cairn setup --all --server <url> --token <jwt>
```

This resolves credentials, saves `server`/`token` to
`~/.cairn/config.toml`, turns on context injection by default (~1k tokens/prompt on
`UserPromptSubmit`; disable with `CAIRN_INJECT_CONTEXT=false`), and auto-detects and wires every
supported agent found in the current project or your home directory. `--server` can be omitted
against a local dev server (it probes `localhost:7777` automatically). `cairn setup --all` is
idempotent: re-running it just updates existing wiring (printed as "re-onboarding").

### 2. Fallback: manual per-agent setup

Reach for `cairn setup` when you want explicit control — one specific agent, a non-default
scope, or a CI environment where auto-detection isn't appropriate:

```sh
cairn setup claude-code --server <url> --token <jwt>
cairn setup codex        --server <url> --token <jwt>
cairn setup opencode     --server <url> --token <jwt>
cairn setup --all        --server <url> --token <jwt>   # every agent cairn actually detects
```

`--server`/`--token` can also come from the `CAIRN_SERVER`/`CAIRN_TOKEN` environment variables
instead of flags. Useful flags:

- `--project` — Claude Code only: write the MCP entry to `.mcp.json` in the current directory
  instead of the global `~/.claude.json`.

Setup is non-destructive and idempotent: existing config is preserved, Cairn's entries are
merged in, and running it twice changes nothing.

### 3. What gets installed per agent

Every `cairn setup <agent>` run (and, transitively, `cairn setup --all`) writes:

| | Claude Code | Codex CLI | OpenCode |
|---|---|---|---|
| MCP entry | `~/.claude.json` (global) or `.mcp.json` (`--project`) → `mcpServers.cairn` | `~/.codex/config.toml` → `[mcp_servers.cairn]` | `opencode.json` (`$XDG_CONFIG_HOME/opencode/`, or `%USERPROFILE%\.config\opencode\` on Windows) → `mcp.cairn` |
| Lifecycle hooks | `.claude/settings.json`: `SessionStart`, `UserPromptSubmit`, `PostToolUse` (Edit\|Write\|MultiEdit\|NotebookEdit), `SessionEnd`, `PreCompact`, `PreToolUse` (Edit\|Write\|MultiEdit\|NotebookEdit\|Bash) | `~/.codex/hooks.json`: `SessionStart`, `UserPromptSubmit`, `PostToolUse` (apply_patch\|Edit\|Write), `Stop` | A generated plugin (`plugins/cairn.js`, auto-loaded — never also listed in the `plugin` array, which would double-fire it) bridging `session.created`/`session.deleted`/`session.idle`/`message.part.updated`/`chat.message` to `cairn hook` |
| On-demand skill | `.claude/skills/cairn/SKILL.md` (same scope as the MCP entry) | — | — |
| Managed instructions block | `CLAUDE.md` (slim pointer — the skill carries the real playbook) | `AGENTS.md` (fuller block — no skill system to fall back on) | `AGENTS.md` (same block as Codex) |

Every write above is a merge, not an overwrite — a foreign hook or an unrelated MCP server
already in the file survives. After setup, run `/mcp` inside Claude Code to approve the
newly-registered server.

### 4. Verify

```sh
cairn doctor --json
```

Runs 8 checks — data directory, remote server reachability + token validity, detected agents,
current project detection, per-agent config health (stale skill revision, duplicate hooks,
double-registered OpenCode plugin, ...), token expiry, client/server version skew, and the
offline-hook spool backlog — and exits non-zero if any failed. `cairn doctor --fix` self-heals
what it can (creates a missing data dir, re-installs an agent whose config drifted) and
re-checks before reporting.

```sh
cairn status
```

Shows the resolved server and token (and whether each came from an env var,
`~/.cairn/config.toml`, or a built-in default), whether context injection is on, the config
file path, the offline spool depth, and which agents were detected. Pass `--json` for
machine-readable output.

### 5. What the model gets automatically

Two mechanisms teach the model to use Cairn without you maintaining anything by hand:

- **MCP `instructions`** — every MCP-speaking client (Claude Code, Codex, OpenCode) receives a
  compact tool playbook in the `initialize` response's `instructions` field the moment it
  connects. This works with zero files written anywhere and zero per-agent configuration.
- **Claude Code skill** — `.claude/skills/cairn/SKILL.md` carries the fuller playbook (session
  lifecycle, edit-safety loop, token hygiene, scope model) and loads on-demand, only when Claude
  Code's own skill matcher judges a task relevant — keeping its always-available cost near
  zero. Codex and OpenCode have no skill system, so they get the fuller version inlined
  directly into their `AGENTS.md` managed block instead (see the table above).

Cairn's MCP server currently exposes 28 tools — `read`/`expand`, `remember`/`recall`/`search`,
`assemble`, `wakeup`, `checkpoint`/`rollback`/`checkpoints`, `anchor`, `prefer`/`profile`,
`compress`, `consolidate`, `verify`, `sanitize`, `proactive_recall`, the eight-strong
`memory_*` family (`edit`/`delete`/`pin`/`promote`/`reinforce`/`timeline`/`crystallize`/
`graph`), `metrics`, and `registry_search` — see `cairn_mcp::tool_defs()` for the authoritative,
current list.

### 6. Optional: the real-time guard

Every agent gets the `PreToolUse` hook wired unconditionally, but it's a silent no-op until you
opt in. Turn it on with either:

```sh
export CAIRN_GUARD=1
```

or in `~/.cairn/config.toml`:

```toml
[hooks]
guard = true
```

With the guard on, Claude Code's `PreToolUse` hook checks proposed `Edit`/`Write` calls against
Cairn's edit-verification endpoint (`/api/guard/verify`) before they run, and `Bash` commands
against Cairn's secret/PII sanitizer (`/api/share/sanitize`). It only ever emits `"ask"` —
surfacing a risk for you or the agent to weigh in on — never `"deny"`, and it fails open (does
nothing) whenever the tool isn't supported yet (`MultiEdit`/`NotebookEdit`), the target file
doesn't exist yet, or the server doesn't answer in time. This is Claude-Code-only for now:
Codex and OpenCode hooks have no way to return a permission decision.

## Troubleshooting

- **`cairn doctor` reports a config-health issue** (stale skill revision, duplicate hooks,
  double-registered OpenCode plugin) — re-run `cairn setup <agent>` (or `cairn doctor --fix`);
  installs are idempotent, so this just refreshes what's stale.
- **Token expired or expiring soon** — `cairn doctor`/`cairn status` flag this explicitly; mint
   a fresh token from the dashboard (**You > Tokens**) and re-run `cairn setup --all --server <url>
  --token <jwt>`.
- **Hooks seem to do nothing** — confirm `cairn status` shows a server and a valid token. Each
  hook runs as its own short-lived process and only sees `~/.cairn/config.toml`/env, not
  whatever env block an agent's own MCP entry carries.
- **Starting over** — `cairn reset` (add `--dry-run` to preview first) strips every Cairn-owned
  entry — MCP servers, hooks, the skill file, and the `CLAUDE.md`/`AGENTS.md` managed blocks —
  while leaving everything else in those files untouched.

## See also

- [docs/reference/architecture.md](../reference/architecture.md)

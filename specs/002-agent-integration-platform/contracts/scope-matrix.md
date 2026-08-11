# Contract: Installation Scope and Ownership Matrix

**Feature**: `002-agent-integration-platform`

The concrete answer to "which file, at which scope, owned by whom" for every
agent × resource. This matrix is a planning contract **and a test source**: each row has a
fixture, and the ownership expectation in the fixture is the row.

Locations were verified against current official sources on 2026-08-11 (research D30–D33).

## The rule behind the defaults

Prefer a project location the developer does **not** commit. Where the agent provides none,
use per-user, and say so. Never fall back to a committed file silently (FR-215, FR-218).

Instructions are the exception by design: they describe how *this repository* uses Cairn, so
they are project-scoped and commit-safe (FR-214).

## Claude Code

| Resource | Official scopes | Cairn default | Location | Committed? | Manager-ownable | Precedence |
|---|---|---|---|---|---|---|
| `mcp` | local, project, user | **user** | `~/.claude.json` (user section) | no | **yes** — CC Switch writes this file | local > project > user, by name, whole entry wins |
| `lifecycle` | user, project, project-local, managed | **project_local** | `.claude/settings.local.json` | no (gitignored) | no | all matching hooks run |
| `instructions` | project, user, managed | **project_shared** | `./CLAUDE.md` (or `./.claude/CLAUDE.md` if that is where the project keeps it) | **yes** | no | concatenated, root→cwd |
| `skill` | personal, project, plugin | **user** | `~/.claude/skills/cairn/` | no | **yes** — CC Switch installs here | personal overrides project |

`--shared` moves `mcp` → `.mcp.json` and `lifecycle` → `.claude/settings.json`.

**Feature 001 adoption**: Feature 001 wrote `lifecycle` to `.claude/settings.json` and `mcp`
to `.mcp.json` — both committed project scope. Both are **adopted in place** and recorded at
`project_shared`. Cairn does not relocate them (FR-217); `cairn integration migrate` does, on
request.

**Collision to expect**: Cairn's default `mcp` scope and CC Switch's target are the same file
at the same scope. If CC Switch owns it, Cairn direct must not install there — the operation
returns `conflicting_owner` rather than picking another scope (FR-219).

**Marker note**: Claude Code strips block-level HTML comments before instructions reach the
model, so the managed block's markers cost zero context (D30).

## Codex

| Resource | Official scopes | Cairn default | Location | Committed? | Manager-ownable | Precedence |
|---|---|---|---|---|---|---|
| `mcp` | user, project | **user** | `~/.codex/config.toml` `[mcp_servers.cairn]` | no | **yes** — CC Switch writes this file | project layer overrides user |
| `lifecycle` | user, project, managed | **user** | `~/.codex/hooks.json` | no | no | layered; trust-gated per hook |
| `instructions` | project, user | **project_shared** | `./AGENTS.md` | **yes** | no | first project match wins (shared with OpenCode) |
| `skill` | user, repo, system, admin | **user** | `~/.codex/skills/cairn/` | no | **yes** — CC Switch installs here | — |

`--shared` moves `mcp` → `.codex/config.toml` and `lifecycle` → `[hooks]` in
`.codex/config.toml`.

**Why lifecycle is per-user**: Codex has no project-local-ignored configuration file. Writing
hooks into `.codex/config.toml` would commit a handler that runs a local binary into a shared
repository. Per-user is machine-local activation, which is what FR-215 actually wants; Cairn
states this at connect time rather than falling back silently (FR-218).

**Activation**: a newly written hook is `Untrusted` and does not run until the user trusts it
inside Codex. Editing a trusted hook resets it to `Modified`. Both are reported as
`installed_not_activated` with the exact step (D24).

**Editing**: TOML is edited with `toml_edit`, preserving comments, ordering, and formatting
(FR-153, D37). `hooks.json` is plain JSON.

## OpenCode

| Resource | Official scopes | Cairn default | Location | Committed? | Manager-ownable | Precedence |
|---|---|---|---|---|---|---|
| `mcp` | global, project | **user** | `~/.config/opencode/opencode.json` `mcp.cairn` | no | **yes** — CC Switch writes this file | project files merge after global; `.jsonc` merges after `.json` |
| `lifecycle` | any config directory | **user** | `~/.config/opencode/plugin/cairn.js` | no | no | all discovered plugins load |
| `instructions` | project, global | **project_shared** | `./AGENTS.md` — **the same block Codex reads** | **yes** | no | first project match wins |
| `skill` | config dirs, `~/.claude/skills`, `~/.agents/skills`, configured paths | **user**, or `shared` | `~/.config/opencode/skills/cairn/`, unless satisfied by Claude Code's copy | no | **yes** | duplicate skill *names* conflict |

`--shared` moves `mcp` → `./opencode.json` and `lifecycle` → `.opencode/plugin/cairn.js`.

**Lifecycle is a file drop, not a config edit**: OpenCode auto-discovers
`{plugin,plugins}/*.{ts,js}` inside every config directory, so installing the plugin needs no
mutation of `opencode.json` at all (D32). This is why the JSONC-editing problem does not
touch the lifecycle path.

**`opencode.jsonc` is never written** (D37). Cairn writes `opencode.json`, which OpenCode
merges alongside it. If a `.jsonc` already declares `mcp.cairn`, it merges *after* the
`.json` and would shadow Cairn's entry — reported as `conflicting_owner`, not edited (D38).

**Shared instruction block**: `AGENTS.md` is read by both Codex and OpenCode. Cairn installs
**one** managed block there and records it for both agents, reporting the sharing rather than
writing it twice (FR-144).

**Shared Skill**: OpenCode scans `~/.claude/skills/**/SKILL.md`. When Claude Code's Cairn
Skill is installed and current, OpenCode's `skill` resource is recorded `shared`, with
`satisfied_by = claude-code`, and no second copy is written — two copies with one skill name
would make OpenCode log a conflict and pick non-deterministically (D28). If Claude Code is
later disconnected, doctor reports OpenCode's Skill `missing` and repair installs its own
copy.

## Generic MCP

| Resource | Cairn default | Location |
|---|---|---|
| `mcp` | — | none. `cairn integration export mcp` emits a block the developer pastes (FR-131) |

No lifecycle, no instructions, no Skill. Level is `MCP_ONLY`, and the MCP `instructions`
string is the only way the usage contract reaches such a client (FR-129).

## CC Switch as owner

CC Switch distributes only `mcp` and `skill`, and only at per-user scope, into each target
application's own configuration (D33):

| Application | MCP written to | Skill installed to |
|---|---|---|
| `claude` | `~/.claude.json` `mcpServers` | `~/.claude/skills/` |
| `codex` | `~/.codex/config.toml` `[mcp_servers]` | `~/.codex/skills/` |
| `opencode` | `~/.config/opencode/opencode.json` `mcp` | `~/.config/opencode/skills/` |

Cairn never writes these files when the owner is `manager`; it verifies them (FR-234) and
records `owner = manager` with no content hash, because it did not write the bytes.

## Ownership identity

| Resource | How Cairn knows it owns it |
|---|---|
| `mcp` | Reserved server name `cairn` + canonical hash of the entry, recorded locally |
| `lifecycle` (Claude, Codex) | Recorded hook keys + canonical hash of each entry |
| `lifecycle` (OpenCode) | Cairn-owned file path; Cairn generated the whole file |
| `instructions` | `<!-- cairn:managed:begin id=agent-contract … -->` … `<!-- cairn:managed:end id=agent-contract -->` |
| `skill` | Cairn-owned directory; Cairn generated every file in it |

Never by searching for the string `cairn` (FR-139).

**Legacy bridge**: with no local record, Feature 001 installations are recognized by a closed
set of exact shapes — a hook entry whose sole command is exactly `cairn hook <Event>` for one
of the six Feature 001 events with the Feature 001 structure, and an MCP entry exactly
`{"command":"cairn","args":["mcp"]}`. Matched once, adopted into the record, and never
matched by shape again. An arbitrary user hook that merely mentions `cairn hook` in a longer
command does not match.

## Summary of defaults

| | Claude Code | Codex | OpenCode |
|---|---|---|---|
| `mcp` | user | user | user |
| `lifecycle` | project_local | user | user |
| `instructions` | project_shared | project_shared (`AGENTS.md`, shared) | project_shared (`AGENTS.md`, shared) |
| `skill` | user | user | user or `shared` |

Nothing is committed by default except the managed instruction block, which is the one
resource whose whole purpose is to travel with the repository.

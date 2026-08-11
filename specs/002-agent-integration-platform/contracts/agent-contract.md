# Contract: Cairn Agent Usage Contract, Skill, and MCP Instructions

**Feature**: `002-agent-integration-platform`

One canonical source teaches an agent how to use Cairn. It renders into two forms, and the
deeper material lives in a Skill that loads only when it is useful.

```
crates/cairn-integrate/assets/agent-contract.md   ─┬─▶  managed instruction block (FR-132)
                                                   └─▶  MCP `instructions` string (FR-129)

skills/cairn/                                       ─┬─▶  installed Skill, embedded (FR-140)
                                                    └─▶  CC Switch import from a published
                                                         skill-release/<schema>-<revision> branch
```

Both sources are embedded into the binary at build time, so an installed Cairn always
carries the version it will write.

## The always-on contract

**Size bound**: 1,200 characters for the rendered body, asserted by test (FR-125, SC-105).
Two reasons for that number: Claude Code's `additionalContext` is capped at 10,000 characters
and its guidance targets CLAUDE.md files under 200 lines, and every byte here is paid on
every session of every agent. The contract is a set of rules, not documentation.

**Content** (FR-124) — the rules, not the prose:

1. Read the Cairn context you were given before re-deriving the project.
2. Search Cairn memory before repeating an investigation you may already have done.
3. Record durable facts, decisions, conventions, failures, and procedures — not routine tool
   calls.
4. Use the narrowest correct scope: task, else branch, else project.
5. Never invent an evidence observation identifier.
6. Never put secrets, credentials, raw prompts, or unbounded output into memory.
7. Session boundaries, checkpoints, and handoffs are automatic where the integration supports
   them. Do not hand-roll them.
8. Bind work to a Cairn task when one applies.
9. For deeper workflows, use the Cairn Skill.

**Versioning** (D26): `contract_schema` (integer) plus `contract_revision` (12-hex digest of
the normalized rendered body). Both appear in the managed block's marker.

**Rendering invariant**: the instruction-block rendering and the MCP-instructions rendering
are produced by one function from one source, and a test asserts both state the same
numbered rules (FR-123). Neither is maintained by hand.

## The managed instruction block

```markdown
<!-- cairn:managed:begin id=agent-contract schema=1 content=8f2b19c40a7d -->
## Cairn — persistent project memory

… the nine rules, rendered …
<!-- cairn:managed:end id=agent-contract -->
```

| Property | Rule |
|---|---|
| Locating | The full literal prefix `<!-- cairn:managed:begin id=`. Never a search for `cairn` (FR-139) |
| Validity | Exactly one `begin`, exactly one `end`, matching `id`, begin first. Anything else is `damaged_markers` and Cairn changes nothing (FR-137) |
| Writing | Only bytes between the markers change. Everything outside is copied verbatim (FR-133) |
| Idempotency | Reconnecting with a matching `schema` and `content` writes nothing and reports `unchanged` (FR-135) |
| Upgrading | A differing `schema` or `content` replaces the block in place, preserving its surroundings (FR-136) |
| Removing | The two markers and everything between them; the file survives if anything else remains (FR-138) |
| Comparison | Semantic: normalized body compared by digest, so reflow or trailing whitespace is not an edit (FR-223) |

Claude Code strips block-level HTML comments before instructions reach the model (D30), so
the markers cost zero context there. In `AGENTS.md` they are ordinary Markdown comments.

**Shared file**: `AGENTS.md` is read by both Codex and OpenCode. Cairn installs **one** block
and binds both agents to it (FR-144, FR-243). Disconnecting one agent drops its binding and
leaves the block in place for the other; the block is removed only when the last bound agent
disconnects. Doctor reports the block with its full `serves` list.

## MCP instructions

`initialize` gains the `instructions` field that MCP `2025-06-18` already defines. The
protocol revision does not change (D34, FR-130).

```json
{
  "protocolVersion": "2025-06-18",
  "capabilities": { "tools": {} },
  "serverInfo": { "name": "cairn", "version": "0.1.0-alpha.2" },
  "instructions": "Cairn is persistent, project-aware memory for this repository.\n1. Call cairn_context before …"
}
```

- Same nine rules, compressed to the tool-facing form: no mention of hooks or Skills, since a
  generic client has neither.
- **Delivery is best-effort.** The specification calls `instructions` a hint clients *may*
  add to the system prompt. OpenCode demonstrably injects it inside an `<mcp_instructions>`
  block (D32); other clients may ignore it. Cairn therefore never reports the contract as
  *delivered* through this path (FR-129).
- The tool surface is unchanged: exactly the six Feature 001 tools, and a test fails if a
  seventh appears (FR-128, SC-106).

## The Cairn Skill

Canonical source at `skills/cairn/` in the repository — embedded for direct installation and
fetched by CC Switch from that same path (D29, FR-141).

```
skills/cairn/
├── SKILL.md                     # entry point, kept short
└── references/
    ├── resuming-work.md         # read the handoff, continue rather than restart
    ├── searching-first.md       # search before investigating; what a good query looks like
    ├── recording-knowledge.md   # decision, failure, convention, procedure — and what not to record
    ├── choosing-scope.md        # project / branch / task / session, and how to choose
    ├── sessions-and-tasks.md    # binding to a task; resolving session ambiguity
    └── diagnosing-cairn.md      # what to do when Cairn reports a problem
```

`SKILL.md` frontmatter uses only fields every target accepts — `name`, `description`, and the
free-form `metadata` map (D30, D31):

```yaml
---
name: cairn
description: Use Cairn's persistent project memory — resume prior work, search before investigating, record durable decisions and failures, choose the right memory scope, and bind work to a task.
metadata:
  cairn_skill_schema: 1
  cairn_skill_revision: c07d4419b2ae
---
```

No standard `version` frontmatter field exists across these agents, which is why the version
lives in `metadata` — accepted and ignored by the agents, and read by `cairn doctor` to
detect an outdated Skill (FR-141).

**Progressive disclosure** (FR-142): `SKILL.md` states when to use Cairn and links the
reference files; the references carry the detail. Nothing in the Skill duplicates the
always-on contract.

**Installation** (FR-143, D28):

- One physical copy per machine per agent, at that agent's per-user Skill directory.
- Never overwrites a Skill named `cairn` that Cairn does not own — that is
  `conflicting_owner`.
- Where one agent reads another's Skill directory, the second agent **binds to the existing
  resource** and no duplicate is written (D28).
- Removal drops the disconnecting agent's binding; the directory is deleted only when no
  binding remains, and nothing else is touched.

## Version summary

| Line | Where it appears | Bumped when |
|---|---|---|
| `contract_schema` | marker `schema=` | The block format or contract structure changes |
| `contract_revision` | marker `content=` | The rendered contract text changes |
| `skill_schema` | `metadata.cairn_skill_schema` | The Skill's layout or frontmatter shape changes |
| `skill_revision` | `metadata.cairn_skill_revision` | Any Skill file changes |

None is tied to Cairn's package version (D26). A patch release that changes neither the
contract nor the Skill rewrites nothing — which on Codex also means it does not invalidate
hook trust (D24).

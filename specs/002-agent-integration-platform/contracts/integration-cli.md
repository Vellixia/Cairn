# Contract: Integration CLI

**Feature**: `002-agent-integration-platform`

Extends [001's CLI contract](../../001-cairn-mvp/contracts/agent-integration.md). Every
command keeps Feature 001's envelope and exit codes:

```json
{ "ok": true,  "data": { … } }
{ "ok": false, "error": { "code": "…", "message": "…" } }
```

Exit `0` success, `1` user error, `2` Cairn unavailable. `cairn hook` still always exits 0.

## Surface

Feature 001's `connect` and `disconnect` stay where they are — moving them would break
existing users for no gain. Three flat verbs are added, plus one sub-noun for the operations
a developer runs rarely.

| Command | Purpose |
|---|---|
| `cairn agents` | List detected agents and managers with version, compatibility, and integration level |
| `cairn connect [<agent>] [flags]` | Install or update an integration; `--auto` for guided onboarding |
| `cairn disconnect <agent>` | Remove Cairn-owned integration for one agent |
| `cairn doctor [<agent>]` | Inspect integration health; no mutation |
| `cairn repair [<agent>] [flags]` | Restore Cairn-owned state only |
| `cairn integration export mcp [flags]` | Emit a deterministic, secret-free MCP configuration |
| `cairn integration migrate <agent> <kind> --to <owner>` | Move one resource between owners |
| `cairn integration distribute --via <manager> [flags]` | Distribute Cairn resources through a manager |

`<agent>` is `claude-code` \| `codex` \| `opencode` \| `generic-mcp`. `<kind>` is `mcp` \|
`lifecycle` \| `instructions` \| `skill`. `<owner>` is `direct` \| `cc-switch`.

---

## `cairn agents`

Detection only. Makes no change, requires no network (FR-105).

```
$ cairn agents
claude-code   2.1.220   verified               FULL
codex         0.58.0    compatible-unverified  MCP_PLUS   (hooks awaiting trust in Codex)
opencode      1.4.2     compatible-unverified  MCP_PLUS   (no automatic session completion)
cc-switch     3.9.1     compatible-unverified  manager    (mcp, skill → claude, codex, opencode)
```

`--json` emits `data.agents[]` and `data.manager`, each as in
[integration-health.md](./integration-health.md)'s detection block.

A level is never printed as a bare word where a capability is missing: the parenthetical
naming the missing behavior is part of the contract (FR-111).

---

## `cairn connect`

```
cairn connect [<agent>]
              [--auto] [--dry-run] [--yes]
              [--shared] [--scope <kind>=<scope>]...
              [--via cc-switch] [--apps <list>]
```

| Flag | Behavior |
|---|---|
| `--auto` | Detect everything installed and propose a plan covering all of it (FR-163) |
| `--dry-run` | Print the change plan and exit. Zero writes, including no temporary files (FR-159, SC-118) |
| `--yes` | Apply without confirmation. Required for non-interactive use; still refuses where a conflict needs a decision (FR-164) |
| `--shared` | Install lifecycle and MCP into committed project scope (FR-218) |
| `--scope` | Override one resource's scope, e.g. `--scope mcp=project_shared` |
| `--via cc-switch` | Distribute `mcp` and `skill` through the manager instead of installing directly |
| `--apps` | Manager target applications; defaults to the agents being connected |

Bare `cairn connect` with no agent is equivalent to `--auto`.

Interactive runs print the plan and ask. Non-interactive runs without `--yes` print the plan
and exit `1` with `confirmation_required`.

### Change-plan output

```
$ cairn connect codex --dry-run
Plan for codex (dry run — nothing written)

ADD       mcp           user            ~/.codex/config.toml            [mcp_servers.cairn]
ADD       lifecycle     user            ~/.codex/hooks.json             6 hook registrations
UPDATE    instructions  project_shared  ./AGENTS.md                     cairn:managed block v1
INSTALL   skill         user            ~/.codex/skills/cairn/          schema 1
UNCHANGED all other MCP servers, all other hooks, all other instructions

Codex will not run these hooks until you trust them:
  codex hooks trust        (then re-run `cairn doctor codex`)
```

Six registrations, seven canonical events: Codex has no separate tool-failure hook, so its one
`PostToolUse` registration normalizes into either `tool_succeeded` or `tool_failed` depending
on the payload (D23). Claude Code registers seven, because it has both.

`--json` emits an `IntegrationChangePlan` (see [data-model.md](../data-model.md)). Preview
output never prints a credential found while inspecting (FR-162, SC-133).

---

## `cairn doctor`

Read-only (FR-170). Exit `0` when every reported condition is `healthy`, `shared`, or
`unknown`; exit `1` when any actionable condition is present; exit `2` when Cairn itself is
unavailable.

```
$ cairn doctor
core        cli 0.1.0-alpha.2 · daemon 0.1.0-alpha.2 · schema 4 · project registered

claude-code  FULL
  mcp           healthy    direct   user            ~/.claude.json
  lifecycle     healthy    direct   project_local   .claude/settings.local.json
  instructions  outdated   direct   project_shared  ./CLAUDE.md  (schema 1, revision a41f→8f2b)
  skill         healthy    direct   user            ~/.claude/skills/cairn/

codex        MCP_PLUS  — automatic session completion pending activation
  lifecycle     installed_not_activated  direct  user  ~/.codex/hooks.json
                run `codex hooks trust`, then re-run doctor

opencode     MCP_PLUS  — no automatic session completion (OpenCode signals no session end)
              tool failures captured only when OpenCode's output establishes them
              awaiting: first observed session open, tool call and quiescence
  skill         shared     ~/.claude/skills/cairn/  serves claude-code, opencode
  instructions  shared     ./AGENTS.md              serves codex, opencode

cc-switch    detected 3.9.1
  mcp  manager  → claude ✓  codex ✓  opencode ✗ (binding not found)
```

`--json` emits the full `IntegrationHealthReport`.

---

## `cairn repair`

```
cairn repair [<agent>] [--dry-run] [--force]
```

Default repair restores `missing`, upgrades `outdated`, removes `duplicated` Cairn-owned
entries, and touches nothing else (FR-172, FR-173). It **reports** `modified`,
`damaged_markers`, `conflicting_owner`, and `malformed_config` and changes nothing for them
(FR-174, FR-177).

`--force` additionally restores `modified` resources, strictly inside Cairn's ownership
boundary and only after writing a recovery artifact (FR-221, FR-222):

```
$ cairn repair claude-code --force
preserved  ~/.cairn/recovery/claude-code/instructions/2026-08-11T09-14-02Z-3c9e11ab04d7.txt
UPDATE     instructions  ./CLAUDE.md  cairn:managed block restored (schema 1)
UNCHANGED  everything outside the cairn:managed markers
```

`--force` never touches `damaged_markers`, `conflicting_owner`, or `malformed_config` — those
need a human decision, and forcing past a damaged marker would mean guessing which text was
Cairn's.

Repair is idempotent: a second run reports `nothing to do` and writes nothing (FR-175,
SC-115).

---

## `cairn disconnect`

```
cairn disconnect <agent> [--only <kind>]... [--dry-run]
```

Removes that agent's dependency on its Cairn-owned lifecycle, managed instruction block,
Cairn-owned Skill, and directly-owned MCP entry. `--only <kind>` restricts removal to the named
resource kinds — repeatable, and the option the ownership-migration sequence uses.

**Removal is by binding, not by file** (FR-243). Disconnect drops this agent's binding to each
resource; the resource itself is deleted only when no other agent is still bound to it. So
disconnecting Codex while OpenCode remains connected leaves the shared `AGENTS.md` block in
place, and OpenCode stays healthy:

```
$ cairn disconnect codex
removed   lifecycle     ~/.codex/hooks.json
removed   skill         ~/.codex/skills/cairn/
unbound   instructions  ./AGENTS.md  (block kept — still serving opencode)
```

The agent's local record is removed last, and **only when its last binding is gone** — an agent
whose remaining resource is manager-owned keeps its record so the withdrawal stays verifiable
(FR-244).

Never removes a manager-owned resource. Where one exists, disconnect completes for everything
Cairn owns and additionally returns a `ManagerActionRequired` for the rest (FR-149, FR-233):

```
$ cairn disconnect codex
removed   lifecycle     ~/.codex/hooks.json
removed   instructions  ./AGENTS.md  (block only; your content is untouched — no other agent bound)
removed   skill         ~/.codex/skills/cairn/

manager action required — CC Switch owns the Cairn MCP entry for codex
  Cairn does not modify CC Switch's own storage.
  In CC Switch: MCP → cairn → turn off "Codex", or remove the server.
  Then run: cairn doctor codex

Codex's local record is kept so this withdrawal stays verifiable; it is removed once
`cairn doctor` observes the entry gone.

Memory, tasks, sessions and handoffs are untouched.
```

Exit `1` with `manager_action_required` — the operation did not fully complete.

---

## `cairn integration export mcp`

```
cairn integration export mcp [--agent <agent>] [--format json|toml]
```

Emits a configuration block for the named agent's format, or a generic MCP `mcpServers`
object when no agent is given. Writes nothing (FR-131). Deterministic and secret-free
(SC-135).

```
$ cairn integration export mcp
{
  "mcpServers": {
    "cairn": { "command": "cairn", "args": ["mcp"] }
  }
}
```

---

## `cairn integration migrate`

```
cairn integration migrate <agent> <kind> --to direct|cc-switch [--dry-run] [--resume] [--abort]
```

Drives the `MigrationState` machine (see [data-model.md](../data-model.md)): install target →
verify target → remove source. `--resume` and `--abort` act on an interrupted migration.

Refuses to start automatically where overlap would make the effective configuration
ambiguous, and prints the manual sequence instead (FR-148, D38):

```
$ cairn integration migrate claude-code mcp --to cc-switch
cannot migrate automatically

  CC Switch writes ~/.claude.json (user scope).
  Cairn's direct entry is also at user scope in the same file, so the two
  cannot coexist unambiguously for the duration of the migration.

  Safe sequence:
    1. cairn integration migrate claude-code mcp --to cc-switch --dry-run
    2. cairn disconnect claude-code --only mcp        # drops just the MCP binding
    3. cairn integration distribute --via cc-switch --resource mcp --apps claude
    4. cairn doctor claude-code
```

Exit `1` with `migration_unsafe`.

---

## `cairn integration distribute`

```
cairn integration distribute --via cc-switch --resource mcp|skill --apps <list> [--dry-run]
```

Initiates the manager's documented import flow and then stops, returning a
`ManagerActionRequired` with status `awaiting_user` (FR-233). It never reports success before
verification. `cairn doctor` performs the verification afterwards (FR-234).

---

## Error codes

Feature 001's codes stay. Feature 002 adds these to the same `codes` module — a closed set,
not ad-hoc strings (FR-167).

| Code | Meaning | Exit |
|---|---|---|
| `agent_not_detected` | The named agent is not installed | 1 |
| `agent_unsupported` | Its version is positively known to be incompatible | 1 |
| `malformed_config` | A configuration file could not be parsed; nothing was written | 1 |
| `permission_denied` | A target file or directory is not writable | 1 |
| `damaged_markers` | A managed block's markers are missing or unbalanced | 1 |
| `resource_modified` | A Cairn-owned resource was hand-edited; `--force` required | 1 |
| `duplicate_resource` | Two Cairn-owned copies exist outside a migration | 1 |
| `conflicting_owner` | Owned by someone other than the record says, or shadowed | 1 |
| `installed_not_activated` | Installed but the agent has not been told to trust it | 1 |
| `migration_in_progress` | A migration for this resource is already running | 1 |
| `migration_unsafe` | Automatic migration would be ambiguous; manual sequence given | 1 |
| `manager_action_required` | Needs a step in the manager that Cairn cannot perform | 1 |
| `verification_failed` | The change was applied but not observed to be effective | 1 |
| `confirmation_required` | Non-interactive run without `--yes` | 1 |
| `unpublished_skill_ref` | A manager Skill import was requested from a build whose embedded Skill revision has no published `skill-release` branch. Emitting an unpublished ref would make CC Switch silently install `main` | 1 |
| `partial_apply` | A multi-file change partly landed; the report names both halves | 1 |

`daemon_unavailable` and `storage_unavailable` keep exit `2`.

Configuration operations **fail loudly** — none of them is fail-soft (FR-196). Only the hook
path fails soft, and it is unchanged (FR-193).

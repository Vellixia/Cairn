# Research: Agent Integration Platform

**Feature**: `002-agent-integration-platform` | **Date**: 2026-08-11

Decisions that shape the plan, continuing Feature 001's numbering (D1–D17). Each records
what was chosen, why, and what was rejected. Anything not listed here is an ordinary
implementation choice.

Vendor facts were verified against primary sources on 2026-08-11 and are recorded in
**D30–D33**. Where a vendor's own documentation site was unreachable from this environment,
the vendor's published source repository was used instead and the file is cited.

---

## D18 — Where the integration layer lives

**Decision**: One new crate, `cairn-integrate`, holding every adapter, the CC Switch
manager, the desired-state model, the change-plan engine, the configuration editors, and
the rendered contract and Skill assets. It depends on `cairn-core` and is depended on by
`cairn` (the CLI). It does **not** depend on `cairn-store`, and `cairnd` does not depend on
it.

Persistence of the local integration record stays in `cairn-store` behind new daemon
requests, because the daemon is the single writer to SQLite (D12).

**Why**: the dependency direction decides this, not taste.

- `cairn-core` is documented as having no I/O, and `cairn-server` depends on it. Putting
  adapters there would drag TOML editing and vendor parsing into the server's build for no
  reason.
- The daemon must not parse vendor configuration: it never reads `~/.codex/config.toml`, and
  making it do so would put file-format failures on the capture hot path.
- The whole SC-104 / SC-110 fixture corpus — 20+ realistic configuration files and every
  vendor payload — must be testable without a daemon, a socket, or a temporary Git
  repository. A crate with no SQLite and no async runtime requirement makes those tests
  plain, fast unit tests.

**Rejected**: putting it all in `crates/cairn` (the fixture corpus would then only be
testable through a binary that also owns the CLI, the hook runtime, and the MCP server);
splitting adapters across `cairn-core` and `cairn` (two homes for one boundary, and the
core's no-I/O rule breaks); giving each adapter its own crate (three crates that share a
change-plan engine and differ only in a parser — abstraction for its own sake, and the
constitution's Principle II says no).

**Consequence for the hook path**: `cairn` gains `toml_edit` and `include_dir` in its
dependency tree. Hooks never call into `cairn-integrate`, but binary size affects process
start. SC-122 measures it; if capture latency regresses against the Feature 001 baseline,
the fallback is to move the hook entry point into its own thin binary. Recorded as a risk,
not pre-emptively solved.

---

## D19 — The adapter boundary is data plus a narrow behavior trait

**Decision**: Each adapter is a small Rust type implementing one trait with these
operations: `detect`, `capability_profile`, `plan` (desired state → change plan),
`inspect` (filesystem → observed resource states), and `normalize` (vendor payload →
canonical lifecycle event). Everything else — the scope matrix, the resource kinds, the
health conditions, the change-plan classification, the atomic write, the marker handling —
is shared code the adapters call, not code they reimplement.

Capability profiles are **static data per adapter**, refined at runtime only by what
detection actually observes (installed version, whether handlers are activated). They are
not discovered by probing.

**Why**: there are exactly four adapters and one manager, and three of the four share the
same shape of work. Behavior that differs — where a file lives, how a payload is parsed,
what an event means — is genuinely per-vendor. Behavior that does not differ — planning,
diffing, writing, verifying — belongs in one place, so a bug in atomic replacement is fixed
once and every adapter inherits the fix. Static capability data keeps FR-108 honest: a
capability is present because the vendor documents it, not because a probe happened to
succeed.

**Rejected**: a registry/plugin framework with dynamic adapter discovery (Principle II —
building an extension mechanism for vendors that do not exist yet); giving each adapter its
own file-writing code (five chances to get atomicity wrong); probing for capabilities (a
probe that fails for an unrelated reason silently downgrades an integration).

---

## D20 — Canonical lifecycle, and what each adapter does not emit

**Decision**: Seven canonical events, mapped as below. An adapter emitting nothing for a
row is a recorded fact, not a gap to fill.

| Canonical | Claude Code | Codex | OpenCode |
|---|---|---|---|
| session opened | `SessionStart` (`source`: startup/resume/clear/compact/fork) | `SessionStart` (`source`: startup/resume/clear/compact) | `session.created`, or first plugin activity for an unseen `sessionID` |
| tool succeeded | `PostToolUse` | `PostToolUse`, classified success from `tool_response` | `tool.execute.after` |
| tool failed | `PostToolUseFailure` (carries `error`) | `PostToolUse`, classified failure from `tool_response` (D23) | **not emitted** — see below |
| agent quiesced | `Stop` | `Stop` | `session.idle` |
| context compacting | `PreCompact` (`trigger`) | `PreCompact` (`trigger`) | `experimental.session.compacting` hook |
| context compacted | `PostCompact` | `PostCompact` | `session.compacted` |
| session closed | `SessionEnd` (`reason`) | `SessionEnd` (`reason` is a constant, D24) | **not emitted** — no such signal exists |

Deliberately **not** mapped:

- OpenCode `session.idle` → session closed. It means the session went quiet. (FR-116)
- OpenCode `session.deleted` → session closed. Deleting a session is not completing it, and
  a `completed` Cairn session with a durable handoff is a claim about work, not about a
  record's existence.
- Any subagent event → a Cairn session. Subagents are inside one agent session; treating
  them as sessions would multiply provenance for one unit of work. Claude's
  `SubagentStart`/`SubagentStop` and Codex's equivalents are ignored in this feature.
- Claude `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `Notification`,
  `PostToolBatch`, `StopFailure`, and the file/config/workspace events. Cairn registers only
  the events its canonical lifecycle needs (FR-102 story scenario, US2 #6).

**OpenCode tool failure**: `tool.execute.after` receives `{title, output, metadata}` with no
outcome flag, and a tool that throws may not reach the hook at all. The adapter therefore
does not claim a distinguishable tool-failure capability for OpenCode. Where the output
carries an unambiguous failure marker the adapter may record a failure; where it does not,
it records the call without asserting one (FR-117). The capability profile says
`tool_failure: absent`.

**Why**: the mapping is derived from each vendor's own event vocabulary and payload, not
from name similarity. Every "not emitted" above is a capability the profile reports as
absent, which is what makes the integration level computed rather than asserted.

---

## D21 — "Agent quiesced" rather than "turn completed"

**Decision**: The canonical checkpoint event is **agent quiesced**: the agent has stopped
working and is waiting. It asserts nothing about success and nothing about the session
being over. Behavior is byte-for-byte Feature 001's `Stop` handling — flush pending capture,
record the checkpoint, session stays `active`, no durable handoff (FR-032).

**Why**: the three vendor signals do not mean the same thing. Claude's `Stop` fires when the
agent finished responding. Codex's `Stop` fires when the turn stopped, and Codex has a
separate `StopFailure`-shaped concern. OpenCode's `session.idle` fires when the session
became idle, which happens after an error just as readily as after a completed answer. The
only claim true of all three is "it stopped working and is waiting". Naming the canonical
event after the strongest of the three would have imported a claim the weakest cannot
support — the exact failure mode FR-115 exists to prevent.

**Rejected**: requiring extra payload evidence before OpenCode emits a turn-completed event
(option B in the finding). It would mean either dropping most OpenCode checkpoints or
inventing an inference from `message.*` events; the checkpoint is valuable precisely because
it is cheap and frequent, and it never needed the stronger claim in the first place.

---

## D22 — Sealed session close: durability before acknowledgment

**Decision**: Session close becomes two phases inside the daemon.

1. **Seal** (synchronous, before the reply): one small transaction sets the session's
   terminal status, end reason, `ended_at`, and a new `handoff_pending` flag. No Git, no
   capture quiesce, no synthesis. The daemon replies here.
2. **Synthesize** (asynchronous, immediately after): quiesce in-flight captures, build the
   handoff, write it, clear `handoff_pending`.

Daemon-start reconciliation gains one rule: any session with `handoff_pending` set gets its
handoff synthesized from what was recorded. `cairn session end` from the command line keeps
the old behavior and waits for the handoff, because nothing is holding a deadline over it.

**Why**: Codex's session-end handler has a **1-second default and 3-second maximum** budget
(D31). Today's path does `quiesce_captures` (up to 500 ms) plus a `git status` plus several
queries before it answers, which can exceed that and would make Codex's completion guarantee
unprovable. Sealing first keeps the guarantee real: the fact that the session terminated is
durably recorded before Cairn says yes, and the handoff — the expensive part — is
recoverable if the process dies between phases. FR-032's promise that a session never ends
without a handoff becomes *durably, eventually*, with `handoff_pending` as the proof
obligation rather than an assumption.

**Rejected**: keeping the synchronous path and hoping it fits (SC-128 would be a coin flip
on a busy machine); making the handoff optional at session end (it is the product); a
background timer that sweeps pending handoffs on an interval (the daemon already has a
deterministic boundary at start, and one more scheduled job earns nothing).

---

## D22a — The existing idle reaper is a safety net, not a completion mechanism

**Decision**: `recover.rs` already closes any session nothing has driven for two hours
(`IDLE_SESSION_TIMEOUT`), writing a `recovered` handoff and marking the session
`interrupted`; daemon start reconciles still-`active` sessions the same way. Both keep
working exactly as they are, and **neither may set `completion_guarantee`** or contribute to a
FULL classification for any agent. A session closed by either is reported as closed by
inactivity, never as completed.

**Why**: without this rule the completion guarantee is trivially satisfiable — every agent,
including one that signals nothing at all, eventually has its sessions closed by the timeout.
OpenCode would then be FULL on the strength of a two-hour clock, which is precisely the
overclaiming FR-207 exists to prevent. Silence is evidence that nothing happened, not evidence
that a session ended.

**What recovery is still for**: backstopping a completion boundary that was missed or that
failed — a crashed agent, a daemon that was down at `SessionEnd`, a sealed close whose second
phase never ran. That is a genuine and necessary job, and it is why the reaper stays.

**Rejected**: deleting the reaper to remove the ambiguity (two `active` sessions in one
worktree make `cairn context` ambiguous, which is the exact failure Feature 001 built it to
avoid); a shorter timeout (does not change what the signal means); counting recovery towards
FULL when no vendor signal exists (dishonest by construction).

---

## D23 — Codex tool-failure classification

**Decision**: The Codex adapter classifies success and failure from `PostToolUse`'s
`tool_response` value, in this order: an explicit non-zero `exit_code`; an explicit
`success: false` or `error` field; otherwise **success**. A response the adapter cannot
interpret produces the success-shaped observation for the tool's canonical category, never
an `error` observation.

**Why**: Codex has no `PostToolUseFailure` (D31). Feature 001's rule that Cairn never infers
a failure from a success payload was written when a dedicated failure event existed; the
underlying principle — do not assert a failure the payload does not establish — is what
FR-117 preserves. Defaulting to failure on an unrecognized shape would fill memory with
fictional errors, which is worse than missing some real ones.

**Rejected**: treating every ambiguous Codex response as a failure; adding a fourth
observation outcome for "unknown" (the existing `outcome` field already carries `unknown`
for tests, and the observation type is what handoffs read).

---

## D24 — Codex hook trust is a first-class state, not an error

**Decision**: `installed_not_activated` is a health condition of its own. `cairn connect
codex` writes the hooks, then reports that Codex will not run them until the user trusts
them inside Codex, with the exact step. The integration level reflects what actually works
until then (FR-209). Upgrading a Cairn hook invalidates its trust, so repair reports
`installed_not_activated` again after an upgrade rather than treating it as a regression.

**Why**: Codex computes a hash over each hook's command and configuration and refuses to run
any non-managed hook whose `trusted_hash` does not match, defaulting to `Untrusted` for a
hook it has never seen (D31, `discovery.rs`). There is no supported way for a third party to
pre-trust a hook, and forging the trust hash into a user layer would be exactly the kind of
private-state write FR-232 prohibits. So the honest design surfaces the state.

**Rejected**: writing `hooks.state.<key>.trusted_hash` ourselves (defeats a security control
the vendor built deliberately); reporting the integration connected and letting the user
discover the silence.

---

## D25 — Ownership markers and ownership identity

**Decision**: Two mechanisms, one for prose and one for structured configuration.

**Managed instruction block** (Markdown):

```markdown
<!-- cairn:managed:begin id=agent-contract schema=1 content=8f2b19c40a7d -->
… rendered contract body …
<!-- cairn:managed:end id=agent-contract -->
```

- `id` names the resource. `schema` is the contract schema version (D26). `content` is the
  first 12 hex characters of the SHA-256 of the **normalized** rendered body.
- Locating the block requires the full literal `<!-- cairn:managed:begin id=` prefix. The
  substring `cairn` is never a matching rule (FR-139).
- Exactly one `begin` and one matching `end`, in that order, with equal `id` → valid.
  Anything else — missing end, two begins, mismatched `id`, end before begin — is
  `damaged_markers`, and Cairn changes nothing (FR-137).
- Claude Code strips block-level HTML comments before instructions reach the model (D30), so
  the markers cost zero context there. In `AGENTS.md` they render as normal Markdown
  comments and are invisible in rendered views.

**Structured configuration entries and generated files**: ownership is the pair
*(reserved name, recorded canonical hash)*. Cairn writes its MCP server under the reserved
name `cairn`, its Codex hooks under keys it records, and its OpenCode plugin at a
Cairn-owned path. The local integration record stores the canonical hash of exactly what
Cairn wrote. On inspection: name present and hash matches → `healthy`; name present and hash
differs → `modified`; name present with no record → `external` (never adopted, never
deleted); name absent with a record → `missing`.

**Why**: prose needs an in-band marker because there is nowhere else to put it; structured
configuration must not carry Cairn-specific keys a vendor's schema validator might reject —
Codex's config structs use `deny_unknown_fields` in places, and Claude's hook entries have a
fixed shape. Hashing what we wrote gives exact ownership without polluting anyone's schema,
and it is what makes FR-223's semantic comparison implementable: the hash is taken over a
canonical normalization, so re-ordering keys or reformatting does not read as an edit.

**Rejected**: a sentinel key inside vendor config objects (schema risk, and it would survive
uninstall in a file we no longer own); marker-free diffing against a regenerated expectation
(cannot distinguish "user edited it" from "Cairn's template changed"); any rule involving
searching for the word `cairn`.

---

## D26 — Version scheme

**Decision**: Four independent version lines, none tied to the package version.

| Line | Form | Bumped when |
|---|---|---|
| `contract_schema` | integer | The contract's structure or marker format changes |
| `contract_revision` | 12-hex content digest | The rendered contract text changes at all |
| `skill_schema` | integer | The Skill's file layout or frontmatter shape changes |
| `skill_revision` | 12-hex content digest | Any Skill file's content changes |
| `desired_state_schema` | integer | The desired-state model's fields change |
| `adapter_version` | integer per adapter | The artifacts that adapter writes change shape |

The local integration record's own schema rides `cairn-store`'s existing migration version;
it needs no separate number.

**Upgrade rule**: repair upgrades an installed artifact when either its schema or its
revision differs from the current build. A schema change may require a migration step; a
revision-only change is a content refresh and is always safe. `cairn 0.1.0-alpha.5` running
`contract_schema=1, skill_schema=1, desired_state_schema=1` is the normal case and produces
no churn.

**Why**: tying artifact versions to the package version would rewrite every developer's
`CLAUDE.md` on every patch release — and on Codex it would additionally invalidate hook
trust every time (D24). Splitting schema from revision separates "the shape changed, handle
it" from "the words changed, refresh it".

**Rejected**: one version for everything (couples unrelated things); semantic versions per
artifact (three-part versions imply a compatibility contract nobody consumes); no revision
digest (content drift would be undetectable without a schema bump).

---

## D27 — Installation scope, resolved per resource

**Decision**: the matrix in [`contracts/scope-matrix.md`](./contracts/scope-matrix.md), with
this rule behind it: **prefer a project-local location the developer does not commit; if the
agent has none, use per-user; never fall back to a committed file silently** (FR-215,
FR-218).

Concretely: Claude Code has `.claude/settings.local.json`, so its lifecycle defaults there.
Codex and OpenCode have no project-local-ignored location, so their lifecycle installs
per-user — machine-local activation that imposes nothing on collaborators — and Cairn says
so. `--shared` writes committed project configuration for teams that want it.

**Why**: a per-user lifecycle handler fires in every repository, including ones Cairn has
never seen. That is acceptable only because the hook already fails soft outside a Cairn
project and returns immediately; it is measured by SC-122 rather than assumed. The
alternative — committing a hook that runs a local binary into a shared repository by
default — is the one FR-215 rules out.

**Rejected**: one universal default (the four resource kinds have genuinely different
audiences); committing lifecycle by default as Feature 001 did (imposes Cairn on every
collaborator of every repository a single developer connects).

---

## D28 — One Skill copy per machine, with sharing detected

**Decision**: The Cairn Skill is installed at each agent's own canonical per-user Skill
directory, **except** where one agent already reads another's directory. OpenCode scans
`~/.claude/skills/**/SKILL.md` (D32), so when Claude Code's Skill is installed and current,
OpenCode's Skill resource is recorded as `satisfied_by: claude-code` and no second copy is
written. If Claude Code is later disconnected, doctor reports OpenCode's Skill `missing` and
repair installs OpenCode's own copy.

**Why**: OpenCode's skill loader keys on the skill's `name` and logs a conflict when two
locations declare the same one. Two physical copies would therefore produce a warning and a
non-deterministic winner — worse than either one copy or none. Detecting the sharing keeps a
single effective Skill, and making the post-disconnect gap a *reported, repairable*
condition is better than a symlink whose behavior across the three agents is unverified.

**Rejected**: symlinking one canonical copy into each agent's directory (Claude Code
documents symlink support; Codex and OpenCode do not, and Windows makes it worse — an
unverified mechanism at the centre of the design); installing per-project copies (the Skill
teaches generic Cairn workflows, FR-217); a copy per agent regardless (name conflict).

---

## D29 — Where the Skill and the contract physically live

**Decision**: The Skill's canonical source is `skills/cairn/` at the repository root,
embedded into the binary at build time. The always-on contract's canonical source is
`crates/cairn-integrate/assets/agent-contract.md`, embedded the same way. Both renderings —
the managed instruction block and the MCP `instructions` string — are generated from the
contract asset by one function, and a test asserts the two renderings state the same rules
(FR-123).

CC Switch distributes Skills by cloning a public Git repository at
`owner/name` + `directory` + `branch` (D33). Pointing it at `Vellixia/Cairn`, directory
`skills/cairn`, at a released tag makes the repository path *the* source — there is no second
copy to drift.

**Why**: FR-123 and FR-141 both demand one canonical source; the CC Switch requirement adds
that the Skill must be fetchable from a public Git path. Putting the canonical files at a
repository path that is both embedded and fetchable satisfies all three without duplication.

**Rejected**: a separate `Vellixia/cairn-skills` repository (a second release process and a
second thing to keep in step); generating the Skill at install time from Rust string
literals (nothing for CC Switch to clone, and the Skill becomes unreviewable in diffs).

---

## D30 — What Claude Code actually provides

Verified against `code.claude.com/docs` (hooks, mcp, skills, memory), 2026-08-11.

- **Hooks** live in `~/.claude/settings.json` (user), `.claude/settings.json` (project,
  committed), `.claude/settings.local.json` (project, gitignored), managed policy settings,
  and plugin `hooks/hooks.json`. Entry shape is `{matcher, hooks: [{type: "command",
  command, args?, timeout?}]}`.
- The event set is much larger than Feature 001 uses and now includes `PostCompact`,
  `SubagentStart`/`SubagentStop`, `Setup`, `StopFailure`, `PostToolBatch`, and file,
  configuration, and workspace events. `PostToolUseFailure` exists and carries `error`.
- Common payload fields: `session_id`, `prompt_id`, `transcript_path`, `cwd`,
  `permission_mode`, `hook_event_name`. Tool events add `tool_name`, `tool_input`,
  `tool_use_id`, plus `tool_response` (success) or `error` (failure).
- `SessionStart` accepts `hookSpecificOutput.additionalContext`, capped at 10,000
  characters. `SessionStart.source` includes `fork` as well as startup/resume/clear/compact.
- **`Stop` now carries `last_assistant_message` and `tool_calls`** — conversation content
  Feature 001's payload did not include. See D34.
- **MCP** scopes: local (`~/.claude.json`, per project path), project (`.mcp.json`,
  committed), user (`~/.claude.json`, all projects). Precedence local > project > user,
  matched by name, whole entry wins, no field merging. Project-scoped servers require
  interactive approval.
- **Instructions**: `./CLAUDE.md` or `./.claude/CLAUDE.md`, plus `.claude/rules/*.md`.
  **Claude Code reads `CLAUDE.md`, not `AGENTS.md`.** Block-level HTML comments are stripped
  before the content reaches the model.
- **Skills**: `~/.claude/skills/<name>/SKILL.md` (personal), `.claude/skills/<name>/SKILL.md`
  (project), plugin. Frontmatter has no `version` field; a free-form `metadata` map is
  accepted and not acted upon.

---

## D31 — What Codex actually provides

`developers.openai.com` is blocked by this environment's egress policy, so this was verified
against the `openai/codex` repository at `main`, 2026-08-11.

- **Events** (`codex-rs/protocol/src/protocol.rs`, `HookEventName`): `PreToolUse`,
  `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SessionStart`,
  `SessionEnd`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop`, `Stop`. **There is no
  `PostToolUseFailure`.**
- **Payloads** (`codex-rs/hooks/src/schema.rs`): every event carries `session_id` (a stable
  thread identifier), `cwd`, `transcript_path`, `hook_event_name`; turn-scoped events add
  `turn_id`, `model`, `permission_mode`, and optional `agent_id`/`agent_type`.
  `PostToolUseCommandInput` carries `tool_name`, `tool_input`, `tool_response`,
  `tool_use_id`. `SessionStart` carries `source`; `SessionEnd` carries `reason`.
- **Session-end budget** (`codex-rs/hooks/src/events/session_end.rs`):
  `SESSION_END_DEFAULT_TIMEOUT_SEC = 1`, `SESSION_END_MAX_TIMEOUT_SEC = 3`, and
  `SESSION_END_REASON` is the constant `"other"` — the reason is not currently
  differentiated.
- **Trust gating** (`codex-rs/hooks/src/engine/discovery.rs`): each hook is hashed over its
  event, matcher, group and configuration; a non-managed hook runs only when it is `enabled`
  *and* its `trusted_hash` matches. A hook Codex has never seen has no trusted hash and is
  `Untrusted`; an edited hook becomes `Modified`. Trust state lives under `hooks.state` in a
  User or session layer.
- **Configuration layers** (`codex-rs/config/src/loader/README.md`): system <
  enterprise-managed < user (`~/.codex/config.toml`) < user profile < **project
  (`.codex/config.toml`)** < session flags < legacy managed. Hooks additionally load from
  `<config folder>/hooks.json`.
- **MCP** is configured under `[mcp_servers.<name>]` with `command`/`args`/`env` for stdio.
- **Skills** exist with `User | Repo | System | Admin` scope; the per-user root is
  `CODEX_HOME/skills`.

---

## D32 — What OpenCode actually provides

`opencode.ai` is blocked by this environment's egress policy, so this was verified against
the `sst/opencode` repository at `dev`, 2026-08-11.

- **Plugin hooks** (`packages/plugin/src/index.ts`): `event`, `config`, `tool`, `auth`,
  `provider`, `chat.message`, `chat.params`, `chat.headers`, `permission.ask`,
  `command.execute.before`, `tool.execute.before`, `tool.execute.after`, `shell.env`,
  `tool.definition`, and the experimental `session.compacting`, `compaction.autocontinue`,
  `chat.messages.transform`, `chat.system.transform`, `text.complete`. There is **no**
  session-start or session-end hook.
- **Session events** reach plugins through the `event` bus
  (`packages/sdk/js/src/gen/types.gen.ts`): `session.created`, `session.updated`,
  `session.deleted`, `session.idle`, `session.status`, `session.compacted`, `session.error`,
  `session.diff`. **There is no session-ended event.** `session.idle` carries only
  `{sessionID}`.
- `tool.execute.after` receives `{tool, sessionID, callID, args}` and `{title, output,
  metadata}` — **no outcome flag**.
- **Plugins are auto-discovered from files**: `ConfigPlugin.load` globs
  `{plugin,plugins}/*.{ts,js}` inside every config directory. So a plugin can be installed by
  writing one file — no configuration edit at all.
- **Config directories**: `~/.config/opencode`, every `.opencode` directory walking up from
  the working directory to the worktree root, and `~/.opencode`. **Config files**:
  `opencode.jsonc` and `opencode.json`, walked up and merged nearest-last.
- **MCP** is the `mcp` map with `{type: "local", command: string[]}` or `{type: "remote",
  url}`.
- **Instructions**: global `~/.config/opencode/AGENTS.md` and `~/.claude/CLAUDE.md`; project
  `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md` (deprecated), **first project match wins** rather
  than stacking; plus everything in the `instructions` config array.
- **Skills** are scanned from `.opencode/{skill,skills}/**/SKILL.md` in config directories,
  from `~/.claude/skills/**/SKILL.md` and `~/.agents/skills/**/SKILL.md` and their project
  equivalents, and from configured `skills.paths` / `skills.urls`. Duplicate skill **names**
  across locations produce a conflict warning.
- OpenCode injects MCP server `instructions` into its system prompt inside an
  `<mcp_instructions>` block (`packages/opencode/src/session/system.ts`) — so MCP-level
  instructions demonstrably reach the model there.

---

## D33 — What CC Switch actually provides

Verified against `farion1231/cc-switch` documentation at `main`, 2026-08-11.

- A desktop configuration manager for Claude Code, Claude Desktop, Codex, Gemini CLI, Grok
  Build, OpenCode, OpenClaw, and Hermes. **Not a coding agent.**
- **MCP distribution targets are global**: Claude → `~/.claude.json` `mcpServers`; Codex →
  `~/.codex/config.toml` `[mcp_servers]`; Gemini → `~/.gemini/settings.json`; OpenCode →
  `~/.config/opencode/opencode.json` `mcp`; Hermes → `~/.hermes/config.yaml`. OpenClaw and
  Claude Desktop are not MCP-sync targets.
- **Skill installation** is per-user: `~/.claude/skills/`, `~/.codex/skills/`,
  `~/.gemini/skills/`, `~/.config/opencode/skills/`, `~/.hermes/skills/`, sourced from a
  **GitHub repository** and materialized from `~/.cc-switch/skills/` by symlink or copy.
- **Own state**: `~/.cc-switch/cc-switch.db` (SQLite), `settings.json`, `skills/`,
  `backups/`. This is private storage.
- **The documented third-party interface is a deep link**:
  `ccswitch://v1/import?resource=provider|mcp|prompt|skill&…`. For MCP: `apps` (comma
  separated), `config` (JSON), `name`. For Skill: `name`, `repo` (`owner/name`),
  `directory`, `branch`. Every import shows a confirmation dialog.
- **There is no documented removal or query interface.** Import is the whole public surface.

---

## D34 — MCP protocol revision: stay at 2025-06-18

**Decision**: Keep `2025-06-18` as the advertised revision and keep the accepted set
(`2025-06-18`, `2025-03-26`, `2024-11-05`) unchanged. Add the `instructions` field to the
initialize result, which that revision already defines.

**Why**: `InitializeResult.instructions` exists in `2025-06-18` — verified in the
specification's own `schema/2025-06-18/schema.ts` — so nothing about carrying the usage
contract requires an upgrade. The current latest revision is `2025-11-25`, and its changelog
lists nothing Cairn's stdio, six-tool, no-sampling, no-elicitation server would implement:
OpenID Connect discovery, incremental OAuth scope consent, tool/resource icons, elicitation
enum shapes, URL elicitation, sampling tool calls, OAuth client-ID metadata documents, and
experimental tasks. Advertising a revision to gain nothing, while asserting support Cairn has
not exercised against real clients, is exactly the dishonesty FR-130 forbids.

**Rejected**: advertising `2025-11-25` because it exists; adding it to the accepted list
"just in case" (a client that asks for it and then uses one of its features would be
misled).

---

## D35 — Privacy of newly exposed payload fields

**Decision**: an explicit allow-list per event. Fields not on it are read for routing and
discarded, never persisted.

| Field | Read | Retained |
|---|---|---|
| `session_id` / `sessionID` / thread id | yes | yes — the agent session key |
| `cwd` | yes | as repository state, per Feature 001 |
| `tool_name` | yes | yes — normalized, ≤64 chars, as bounded provenance (D36) |
| `tool_input.file_path`, `.command` | yes | yes — via the existing exclusion → redaction → bound pipeline |
| `tool_response` | yes | only the derived outcome and exit code |
| `error` | yes | only the bounded, redacted failure summary |
| `transcript_path` | no | never — a path to conversation content |
| `last_assistant_message` | **no** | never |
| `tool_calls` (Claude `Stop`) | **no** | never |
| `prompt` / `user_prompt` | **no** | never |
| `model`, `permission_mode`, `turn_id`, `agent_id`, `agent_type` | no | never |
| `metadata` (OpenCode tool output) | no | never |
| `output` (OpenCode tool output) | yes | only a derived outcome, never the text |

**Why**: Claude's `Stop` payload now carries the full assistant message and the turn's tool
calls, and Codex's carries `last_assistant_message`. Feature 001 forbids persisting
conversations (FR-048). A new adapter must not become the route around that, so the
allow-list is stated per field rather than left to each adapter's judgment. `transcript_path`
is excluded even though it is only a path, because Cairn has no reason to hold a pointer to
the transcript and every reason not to.

---

## D36 — Raw vendor tool names as bounded provenance

**Decision**: Reuse Feature 001's `classify_tool` and `is_test_command` unchanged, extended
with the vendor names the new adapters see (Codex `shell`/`apply_patch`; OpenCode
`bash`/`edit`/`write`/`read`/`glob`/`grep`). The raw vendor tool name is stored on the
observation in a new optional column, normalized to `[A-Za-z0-9_.-]`, truncated to 64
characters, redacted like every other field, and never used by ranking, handoff synthesis,
or context assembly.

**Why**: FR-120–FR-122. The classification table already exists and works; a parallel
taxonomy would be a second thing to keep correct. The raw name is genuinely useful for
diagnosing a mapping bug — "which vendor tool produced this odd observation" — and 64
characters of an identifier-shaped string is a bounded, low-risk field.

**Rejected**: storing the whole `tool_input` as provenance (raw payload, FR-012);
introducing canonical categories parallel to the existing observation types (no demonstrated
gap).

---

## D37 — Configuration editing per format

**Decision**:

| Format | Where | Mechanism |
|---|---|---|
| JSON | `.claude/settings*.json`, `.mcp.json`, `~/.claude.json`, `opencode.json` | `serde_json` with `preserve_order`, read → mutate the owned node → write pretty, atomic replace |
| TOML | `~/.codex/config.toml`, `.codex/config.toml` | **`toml_edit`** — document-preserving, keeps comments, ordering, and formatting |
| JSON with comments | `opencode.jsonc` | **not written.** Cairn writes `opencode.json`, which OpenCode merges alongside it. If a `.jsonc` already declares a Cairn resource, that is reported as a conflicting owner rather than edited (D38) |
| Markdown | `CLAUDE.md`, `AGENTS.md` | marker-delimited block splice (D25); everything outside the markers is copied byte-for-byte |
| Generated files | `~/.config/opencode/plugin/cairn.js`, `SKILL.md` trees | written whole; Cairn owns the entire file |

Every write is atomic: write to a temporary file in the same directory, `fsync`, rename over
the target (FR-154). The original is untouched until the rename succeeds, which is what makes
the whole-file backup unnecessary (FR-156, D39).

**Why**: `toml_edit` is the crate Cargo itself uses for exactly this problem and is the only
mature option for preserving a TOML document's comments and layout. JSON has no comments to
preserve and `serde_json`'s `preserve_order` feature keeps key order stable, which is what
FR-152 actually needs there. JSONC is the one format where a faithful round-trip is not
available from a maintained crate, and OpenCode's own merge semantics give us a way to avoid
needing one.

**Rejected**: `toml` (rewrites the document and drops comments — FR-152 violation);
hand-written string splicing into structured formats (FR-153 forbids it); adopting a
low-maturity JSONC editor at the centre of a safety-critical path.

---

## D38 — When precedence makes a change unsafe

**Decision**: Before writing, the planner computes which location would be **effective**
under the agent's own precedence rules, and refuses to proceed automatically when the write
would not be effective or would be ambiguous. Two known cases:

- A Cairn MCP entry exists at Claude's local scope (`~/.claude.json` for this project path)
  while Cairn is asked to install at user scope: local wins, so the user-scope write would be
  inert. Reported as `conflicting_owner`, not written.
- A Cairn resource is declared in `opencode.jsonc`, which OpenCode merges after
  `opencode.json`: a write to `opencode.json` would be shadowed. Reported, not written.

The same rule governs migration overlap (FR-148): overlap is permitted only where precedence
makes exactly one side effective for the whole window.

**Why**: FR-146 and FR-148 are about the developer's *effective* configuration, not about
file contents. A write that lands in a file the agent ignores is a silent failure, and
Cairn's promise is to fail loudly instead.

---

## D39 — Recovery artifacts

**Decision**: `$CAIRN_HOME/recovery/<agent>/<resource-kind>/<RFC3339-utc>-<content-hash>.txt`
holds the **Cairn-owned prior content only** — the managed block body, the canonical
serialization of the owned entry, or the whole file where Cairn generated the whole file. A
sibling `.json` sidecar records agent, resource kind, source path, timestamp, and the hash.
Retention: the ten most recent per (agent, resource kind); older ones are pruned when a new
one is written. Never synced, never logged (the path may be logged; the content never is),
and subject to the same redaction as any stored content.

**Why**: FR-156 originally asked for a copy of every modified file, which would have
duplicated `~/.claude.json` — a file that holds credentials — into Cairn's own storage.
Atomic replacement already provides crash safety, so the only thing a copy adds is *undo* of
a deliberate forced overwrite. Scoping the copy to Cairn's own content gives the undo without
holding anything Cairn is forbidden to hold.

**Rejected**: a general backup subsystem with restore commands (an elaborate answer to a
narrow need); no artifact at all (a forced repair would then be genuinely destructive,
against FR-222).

---

## D40 — Test topology

**Decision**: Five tiers, and only the first four are required in CI.

1. **Unit** — inside `cairn-integrate`: marker parsing, canonical hashing, semantic
   equivalence, capability computation, level derivation, change-plan classification.
2. **Configuration fixtures** — the ≥20-file corpus in
   `crates/cairn-integrate/tests/fixtures/`, each with a declared ownership expectation.
   Connect → disconnect must return every non-Cairn byte unchanged (SC-104).
3. **Lifecycle fixtures** — recorded vendor payloads in
   `tests/integrations/{claude-code,codex,opencode,cc-switch,generic-mcp}/`, asserting the
   canonical event produced and, for every capability the profile does not claim, that
   nothing is produced (SC-110).
4. **Daemon integration** — in the existing `tests` crate: cross-agent continuity,
   concurrency, sealed session close, recovery.
5. **Live agent** — manual release evidence with real, authenticated Claude Code, Codex, and
   OpenCode. Never required for CI.

CI-required tests use no vendor binary, no authentication, and no network.

**Why**: this is the split FR-203–FR-205 require, and it maps cleanly onto where the code
lives (D18). Tier 3's "assert nothing is produced" half is what makes the honesty
requirements testable rather than aspirational.

---

## D41 — Cross-agent continuity proved without vendor binaries

**Decision**: The release-blocking continuity scenario runs in the `tests` crate against a
real daemon and a real temporary Git repository, driving each adapter's `normalize` with
recorded vendor payloads rather than launching an agent. The three "sessions" are three
distinct `agent_session_key`s with three different `agent` values, exercised through the same
daemon requests the real hooks send.

**Why**: the thing under test is Cairn's project resolution, memory scoping, provenance, and
handoff — none of which depends on a real agent being present. Requiring three authenticated
vendor CLIs in CI would make the most important test in the feature the least likely to run.
The live version stays as release evidence, where a human can confirm the payloads still look
like the fixtures.

---

## D42 — Playwright in hosted CI

**Decision**: A new `web-e2e` job: PostgreSQL service container, `cargo build --release -p
cairn-server`, start it on `127.0.0.1:8080`, `npm ci && npm run build && npm start` on
`127.0.0.1:3100` with `NEXT_PUBLIC_CAIRN_API` pointed at the server, then `npx playwright
test` with both existing projects. Chromium only, via `npx playwright install --with-deps
chromium`. Traces uploaded on failure. Job timeout 20 minutes. Added to required checks.

No database fixture is needed: `web/e2e/seed.ts` registers a fresh user and seeds through the
real API on every run, so each run is already isolated and deterministic.

**Why**: the release build is not optional — `playwright.config.ts` documents that an
unoptimized Argon2 verify costs ~0.7 s against ~0.03 s released, which under parallel workers
reads as a flaky UI. The existing web job stays as-is for lint, typecheck, and build; e2e is a
separate job so a browser failure is distinguishable from a build failure at a glance.

**Rejected**: running against `next dev` (slower and not what ships); adding Firefox and
WebKit (triples the runtime to protect a surface no user has reported); a seeded SQL dump
(the API-driven seed already exists and cannot drift from the API).

---

## Open questions carried into implementation

- Whether a per-user Codex hook and a per-user OpenCode plugin add measurable cost in
  repositories Cairn does not manage. SC-122 measures it; if it does, `--shared` project
  installation becomes the recommended default for those two and D27's matrix is revised.
- Whether Codex's `SessionEnd` reason stays a constant. If it becomes differentiated, the
  adapter should carry it into the handoff's boundary record; nothing else changes.
- Whether any client other than OpenCode surfaces MCP `instructions`. Cairn treats delivery
  as best-effort either way (FR-129); the answer only affects how much the generic MCP path
  can be relied on in practice.

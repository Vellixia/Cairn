# Connecting agents

Cairn edits files you own — your agent's settings, its hook registrations, your
`CLAUDE.md`. So the whole integration surface is built around one promise: it tells you
what it will change before it changes anything, it changes only what it wrote, and it can
take all of it back.

- [What "connected" actually means](#what-connected-actually-means)
- [Connecting](#connecting)
- [Where things go, and why](#where-things-go-and-why)
- [`--shared`](#--shared)
- [Checking and fixing](#checking-and-fixing)
- [Removing](#removing)
- [Distributing through CC Switch](#distributing-through-cc-switch)
- [Cloning a repository installs nothing](#cloning-a-repository-installs-nothing)

## What "connected" actually means

Different agents can do different things, and Cairn reports what each one *actually*
provides rather than a single word.

```console
$ cairn agents
claude-code  2.1.220   verified               FULL
codex        0.58.0    compatible-unverified  MCP_PLUS   (awaiting a first session closed here)
opencode     1.4.2     compatible-unverified  MCP_PLUS   (no automatic session completion)
             this agent signals no session end, so sessions here are closed by Cairn's
             inactivity timeout rather than completed
generic-mcp  -         compatible-unverified  MCP_ONLY   (no automatic session start)
```

| Level | What you get |
|---|---|
| `FULL` | Sessions open, capture, checkpoint and complete automatically, and Cairn has *observed* all of it on this machine |
| `MCP_PLUS` | The tools, the usage contract, and some of the lifecycle — the report names exactly which parts are missing |
| `MCP_ONLY` | The six tools and nothing automatic; you open and close sessions yourself |
| `UNSUPPORTED` | Detected, but no safe integration exists for this version — with the reason |

Two things about `FULL` are worth knowing. It is never granted because a vendor documents
a capability: it requires Cairn to have *seen* each one work here, so a newly connected
agent sits at `MCP_PLUS` with the awaited behaviors named until you run one ordinary
session. And it is withdrawn when your agent updates, because evidence about the previous
build is not evidence about this one — one more session restores it.

## Connecting

```bash
cairn connect                     # every agent Cairn detects
cairn connect claude-code         # one of them
cairn connect claude-code --dry-run   # show the plan, change nothing
```

A connect always prints its plan first — every file it would touch, what it would do to
each, and, explicitly, what it will not touch. Nothing is written until you confirm; in a
non-interactive shell pass `--yes`.

```console
$ cairn connect claude-code --dry-run
add       mcp           ~/.claude.json
add       lifecycle     .claude/settings.local.json
add       instructions  CLAUDE.md
add       skill         ~/.claude/skills/cairn

untouched every other MCP server · every other lifecycle handler and plugin ·
          all content outside the cairn:managed markers ·
          every provider, credential and model setting
```

Running it twice changes nothing the second time. Cairn recognizes what it wrote by a
recorded hash of exactly those bytes, so reformatting your file, reordering its keys, or
adding your own entries beside Cairn's is not a change it will undo.

## Where things go, and why

Each of the four resources has its own default scope, because the right answer differs per
resource and per agent — one global choice would be wrong for something.

| Resource | Claude Code | Codex | OpenCode |
|---|---|---|---|
| **MCP server** | `~/.claude.json` (user) | `~/.codex/config.toml` (user) | `~/.config/opencode/opencode.json` (user) |
| **Lifecycle** | `.claude/settings.local.json` (project, gitignored) | `~/.codex/hooks.json` (user) | `~/.config/opencode/plugin/cairn.js` (user) |
| **Instructions** | `CLAUDE.md` (project, committed) | `AGENTS.md` (project, committed) | `AGENTS.md` (project, committed) |
| **Skill** | `~/.claude/skills/cairn` (user) | `~/.codex/skills/cairn` (user) | binds to Claude Code's copy where it exists |

The reasoning behind each default:

- **The MCP server is per user.** It points at the `cairn` binary on *your* PATH, which is
  a fact about your machine and not about the repository.
- **Lifecycle handlers default to the most local place that is not committed.** Claude Code
  documents a gitignored project settings file, so Cairn uses it: the hooks apply to this
  repository and reach nobody else. Codex and OpenCode have no such file, so their handlers
  are per user — machine-local, and stated at connect time rather than committed silently.
- **The instruction block is committed**, because it is the one resource your collaborators
  genuinely benefit from. It lives between `cairn:managed` markers, and everything outside
  them is yours.
- **The Skill is per user**, and where two agents read the same directory, they share one
  copy rather than getting two.

Override any of them per resource:

```bash
cairn connect claude-code --scope mcp=project_shared --scope skill=user
```

## `--shared`

`--shared` moves every resource that *can* be committed into the repository, so a
collaborator who clones it gets the same configuration you have.

```bash
cairn connect claude-code --shared --dry-run
```

| | Default | `--shared` |
|---|---|---|
| MCP server | `~/.claude.json` | `.mcp.json` (committed) |
| Lifecycle | `.claude/settings.local.json` (gitignored) | `.claude/settings.json` (committed) |
| Instructions | `CLAUDE.md` (committed) | `CLAUDE.md` (committed) |
| Skill | `~/.claude/skills/cairn` | unchanged — a Skill is a per-user install |

Two things follow, and Cairn says both before it writes:

1. **You are committing configuration on your collaborators' behalf.** Shared hooks run
   `cairn` on their machine; if they do not have it installed, their agent reports a
   missing command. Cairn requires an explicit agreement for exactly these resources
   rather than treating `--shared` as a formatting preference.
2. **A commit is not an installation.** Cloning the repository gives them the *files*.
   Nothing about Cairn is active on their machine until they run `cairn connect`
   themselves — see below.

## Checking and fixing

```bash
cairn doctor              # every agent
cairn doctor claude-code  # one of them
```

Doctor inspects what is actually on disk — never what Cairn's records claim — and reports
each resource as healthy, missing, outdated, modified, or in conflict, with the command
that fixes it. It exits non-zero when something needs your attention, so it is usable in a
script.

```console
$ cairn doctor
core        cli 0.1.0-alpha.2 · daemon 0.1.0-alpha.2 · schema 4 · project registered

claude-code  FULL
             mcp           healthy   ~/.claude.json
             lifecycle     outdated  .claude/settings.local.json
                           → cairn repair claude-code
             instructions  healthy   CLAUDE.md
             skill         healthy   ~/.claude/skills/cairn
```

`cairn repair` restores what Cairn owns and is unambiguous. It will not overwrite
something you edited by hand: that is reported as a conflict, with both versions named and
the choice left to you. `cairn repair --force` takes your edit, but preserves the previous
Cairn-owned content first and tells you where it put it.

## Removing

```bash
cairn disconnect claude-code
cairn disconnect claude-code --only lifecycle   # just the hooks
cairn disconnect claude-code --dry-run
```

Disconnect removes this agent's dependency on each resource. What that means depends on
who else is using it: a resource two agents share loses one binding and stays; the last
binding takes it with it.

**What disconnect never does**, and this is asserted by tests rather than promised:

- It deletes no project, task, session, observation, memory or handoff. Everything you
  learned with that agent is still there, and still available to every other agent.
- It modifies no MCP server, hook, plugin, instruction, credential or setting that Cairn
  did not write. Every unrelated byte in a file it edits is identical afterwards.
- It changes no other agent's configuration.
- It leaves your instruction file in place, with your content, minus Cairn's block.

## Distributing through CC Switch

If you manage your agents with [CC Switch](https://github.com/farion1231/cc-switch), Cairn
can hand it the MCP entry and the Skill to distribute:

```bash
cairn integration distribute --via cc-switch --resource mcp --apps claude,codex
```

Cairn opens CC Switch's own import link and stops there — you confirm the import inside CC
Switch, and `cairn doctor` verifies the result afterwards by reading the *target
applications'* configuration. Cairn never writes to CC Switch's own storage, for anything,
including checking whether an import worked.

There is no documented removal interface, so `cairn disconnect` reports the manual step
rather than inventing one.

## Cloning a repository installs nothing

A repository that someone connected with `--shared` carries Cairn's committed
configuration: an instruction block, possibly `.mcp.json`, possibly committed hooks.
Cloning it gives you those files and nothing else.

- No hook is registered on your machine.
- No daemon starts.
- Nothing is captured, and no project is created.
- Cairn does not read the repository's intent and apply it.

Committed configuration is *offered*, not applied. To take it up, run `cairn init` and
`cairn connect` yourself — at which point Cairn recognizes what is already there, tells you
so, and installs only what is missing.

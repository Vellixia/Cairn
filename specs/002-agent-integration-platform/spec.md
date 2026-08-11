# Feature Specification: Agent Integration Platform

**Feature Directory**: `specs/002-agent-integration-platform` (recorded in `.specify/feature.json`)

**Git Branch**: `claude/agent-integration-spec-r05mg4`

**Created**: 2026-08-11

**Status**: Clarified — ready for planning

**Input**: User description: "Cairn connects safely and consistently to multiple AI coding agents, teaches them how to use persistent project memory, normalizes their lifecycle into one Cairn session model, preserves shared project knowledge across agents, supports CC Switch-managed distribution, and can detect, repair, migrate, and remove its integrations without damaging user configuration."

## Overview

Feature 001 delivered persistent, project-aware memory and proved it end to end with one
agent: Claude Code. Everything downstream of the agent — sessions, observations, scoped
memory, context briefings, handoffs, tasks, the six-tool MCP surface, optional sharing — is
agent-neutral already. Everything upstream of it is not. Detection, connection, lifecycle
capture, and the instructions that teach an agent to use Cairn are all shaped around one
vendor.

Developers do not work that way. They run Claude Code today, Codex tomorrow, OpenCode for a
particular job, and they increasingly manage all of them through a configuration manager
such as CC Switch. Today, switching agents means losing the project's memory, because the
memory was never the agent's — it just could not be reached from anywhere else.

Feature 002 makes Cairn agent-independent. It introduces an adapter boundary so each agent's
own lifecycle is translated into one canonical Cairn lifecycle; a small versioned contract
that teaches any connected agent how to use Cairn; a capability model that reports honestly
what a given agent can and cannot do; an ownership model so a resource installed directly by
Cairn and a resource distributed by CC Switch never collide; and the diagnostic, repair,
migration, and removal behavior that makes changing all of this safe on a machine full of
configuration a developer did not ask Cairn to manage.

The promise this feature must keep is one sentence: **use different coding agents without
losing the project's memory.**

## User Scenarios & Testing *(mandatory)*

Priorities order delivery. Feature 002 is complete only when every story below is delivered;
a P2 story is later in sequence, not optional.

### User Story 1 - The agent understands Cairn (Priority: P1) 🎯 First usable slice

A developer connects an agent to Cairn. From the first session, the agent knows what Cairn
is, that it should read the context Cairn gives it before re-deriving the project, that it
should search existing memory before repeating an investigation, and which kinds of things
are worth remembering durably. It gets this without the developer writing any instructions,
and without Cairn dumping a manual into every session.

**Why this priority**: Every other story depends on the agent actually using Cairn. An agent
with tools it does not know when to call produces an empty memory store. This is the slice
that turns "Cairn is installed" into "Cairn is used", and it is independently valuable even
for an agent with no lifecycle integration at all.

**Independent Test**: Connect a supported agent to a scratch repository, start a session, and
ask the agent — with no further prompting — what it should do before investigating an
unfamiliar part of the codebase and what it should record when it makes a design decision.
Its answer must reflect the Cairn usage contract. Separately, assert that the always-on
instruction text stays within its size bound and that the deeper workflow material is not
loaded until it is needed.

**Acceptance Scenarios**:

1. **Given** a repository with no Cairn instructions, **When** the developer connects an
   agent that supports persistent instructions, **Then** Cairn installs a compact managed
   instruction block carrying the Cairn Agent Usage Contract, bounded to its configured size,
   and leaves every pre-existing instruction in that file byte-for-byte unchanged.
2. **Given** an agent that supports Skills, **When** it is connected, **Then** Cairn installs
   a versioned Cairn Skill covering the deeper workflows, and that material is not present in
   the always-on instruction block.
3. **Given** an agent connected only through MCP, **When** its client requests initialization
   and the protocol carries server instructions, **Then** Cairn returns the compact universal
   form of the same usage contract, so the agent receives basic correct behavior with no
   native adapter.
4. **Given** a connected agent, **When** the same connect operation is run again, **Then** the
   instruction block and the Skill are unchanged, not duplicated, and the operation reports
   `unchanged`.
5. **Given** an instruction file that a developer has since edited outside the Cairn block,
   **When** Cairn updates its own block, **Then** only the content between Cairn's ownership
   markers changes.
6. **Given** the same usage contract, **When** it is rendered for two different agents,
   **Then** both renderings state the same behavioral rules, because they are generated from
   one canonical source rather than maintained per agent.

---

### User Story 2 - An existing Claude Code user upgrades without damage (Priority: P1)

A developer who connected Claude Code under Feature 001 installs Feature 002 and reconnects.
The lifecycle capture they already rely on keeps working. Their configuration does not grow a
second copy of anything, and nothing they configured themselves is touched.

**Why this priority**: This is the only story with existing users to lose. A migration that
duplicates hooks, duplicates the MCP entry, or drops an unrelated setting is worse than not
shipping.

**Independent Test**: Take a repository configured by Feature 001 — six Claude Code hook
entries, the project MCP entry, plus unrelated user hooks, unrelated MCP servers, and
unrelated settings. Run the Feature 002 connect. Assert exactly one Cairn entry per hook
event, exactly one Cairn MCP entry, every unrelated entry preserved, and captured behavior
identical to before.

**Acceptance Scenarios**:

1. **Given** a repository connected under Feature 001, **When** the developer connects under
   Feature 002, **Then** the existing Cairn hook entries and MCP entry are recognized as
   Cairn-owned and updated in place rather than added alongside.
2. **Given** that repository also contains user-authored hooks on the same events and other
   MCP servers, **When** the migration runs, **Then** those entries are byte-identical
   afterwards.
3. **Given** the migration has run once, **When** it runs again, **Then** it reports
   `unchanged` and writes nothing.
4. **Given** Feature 001 installed its resources at project scope, **When** Feature 002 — whose
   defaults differ per resource kind — connects, **Then** it adopts those resources where they
   already are, records their actual scope, and does not relocate them; moving one to a different
   scope is offered as an explicit migration and never performed silently.
5. **Given** a Claude Code session started after migration, **When** the agent reads files,
   edits files, runs a test that fails, finishes a turn, hits compaction, and ends, **Then**
   the observations, the quiescence checkpoint, the compaction handoff, and the session-end
   handoff are the same records Feature 001 produced for the same actions — the canonical rename
   changes no stored behavior.
6. **Given** the installed Claude Code version exposes lifecycle events Feature 001 did not
   use, **When** Cairn connects, **Then** it registers only the events its canonical lifecycle
   needs and reports the rest as unused rather than claiming them.

---

### User Story 3 - Codex participates natively (Priority: P1)

A developer who works in Codex connects it to Cairn. Codex sessions become Cairn sessions,
Codex receives project context, Codex's work is captured, and Codex produces handoffs — using
Codex's own officially supported configuration and lifecycle surfaces, not a Claude-shaped
guess about them.

**Why this priority**: Codex is the first proof that the adapter boundary is real. If the
canonical lifecycle only fits the agent it was extracted from, the abstraction is decorative.

**Independent Test**: In a scratch repository with Codex installed, run the Cairn connect for
Codex, complete whatever confirmation Codex itself requires to activate lifecycle handlers,
then run a short Codex session that edits a file and runs a failing command. Assert a Cairn
session exists with Codex recorded as its agent, observations exist with the correct canonical
categories, and a handoff exists at the boundary Codex actually signals.

**Acceptance Scenarios**:

1. **Given** Codex is installed, **When** the developer runs detection, **Then** Cairn reports
   Codex as detected with its version where obtainable and its capability profile.
2. **Given** Codex is detected, **When** the developer connects it, **Then** Cairn writes its
   MCP entry and lifecycle handler configuration into Codex's own supported configuration
   format, preserving the file's existing entries, comments, and formatting.
3. **Given** Codex requires the user to confirm or trust externally-supplied lifecycle
   handlers before they run, **When** connect completes, **Then** Cairn states plainly that
   the handlers are installed but not yet active, tells the developer exactly what to do in
   Codex to activate them, and reports the integration as incomplete until they are.
4. **Given** an active Codex session, **When** a tool call succeeds, **Then** Cairn records a
   success observation in the canonical category for that tool.
5. **Given** an active Codex session, **When** a tool call fails, **Then** Cairn records a
   failure observation only if the payload actually establishes failure; where the payload is
   ambiguous, Cairn records the call without asserting a failure it cannot prove.
6. **Given** Codex signals its session boundary under a strict time limit, **When** that
   boundary arrives, **Then** Cairn's session-end work completes inside Codex's own budget,
   never exceeds it, and anything that does not fit is recovered at the next deterministic
   boundary rather than lost silently.
7. **Given** measurements show Cairn's session-end work fits reliably inside Codex's handler
   budget and its failure and recovery cases pass, **When** the integration level is computed,
   **Then** Codex is reported FULL; **and** if that reliability is not demonstrated, Codex is
   reported below FULL with automatic session completion named as the missing behavior.
8. **Given** a Codex session and a Claude Code session, **When** each records memory, **Then**
   both memories carry the correct originating agent and session as provenance and are scoped
   by the project's memory model, not by which agent wrote them.

---

### User Story 4 - OpenCode participates natively (Priority: P1)

A developer who works in OpenCode connects it to Cairn through OpenCode's own plugin and
configuration surfaces. Capture and context work; the parts of the lifecycle OpenCode does not
signal are reported as absent rather than faked.

**Why this priority**: OpenCode is the case that proves the honesty rule. Its event vocabulary
does not line up with Claude Code's, and the tempting shortcuts are wrong. Getting this right
is what makes the capability model trustworthy.

**Independent Test**: In a scratch repository with OpenCode installed, connect it, run a short
session, and assert that observations are captured, that going idle produces a quiescence
checkpoint and leaves the Cairn session active, and that the capability report does not claim a
session-completion signal OpenCode does not send.

**Acceptance Scenarios**:

1. **Given** OpenCode is installed, **When** the developer connects it, **Then** Cairn
   installs its MCP entry, its managed instruction block, its Skill where supported, and a
   Cairn-owned plugin registration, each idempotently and alongside any plugins already
   configured.
2. **Given** an OpenCode session doing work, **When** tools complete, **Then** Cairn records
   canonical observations for them.
3. **Given** an OpenCode session that goes idle, **When** the idle signal arrives, **Then**
   Cairn records a quiescence checkpoint, the Cairn session remains `active`, and no durable
   handoff is produced — idleness means the agent stopped working, not that a turn succeeded and
   not that the session ended.
4. **Given** an OpenCode session that goes idle immediately after an error, **When** the idle
   signal arrives, **Then** Cairn records the same quiescence checkpoint and no success or
   failure observation is synthesized from it; the error observation comes from the tool event
   that produced it.
5. **Given** an OpenCode session that compacts, **When** the compaction boundary is signalled,
   **Then** Cairn produces a durable handoff for that boundary and the session remains usable.
6. **Given** OpenCode provides no signal that a session has ended, **When** the capability
   report is produced, **Then** OpenCode is not reported as FULL, the report states that
   automatic session completion is unavailable, and OpenCode sessions leave `active` only
   through Cairn's existing deterministic boundaries — an explicit end, reconciliation at daemon
   start, or the inactivity timeout — which are recovery from silence, not completion.
7. **Given** only those safety nets, **When** the integration level is computed, **Then** they
   contribute nothing towards FULL, and the report says OpenCode sessions are closed by
   inactivity rather than completed.
8. **Given** a future OpenCode mechanism that establishes that a session actually terminated —
   not merely that it went quiet — **When** that mechanism is demonstrated by test, **Then**
   OpenCode may be reported FULL on that basis; the requirement is evidence of termination, not
   a vendor event of any particular name.
9. **Given** Cairn's plugin registration is already present, **When** connect runs again,
   **Then** it is updated in place, never appended a second time, and unrelated plugin
   registrations are untouched.

---

### User Story 5 - CC Switch distributes Cairn across agents (Priority: P2)

A developer who manages their coding agents through CC Switch wants Cairn's MCP server and
Cairn's Skill distributed to the applications CC Switch manages, without ending up with two
copies of each and without Cairn reaching into CC Switch's private storage.

**Why this priority**: CC Switch is how a growing number of developers actually configure
these tools, and ignoring it guarantees the exact duplicate-configuration mess this feature
exists to prevent. It comes after native adapters because it distributes resources those
adapters define.

**Independent Test**: With CC Switch installed, use Cairn to distribute the Cairn MCP server
and the Cairn Skill to a chosen set of CC Switch-managed applications, then run diagnostics and
assert each target application has exactly one Cairn MCP entry, that Cairn records CC Switch as
the owner of those resources, and that no unrelated CC Switch configuration changed.

**Acceptance Scenarios**:

1. **Given** CC Switch is installed, **When** the developer runs detection, **Then** Cairn
   reports CC Switch as an integration manager — explicitly not as an agent — with its version
   where obtainable and the applications it can distribute to.
2. **Given** CC Switch is present and preferred, **When** the developer distributes Cairn's
   MCP server through it, **Then** Cairn uses CC Switch's officially supported import surface,
   the developer confirms the import inside CC Switch, and Cairn never writes to CC Switch's
   private storage.
3. **Given** the distribution has been performed, **When** diagnostics run, **Then** Cairn
   verifies the resulting bindings in each selected application and reports exactly one Cairn
   MCP entry per application.
4. **Given** Cairn's MCP server is already installed directly by Cairn for an agent, **When**
   the same server would also be distributed through CC Switch to that agent, **Then** Cairn
   reports a conflicting-ownership condition and does not silently create the second copy.
5. **Given** a developer switches provider or configuration inside CC Switch, **When**
   diagnostics run afterwards, **Then** the Cairn integration is still reported healthy and no
   Cairn-owned resource was removed or duplicated by the switch.
6. **Given** resources distributed through CC Switch, **When** the developer removes Cairn's
   CC Switch management and CC Switch documents no automated removal interface, **Then** Cairn
   reports manager action required — naming the resource, the target applications, and the
   supported CC Switch removal path — writes nothing to CC Switch's own storage, and does not
   report the resource as removed.
7. **Given** the developer has performed that removal inside CC Switch, **When** they re-run
   diagnostics, **Then** Cairn verifies the target applications' real configuration, updates its
   ownership record from what it finds, and reports the resource gone only if it actually is.
8. **Given** any of the above, **When** it completes, **Then** every other CC Switch-managed
   provider, MCP server, prompt, and Skill is untouched, and native lifecycle adapters installed
   directly by Cairn are unaffected.

---

### User Story 6 - Work continues in a different agent (Priority: P1)

A developer starts work in one agent and continues it in another, on the same repository. The
second agent receives the decisions, failures, procedures, and handoff the first one produced,
with no export, no import, and no manual re-explanation.

**Why this priority**: This is the product. Everything else in Feature 002 is machinery for it.

**Independent Test**: Run the cross-agent continuity scenario in the acceptance section below,
end to end, on one repository, and assert the second and third agents retrieve the first
agent's durable knowledge and latest handoff, with provenance naming the agent and session that
produced each item.

**Acceptance Scenarios**:

1. **Given** an agent recorded a durable decision, a reusable failure, and a handoff on a
   repository, **When** a different supported agent opens the same repository, **Then** Cairn
   resolves the same project — and the same task where one is bound — and delivers that
   knowledge in the second agent's briefing.
2. **Given** the second agent records a durable procedure, **When** a third supported agent
   opens the same repository, **Then** it receives the decision, the failure, and the procedure,
   and the most recent handoff.
3. **Given** knowledge produced by several agents, **When** any of it is retrieved, **Then**
   each item names the agent and session that produced it, as provenance.
4. **Given** knowledge produced by several agents, **When** memory is scoped or searched,
   **Then** scoping and ranking use only project, branch, task, and session; the producing
   agent never narrows, widens, or partitions what a later agent can retrieve.
5. **Given** three different agents used on one repository, **When** projects are listed,
   **Then** there is exactly one project, not one per agent.

---

### User Story 7 - Cairn diagnoses and repairs its own integration (Priority: P1)

Configuration drifts. An agent updates, a developer edits a file, a manager rewrites a config,
a Cairn artifact goes stale. Cairn can tell the developer exactly what is wrong and fix the
parts it owns, without touching anything else.

**Why this priority**: An integration layer that spans four agents and a configuration manager
will be broken sometimes. Without honest diagnosis, every failure looks like "Cairn does not
work" and the safe fix is uninstalling it.

**Independent Test**: Connect every supported agent, then deliberately break one thing per
agent — delete a hook entry, corrupt the managed instruction markers, downgrade the Skill
version, add a duplicate MCP entry, point a resource at a different owner. Run diagnostics and
assert each break is identified by exact resource and state. Run repair and assert each
Cairn-owned break is fixed, nothing else changed, and a second repair reports nothing to do.

**Acceptance Scenarios**:

1. **Given** connected agents, **When** diagnostics run, **Then** Cairn reports core health
   (component version alignment, daemon reachability, project registration) and, per agent,
   detection, version and compatibility classification, capability level, and the state of each
   Cairn-owned resource.
2. **Given** a Cairn-owned resource is missing, modified, outdated, duplicated, or owned by a
   different party than Cairn recorded, **When** diagnostics run, **Then** each condition is
   reported as a distinct state naming the exact resource, not as a generic failure.
3. **Given** an agent's configuration file is malformed, **When** diagnostics run, **Then**
   Cairn reports it as malformed and refuses to rewrite it, rather than replacing it with a
   valid file of Cairn's own construction.
4. **Given** reported problems that Cairn owns, **When** repair runs, **Then** Cairn restores
   missing resources, upgrades outdated ones, removes duplicate Cairn-owned entries, and leaves
   every non-Cairn setting unchanged.
5. **Given** repair has run, **When** it runs again, **Then** it reports nothing to do and
   writes nothing.
6. **Given** a problem whose safe resolution requires a human decision, **When** repair runs,
   **Then** it explains the conflict and the options and changes nothing, rather than guessing.
7. **Given** an agent version Cairn has never verified, **When** diagnostics run, **Then**
   Cairn classifies it as compatible-but-unverified and continues, and only reports unsupported
   for versions it knows are incompatible.
8. **Given** any diagnostic output, **When** it is produced in either human or machine-readable
   form, **Then** it contains no credential, token, or key value.

---

### User Story 8 - Any MCP-compatible agent gets useful memory (Priority: P2)

A developer uses an agent Cairn has no native adapter for. They can still point it at Cairn's
MCP server and get context, search, memory, tasks, and handoffs — and Cairn tells them plainly
which automatic behavior they are not getting.

**Why this priority**: Cairn cannot chase every agent, and it should not have to. This keeps the
product useful beyond its adapter list while making the reduced capability honest.

**Independent Test**: Drive Cairn's MCP server with a plain MCP client. Assert initialization
succeeds, the usage contract arrives where the protocol carries it, exactly the six tools are
offered, each tool works, and the reported integration level states that automatic lifecycle and
capture are unavailable.

**Acceptance Scenarios**:

1. **Given** any MCP-compatible client, **When** it initializes against Cairn, **Then**
   initialization succeeds and exactly the six Feature 001 tools are advertised.
2. **Given** that client, **When** it calls the context, search, memory, session, task, and
   handoff tools, **Then** each behaves as specified in Feature 001.
3. **Given** that client, **When** the integration level is reported, **Then** it is stated as
   MCP-only, with automatic lifecycle and automatic capture explicitly listed as unavailable,
   and it is never reported as a full integration.
4. **Given** a developer using an unsupported MCP client, **When** they ask Cairn for a
   configuration export, **Then** Cairn emits a deterministic, secret-free MCP server
   configuration they can paste into that client.

---

### User Story 9 - Removal and ownership changes are safe (Priority: P2)

A developer disconnects an agent, or moves a resource from Cairn's direct management to CC
Switch's. Their configuration survives, and their memory is never at risk.

**Why this priority**: People try tools they can undo. Disconnect and ownership migration are
what make connecting a low-risk decision, and getting them wrong destroys trust permanently.

**Independent Test**: With several agents connected and unrelated configuration present in each
file Cairn touched, disconnect one agent. Assert its Cairn resources are gone, every other
agent still works, every unrelated setting is intact, and every project, task, session, memory,
and handoff still exists. Then migrate one resource between owners and assert no window exists
in which the resource is absent or doubled.

**Acceptance Scenarios**:

1. **Given** a connected agent, **When** the developer disconnects it, **Then** Cairn removes
   its lifecycle integration, its Cairn-owned instruction block, its Cairn-owned Skill, its
   Cairn-owned MCP entry where Cairn owns it directly, and its local integration record.
2. **Given** the same disconnect, **When** it completes, **Then** no project, task, session,
   observation, memory, or handoff is deleted, and no unrelated MCP server, hook, plugin,
   instruction, or credential is modified.
3. **Given** a resource whose recorded owner is an integration manager, **When** the developer
   disconnects the agent, **Then** Cairn does not delete that resource on the manager's behalf
   and does not touch the manager's own storage; it reports manager action required with the
   supported withdrawal path, and verifies the result only after the developer has acted.
4. **Given** other agents are connected, **When** one is disconnected, **Then** the others
   continue working with no change to their configuration.
5. **Given** a Cairn resource owned directly, **When** the developer migrates it to
   manager-owned, **Then** Cairn records the resource as migrating, installs the target through
   the manager's supported interface, verifies the result in the agent's real configuration, and
   only then removes the source — leaving exactly one owner; and if verification fails at any
   step, the previously working resource is intact and the failure is reported.
6. **Given** a migration whose two owners write physically distinct locations, **When** the
   overlap window exists, **Then** exactly one resource is effective throughout according to the
   agent's own precedence rules, and diagnostics report the state as migrating rather than as
   duplicated or conflicting ownership; where overlap would make the effective configuration
   ambiguous, Cairn refuses to migrate automatically and states the manual sequence.
7. **Given** a migration interrupted part-way, **When** diagnostics run, **Then** the migrating
   state is reported with its source and target, and the developer can either resume or reverse
   it without first disconnecting the agent.
8. **Given** an instruction file with only Cairn's block in it, **When** Cairn's block is
   removed, **Then** any content the developer added remains and the file is not deleted merely
   because Cairn's part is gone.

---

### User Story 10 - Several agents at once (Priority: P1)

A developer runs two agents at the same time on the same repository — the same checkout, or two
worktrees. Each gets its own Cairn session. Neither steals, ends, or corrupts the other's.

**Why this priority**: This is normal now, and Feature 001 already showed that ambiguous session
resolution is the most dangerous failure a memory system can have. Adding agents multiplies the
opportunity for it.

**Independent Test**: Run two supported agents concurrently in one worktree and, separately, in
two worktrees of one repository. Assert two distinct sessions, correct per-session provenance on
every observation and memory, one project, and that ambiguous requests fail with an actionable
error instead of resolving to an arbitrary session.

**Acceptance Scenarios**:

1. **Given** two agents working in one checkout, **When** both sessions start, **Then** Cairn
   holds two distinct active sessions keyed to each agent's own session identifier, bound to one
   project.
2. **Given** those two sessions, **When** lifecycle events arrive from either, **Then** each is
   routed to the session that produced it, and neither session's status is changed by the
   other's events.
3. **Given** those two sessions, **When** either records memory or an observation, **Then**
   provenance names the correct agent and session.
4. **Given** a request that cannot be attributed to one session, **When** it is made, **Then**
   Cairn reports an ambiguous-session error naming the candidates, and neither guesses nor
   invents a session.
5. **Given** two worktrees of one repository with a different agent in each, **When** both work,
   **Then** there is one project, two sessions, and repository state is recorded per worktree.
6. **Given** an agent whose adapter cannot supply a stable session identifier, **When** a
   session starts, **Then** Cairn reports that limitation in the capability profile rather than
   sharing one session between agents.

---

### Edge Cases

- **An agent is installed but its configuration directory does not exist yet**: Cairn creates
  only what it owns, at the officially documented location, and reports what it created.
- **An agent's configuration file is malformed**: connect, repair, and disconnect fail loudly
  with the file named and the parse problem stated. Nothing is rewritten, and no partially
  written file is left behind.
- **An agent's configuration file is read-only, or the filesystem is full**: the operation fails
  with the reason and the original file is left exactly as it was.
- **Two agents share one configuration surface**: one agent reads instruction or Skill files that
  another agent also reads. Cairn installs one owned copy per logical resource, detects when its
  own resource is being read by a second agent, and reports that as shared rather than
  duplicated.
- **A vendor renames or removes a lifecycle event Cairn used**: the missing event is reported as
  an unmet capability, the capability level drops accordingly, and the integration keeps working
  at the reduced level rather than failing.
- **A vendor adds a lifecycle event Cairn does not use**: it is ignored. Cairn registers only the
  events its canonical lifecycle needs.
- **A vendor payload carries conversation text** (an assistant message, a user prompt, a raw tool
  transcript): it is never persisted. New payload fields do not create a route around the Feature
  001 capture policy.
- **A lifecycle handler is installed but the agent requires the user to trust or enable it**:
  Cairn reports the integration as installed-but-inactive with the exact action required, and
  does not report it as connected.
- **A Cairn lifecycle handler is upgraded and the agent's trust of it is invalidated**: this is
  reported as a distinct state with the re-activation step, not as a missing integration.
- **A managed instruction block's markers have been damaged or removed by hand**: Cairn does not
  guess which text was its own. It reports the block as unrecoverable and asks before writing.
- **Two Cairn-owned copies of one resource exist at different scopes** (project and global):
  reported as duplicated, with the effective one identified according to the agent's own
  precedence rules.
- **A resource exists but is owned by a different party than Cairn's record says**: reported as
  conflicting ownership, never silently adopted or deleted.
- **The configuration manager is uninstalled while it still owns Cairn resources**: reported as
  orphaned resources, with the option to take direct ownership.
- **The configuration manager offers no documented way to remove what it distributed**: Cairn
  reports manager action required with the supported path, writes nothing to the manager's own
  state, and verifies the outcome after the developer acts.
- **A migration is interrupted with both resources present**: reported as migrating with its source
  and target, not as accidental duplication, and resumable or reversible without disconnecting.
- **A session's agent goes quiet after an error rather than after answering**: a quiescence
  checkpoint is recorded; no outcome is inferred from the quiescence itself.
- **A session is closed by the inactivity timeout**: it is reported as closed by inactivity, the
  handoff is marked as recovered, and this contributes nothing to the agent's capability level.
- **A dry run is executed against a broken configuration**: it reports the conflict and still
  writes nothing.
- **A connect touches several files and one write fails**: the command fails, reports exactly
  which parts landed and which did not, and diagnostics report the same partial state. Cairn
  never reports success for a half-installed integration.
- **The daemon is unreachable during connect**: connect fails clearly; it does not write agent
  configuration it cannot verify.
- **The daemon is unreachable during a lifecycle event**: the event is dropped, the agent
  continues, and nothing is written to the agent's error surface.
- **An agent version is newer than anything Cairn has verified**: classified as
  compatible-but-unverified and used.
- **The same repository is opened by different agents at different absolute paths** (worktrees,
  clones): project identity follows Feature 001's rules; no path becomes an identity.

## Clarifications

### Session 2026-08-11

Four decisions were resolved during specification. Each was open enough to change the shape of
the feature, and each is recorded here with the reasoning, so it can be revisited deliberately
rather than discovered later in planning.

- **Q: Does the FULL integration level require automatic session completion at a boundary the
  agent itself signals?** → **A: No — FULL requires a reliable lifecycle-completion guarantee, and
  a vendor session-end signal is the preferred mechanism for it, not the requirement itself.**
  FULL means Cairn's MCP server, the usage contract as persistent instructions, the Cairn Skill
  where the agent supports Skills, a stable session identity, automatic context at session start,
  automatic tool capture, quiescence checkpoints, **and** a mechanism that positively establishes
  that a session terminated and finalizes it without user intervention, inside a defined bounded
  contract. An adapter may satisfy the last clause through a different deterministic, testable
  mechanism only where that mechanism establishes termination with equivalent fidelity. Cairn's
  generic safety nets — the inactivity timeout that closes a session nothing has driven for hours,
  and daemon-start reconciliation — are recovery from silence, not evidence that a session ended;
  they remain in place, they may backstop a missed or failed completion boundary, and they never
  count towards FULL. Against current verified capability: Claude Code is FULL; Codex is eligible
  for FULL if Cairn's session-end handoff reliably fits inside Codex's 1-second default and
  3-second maximum handler budget and its injected-failure recovery tests pass; OpenCode is not
  FULL, because `session.idle` must never be treated as termination, and a timeout is not a
  substitute for one.
  (FR-109, FR-110, FR-207, FR-208, FR-209, FR-229, SC-127, SC-128, SC-129, SC-131)

- **Q: At which scope does Cairn install the resources it owns, by default?** → **A: Per resource
  kind, not one universal default — separating portable project intent from machine-local
  activation.** Managed instructions are project-scoped and commit-safe, because they describe how
  this repository uses Cairn. Lifecycle handlers and plugins are project-scoped but local and
  uncommitted by default, because they execute a local binary and must not be silently imposed on
  every collaborator. The MCP registration prefers per-user installation where the agent can
  safely run one Cairn MCP server across repositories, with project-local registration where the
  agent requires it or the developer asks; the active project is resolved from the working
  directory either way. The Cairn Skill prefers a single per-user installation where supported,
  because it teaches generic Cairn workflows and does not belong in every repository. Resources
  distributed by an integration manager follow that manager's own per-user model, and Cairn must
  not create a competing direct copy of the same resource. Teams that want committed lifecycle and
  MCP configuration get it through an explicit shared option. Cloning a repository never installs
  or activates anything: explicit connection remains the activation boundary.
  (FR-210 – FR-220, SC-126)

- **Q: What does repair do when a Cairn-managed resource has been edited by hand?** → **A:
  Non-destructive conflict reporting by default; forced restoration is confined to Cairn's exact
  ownership boundary.** Repair classifies the resource as modified, shows what differs, and changes
  nothing. A forced repair may restore Cairn's canonical version, but only the managed block, only
  the exact owned entry, or only the fully generated file — never anything outside Cairn's
  markers — and only after preserving the previous managed content somewhere recoverable. Content
  that differs only in formatting or ordering is semantically equivalent and is reported healthy,
  not modified. No recovery path may require disconnecting and reconnecting. Adopting a developer's
  customization as Cairn-managed content is a separate, explicit future operation; repair itself
  never adopts an edit silently. (FR-177, FR-221 – FR-225, SC-130)

- **Q: Does Feature 002 include a committed project-local desired-state manifest?** → **A: The
  committed file is deferred; the model behind it is not.** Feature 002 has one canonical internal
  representation of intended integrations — agent selection, resource ownership, and capability
  mode — and guided onboarding, preview, diagnostics, repair, and manager migration all operate on
  that one model. In this feature the desired state is composed from explicit developer choices,
  Cairn's local integration record, and the configuration actually detected on the machine; no
  project-local manifest file is written or treated as authoritative. The model is versioned,
  deterministic, secret-free, and serializable so that a later feature can expose it as a committed
  manifest without changing adapter or reconciliation semantics. Manifest-versus-reality drift
  handling, manifest merge semantics, and automatic application on clone are out of scope here.
  (FR-201, FR-202, FR-226, FR-227, SC-135, Out of Scope)

## Requirements *(mandatory)*

Feature 002 requirements are numbered from **FR-101** so that references to Feature 001
requirements (FR-001–FR-064) stay unambiguous. Feature 001 requirements remain in force; where
Feature 002 extends one, it says so explicitly. Requirement numbers are stable identifiers, not
an ordering.

### Adapters, managers, and detection

- **FR-101**: Cairn MUST model connected coding agents and configuration managers as two
  distinct kinds of integration. An agent adapter owns the translation between one agent's
  configuration and lifecycle and Cairn's canonical model. An integration manager distributes
  Cairn resources to agents but is never treated as an agent, never produces sessions, and never
  produces observations.
- **FR-102**: Cairn MUST provide native agent adapters for Claude Code, Codex, and OpenCode, and
  a generic adapter for MCP-compatible agents with no native support.
- **FR-103**: Cairn MUST provide exactly one integration manager in this feature: CC Switch.
- **FR-104**: Cairn MUST detect, on the local machine, which supported agents and which
  supported integration managers are installed, and MUST report each one's version where the
  agent or manager exposes one obtainable without authentication.
- **FR-105**: Detection MUST NOT modify any agent or manager configuration, and MUST NOT require
  network access.
- **FR-106**: Cairn MUST NOT add a native lifecycle adapter for any agent beyond those in
  FR-102, including applications a detected integration manager happens to support.

### Capability model and integration level

- **FR-107**: Cairn MUST maintain, per agent, an explicit capability profile describing which
  integration surfaces that agent actually supports. The profile MUST cover at least: MCP
  configuration and the scopes at which it can be written; persistent instructions and their
  scopes; Skills and their scopes; a session-open signal; a tool-success signal; a
  distinguishable tool-failure signal; a quiescence boundary; a pre-compaction boundary; a
  post-compaction boundary; a session-close signal; delivery of context at or near session open;
  availability of a stable agent-supplied session identifier; and whether lifecycle handlers
  require a user trust or activation step inside the agent before they run.
- **FR-108**: A capability MUST be recorded as present only when the agent's own documented
  behavior provides it. Cairn MUST NOT infer a capability from the presence of a similarly named
  one.
- **FR-109**: Cairn MUST derive an integration level from the capability profile and the
  verified state of installed resources, using these levels:
  **FULL** — Cairn's MCP server, the Cairn usage contract as persistent instructions, the Cairn
  Skill where the agent supports Skills, a stable session identity, context delivered
  automatically at session start, automatic tool capture, quiescence checkpoints, and a
  **reliable lifecycle-completion guarantee** as defined in FR-207;
  **MCP_PLUS** — Cairn's MCP server plus at least one of persistent instructions, the Skill, or
  partial lifecycle capture, but not everything FULL requires;
  **MCP_ONLY** — Cairn's MCP tools are reachable and nothing else is claimed;
  **UNSUPPORTED** — the agent is detected but cannot be integrated safely.
- **FR-110**: Every surface that reports an integration MUST report its level and MUST NOT
  describe an integration as full, complete, or fully connected when its level is below FULL.
- **FR-111**: When a capability is absent, Cairn MUST name the specific behavior the developer
  will not get, rather than reporting a numeric or unlabeled score.
- **FR-207**: A **reliable lifecycle-completion guarantee** means that, without user intervention
  and within a bounded documented contract, Cairn positively establishes that a session has
  terminated and finalizes it — producing that session's durable handoff and leaving it in a
  terminal state. A signal from the agent whose documented meaning is that the session has ended
  is the preferred mechanism. An adapter MAY satisfy this requirement through a different
  deterministic mechanism only where that mechanism **establishes termination with equivalent
  fidelity** — it observes that the session actually ended — and is demonstrated by test rather
  than asserted. An adapter with no such mechanism MUST NOT be reported as FULL, and its report
  MUST name automatic session completion as the missing behavior.
- **FR-229**: Cairn's generic safety nets — the inactivity timeout that closes a session nothing
  has driven for hours, and daemon-start reconciliation of sessions left `active` by a previous
  run (FR-009) — are **recovery from silence, not evidence of termination**. They MUST continue to
  exist, they MAY backstop a normal completion boundary that was missed or that failed, and they
  MUST NOT, alone or in combination, satisfy FR-207 or contribute to a FULL classification for any
  agent. An agent whose only route to a terminal session state is a timeout is below FULL, and its
  report MUST say that sessions are closed by inactivity rather than completed.
- **FR-208**: Where an agent's session-boundary handler runs under a deadline shorter than the
  work Cairn would do at that boundary, the adapter MUST be shown by measurement to complete its
  session-end work reliably inside the agent's own budget, or Cairn MUST NOT claim the completion
  guarantee for that agent. Adapters MUST NOT exceed an agent's handler deadline to obtain it.
- **FR-209**: An agent whose lifecycle handlers are installed but not yet activated by the user
  MUST NOT be reported at the level those handlers would provide once active. Until they are
  active, the level reflects what actually works, and the required action is stated.

### Canonical lifecycle

- **FR-112**: Cairn MUST define one canonical lifecycle vocabulary that all adapters translate
  into, and Cairn's session, capture, and handoff behavior MUST depend only on that vocabulary.
  No vendor event name, payload shape, or ordering assumption may reach Cairn's core semantics.
- **FR-113**: The canonical vocabulary MUST be the minimum set that expresses Feature 001's
  established behavior across the supported agents: session opened; tool succeeded; tool failed;
  **agent quiesced**; context compacting; context compacted; session closed. Feature 002 MUST NOT
  add canonical events that no supported agent signals.
- **FR-114**: Canonical events MUST preserve Feature 001's meanings exactly: session opened
  starts or resumes a session and is the context-delivery point; tool succeeded and tool failed
  produce the corresponding typed observations; **agent quiesced** means the agent has stopped
  working and is waiting — it is a checkpoint that leaves the session `active` and produces no
  durable handoff; context compacting produces a durable handoff and leaves the session usable;
  session closed completes the session and produces its final handoff.
- **FR-115**: An adapter MUST NOT emit a canonical event the agent has not actually signalled.
  Where an agent provides no signal for a canonical event, that event simply does not occur for
  that agent, the gap is reflected in the capability profile, and Cairn's existing deterministic
  session boundaries (FR-009) govern the outcome.
- **FR-116**: An adapter MUST NOT map an agent's idle, quiet, or inactive signal to session
  closed. Emitting session closed requires evidence that the session actually terminated: a signal
  whose documented meaning is that it ended, or another mechanism meeting FR-207's
  equivalent-fidelity bar. Absence of activity is never such evidence (FR-229).
- **FR-117**: An adapter MUST NOT report a tool failure that the agent's payload does not
  establish. Where an agent reports success and failure on one event, the adapter MUST classify
  from the payload's own outcome data; where the payload is ambiguous, the observation MUST record
  the call without asserting failure.
- **FR-230**: **Agent quiesced** is deliberately weaker than "the agent finished answering". It
  asserts only that the agent has stopped working and is waiting, which is the strongest claim all
  supported agents actually establish: one signals that it finished responding, another signals
  that its turn stopped, and a third signals only that the session became idle — and that third
  signal can follow an error as readily as a completed answer. Cairn MUST NOT infer from this event
  that the preceding work succeeded, that a turn produced an answer, or that the session is
  finished. It preserves Feature 001's `Stop` behavior exactly: flush pending capture, record the
  checkpoint, leave the session `active`, write no handoff (FR-032).
- **FR-231**: Where an agent's quiescence signal can follow a failure, the adapter MUST NOT
  synthesize a success or failure observation from the quiescence event itself. Outcome
  observations come only from tool events (FR-117).
- **FR-118**: Every canonical event MUST carry the identity of the agent session that produced
  it, so that concurrent sessions are routed correctly (FR-010).
- **FR-119**: Post-compaction is an extension to Feature 001's boundary set. It MUST leave the
  session `active`, MUST NOT produce a second durable handoff for the same compaction, and MAY
  re-deliver context where the agent accepts it.

### Canonical tool categories

- **FR-120**: Cairn MUST normalize agent-specific tool names into canonical categories before
  storage, and MUST reuse Feature 001's observation types rather than introducing a parallel
  taxonomy.
- **FR-121**: Memory, handoff, and context behavior MUST depend only on canonical categories,
  never on a vendor tool name.
- **FR-122**: Cairn MAY retain the raw vendor tool name as bounded provenance on an observation,
  subject to the same redaction and size bounds as every other captured field. It MUST NOT retain
  raw tool payloads.

### Agent usage contract

- **FR-123**: Cairn MUST define one canonical, versioned Cairn Agent Usage Contract that teaches
  an agent how to use Cairn. All agent-facing renderings of the contract MUST be generated from
  that single source.
- **FR-124**: The contract MUST state at least: consume the Cairn context before re-deriving the
  project; search existing memory before repeating a prior investigation; record durable facts,
  decisions, conventions, failures, and procedures; do not record routine tool invocations; use
  the narrowest correct memory scope; never invent evidence observation identifiers; never place
  secrets, credentials, raw prompts, or unbounded output into memory; leave automatic session
  boundaries, checkpoints, and handoffs to the lifecycle integration; and bind work to a Cairn
  task when one applies.
- **FR-125**: The always-on rendering of the contract MUST be bounded to a documented maximum
  size, and that bound MUST be asserted automatically rather than assumed.
- **FR-126**: The contract MUST carry a version identifier, and every installed rendering MUST
  record the version it was generated from so that outdated installations are detectable.
- **FR-127**: Cairn MUST NOT place its full documentation, workflow guidance, or tool reference
  into the always-on rendering.

### MCP surface

- **FR-128**: Cairn's MCP tool surface MUST remain exactly the six Feature 001 tools
  (FR-040). Feature 002 MUST NOT add MCP tools; diagnostics, repair, connection, and export are
  developer operations, not agent tools.
- **FR-129**: Cairn's MCP server MUST provide the compact universal rendering of the usage
  contract through the protocol's own server-instructions mechanism at initialization, so that a
  client with no native adapter still receives correct behavior. Where a client does not surface
  server instructions, Cairn MUST NOT treat the contract as delivered.
- **FR-130**: Cairn MUST negotiate MCP protocol versions honestly: it MUST answer with a version
  it actually implements, and MUST NOT echo a version it does not support.
- **FR-131**: Cairn MUST be able to emit a deterministic, secret-free MCP server configuration
  for a supported agent or for a generic MCP client, without modifying any configuration file.

### Managed instructions

- **FR-132**: For each agent that supports persistent instructions, Cairn MUST install the
  contract into that agent's officially supported instruction surface.
- **FR-133**: Cairn MUST write only inside its own explicitly marked block. It MUST NOT replace,
  reorder, reformat, or remove any content outside that block, and MUST NOT replace an entire
  instruction file.
- **FR-134**: The Cairn block MUST be delimited by explicit ownership markers carrying the
  contract version, so that the block can be located, verified, updated, and removed exactly.
- **FR-135**: Installation MUST be idempotent: reconnecting MUST NOT create a second block, and
  MUST report `unchanged` when the installed block already matches the current contract.
- **FR-136**: When an installed block's version is older than the current contract, repair MUST
  be able to replace that block in place, preserving everything around it.
- **FR-137**: When an instruction file is malformed, or Cairn's markers are damaged, missing, or
  unbalanced, Cairn MUST fail safely: it reports the condition and changes nothing.
- **FR-138**: Disconnect MUST remove the Cairn block and nothing else, and MUST leave the file in
  place if any other content remains.
- **FR-139**: Cairn MUST NOT match its own content by searching for the word "cairn". Ownership is
  established by the markers and the local integration record, never by fuzzy text matching.

### Cairn Skill

- **FR-140**: Cairn MUST provide an official Cairn Skill for agents that support Skills, covering
  the deeper workflows: resuming existing work, inspecting a prior handoff, searching before
  investigating, recording a durable decision, recording a reusable failure, recording a project
  convention, choosing the correct memory scope, handling session ambiguity, binding work to a
  task, and inspecting integration problems.
- **FR-141**: The Skill MUST be generated from one canonical source into whatever forms the
  supported agents accept, and MUST carry its own version identifier independent of the contract
  version, so that diagnostics can detect an outdated Skill.
- **FR-142**: The Skill's entry document MUST stay concise, with depth reached through
  progressive references rather than one large document.
- **FR-143**: Skill installation MUST be idempotent, MUST NOT overwrite a Skill of the same name
  that Cairn does not own, and MUST be removable leaving no Cairn Skill content behind.
- **FR-144**: Where one installed Skill location is read by more than one supported agent, Cairn
  MUST install one owned copy and report the sharing, rather than installing a second copy per
  agent.

### Configuration ownership

- **FR-145**: Cairn MUST record, per agent and per resource kind — MCP entry, lifecycle
  integration, managed instructions, Skill — exactly one intended owner: Cairn directly, an
  integration manager, or external (installed by the user or another tool and not managed by
  Cairn).
- **FR-146**: In **steady state**, Cairn MUST NOT hold the same logical resource under two owners.
  An operation that would leave two owners standing MUST fail with a conflicting-ownership error
  naming both. This requirement governs stable state and accidental duplication; it does not
  govern the controlled transition defined in FR-148, which is the only sanctioned way two
  physical resources for one logical resource may coexist.
- **FR-147**: Mixed ownership across resource kinds for one agent MUST be valid — for example
  lifecycle and instructions owned directly while the MCP entry and Skill are manager-owned.
- **FR-148**: Cairn MUST support migrating a resource between owners without ever leaving the
  developer with **no effective resource**. The migration MUST be an explicit, recorded state:
  - Where both owners write the same effective slot and the format permits it, the change MUST be
    a single atomic replacement, and no overlap occurs.
  - Where the owners write physically distinct locations, bounded temporary overlap is permitted
    **only** for the duration of the migration, and only where the agent's own precedence rules
    make the effective configuration during that window unambiguous. Where overlap would make the
    effective configuration ambiguous or unsafe, Cairn MUST NOT proceed automatically; it reports
    the conflict and the manual sequence instead.
  - The target MUST be verified before the source is removed.
  - On success, exactly one owner and one resource remain, and the record names it.
  - On failure at any step, the previously working configuration MUST be preserved or restored,
    and the migration MUST report failure.
- **FR-228**: While a migration is in progress, Cairn MUST record the resource as migrating,
  naming the source owner and the target owner. Diagnostics MUST report that state distinctly from
  `duplicated` and from `conflicting owner`, and a migration that was interrupted MUST be
  resumable or reversible rather than left indistinguishable from accidental duplication.
- **FR-149**: Cairn MUST only remove resources whose recorded owner is Cairn directly. A
  manager-owned resource MUST be withdrawn through an interface that manager documents for that
  purpose; where no such interface exists, Cairn MUST NOT remove it by any other means and MUST
  report what it did not remove, why, and the supported path for the developer to do it
  (FR-229).
- **FR-150**: Cairn MUST detect a Cairn resource that exists under an owner other than the one
  recorded, and MUST report it rather than adopting or deleting it.
- **FR-232**: Cairn MUST interact with an integration manager only through interfaces that manager
  documents for third-party use. Writing to a manager's private storage, database, or internal
  state is prohibited in every operation — connect, migrate, repair, and disconnect alike — and no
  requirement in this feature may be satisfied by doing so.
- **FR-233**: Where a manager documents an import or distribution interface but no automated
  removal interface, Cairn MUST NOT claim to have removed a manager-owned resource. It MUST report
  a distinct **manager action required** outcome that names the resource, the target applications,
  and the supported path for the developer to withdraw it inside that manager, and it MUST leave
  Cairn's record showing the resource as still manager-owned until verification says otherwise.
- **FR-234**: After a developer completes a manager-side action, Cairn MUST be able to verify the
  real outcome by inspecting the target applications' own configuration, and MUST update its
  ownership record from what it finds rather than from what was requested.
- **FR-235**: Where a manager does expose a documented automated removal interface, the adapter
  MAY use it, and the same verification in FR-234 still applies before the record is updated.

### Installation scope

- **FR-210**: Cairn MUST choose an installation scope per resource kind rather than applying one
  universal default, separating portable project intent from machine-local activation.
- **FR-211**: Managed instructions MUST default to project scope in a commit-safe location where
  the agent supports one, because they describe how this repository uses Cairn.
- **FR-212**: Lifecycle handlers and plugin registrations MUST default to project scope but to a
  developer-local, uncommitted location where the agent provides one. They execute a local binary,
  and Cairn MUST NOT impose them on collaborators by default.
- **FR-213**: The Cairn MCP registration MUST default to per-user scope where the agent can safely
  run one Cairn MCP server across repositories, and MUST use project scope where the agent requires
  it or the developer asks for it. Under either scope the active project MUST be resolved from the
  working directory, never from where the registration lives.
- **FR-214**: The Cairn Skill MUST default to a single per-user installation where the agent
  supports one, because it teaches generic Cairn workflows that do not belong in every repository.
- **FR-215**: Cairn MUST offer an explicit shared option that installs lifecycle and MCP
  configuration into committed project scope for teams that want it, and the resulting scope MUST
  be recorded alongside ownership.
- **FR-216**: Cloning a repository MUST NOT install or activate any integration. Explicit
  connection is the only activation boundary, and no project-committed content may cause Cairn to
  configure an agent on a machine by itself.
- **FR-217**: Migration from a Feature 001 installation MUST NOT silently relocate a resource
  across scopes. Existing project-scoped resources MUST be recognized, adopted in place, and
  recorded with their actual scope; moving one to a different scope MUST be an explicit,
  previewable migration (FR-148).
- **FR-218**: Where an agent provides no developer-local location for a resource whose default is
  developer-local, Cairn MUST state that the only available location is committed, and MUST obtain
  the developer's agreement before writing it. It MUST NOT fall back to a committed location
  silently.
- **FR-219**: Resources distributed by an integration manager follow that manager's own scope
  model. Where the manager installs a resource at per-user scope, Cairn MUST NOT also install its
  own copy of that resource at per-user scope for the same agent; the conflict is reported under
  FR-146 rather than resolved by choosing a different scope.
- **FR-220**: Every installed resource MUST have its scope recorded in the local integration
  record (FR-182), and diagnostics MUST report the scope alongside the owner, so that a resource
  present at two scopes is distinguishable from a resource present twice at one scope.

### Safe configuration mutation

- **FR-151**: Every operation that changes agent or manager configuration MUST follow the same
  sequence: inspect current state, compute the intended change, validate it, apply it, and verify
  the result.
- **FR-152**: Cairn MUST preserve every setting it does not own in every file it writes,
  including entries, ordering where the format is order-significant, comments, and formatting
  where the format carries them.
- **FR-153**: Cairn MUST edit structured configuration through a structure-preserving mechanism
  for that format. Hand-built string substitution into a structured configuration file is
  prohibited.
- **FR-154**: Each individual configuration file MUST be replaced atomically, so that an
  interrupted write can never leave a truncated or partially written file.
- **FR-155**: Where an operation spans several files, Cairn MUST verify the outcome and, on
  partial failure, MUST report failure naming exactly which changes were applied and which were
  not. Cairn MUST NOT report success for an incomplete integration.
- **FR-156**: Recoverability comes from atomic replacement, not from copying the developer's
  files. The original file MUST remain intact and readable until its replacement is complete
  (FR-154), so an interrupted operation leaves the prior state in place. Cairn MUST NOT, as normal
  behavior, persist an additional copy of a pre-existing configuration file, because those files
  routinely carry tokens, provider credentials, and environment secrets that Cairn is forbidden to
  hold (FR-197, FR-200).
- **FR-238**: Where recovery content is needed, Cairn MUST preserve only the Cairn-owned content
  that is about to change — the managed block, the specific owned entry, or a file Cairn generated
  in its entirety — never the surrounding file. If the Cairn-owned content cannot be isolated from
  the rest of the file, Cairn MUST NOT proceed by copying the whole file; it reports the condition
  and changes nothing (FR-137).
- **FR-239**: Recovery artifacts are local machine state. They MUST NOT be synchronized, MUST NOT
  have their content written to logs or diagnostics, and MUST be subject to the same redaction as
  any other stored content. A recovery artifact MUST NOT contain a credential belonging to
  configuration Cairn does not own.
- **FR-157**: All connect, repair, migrate, and disconnect operations MUST be idempotent:
  repeating one changes nothing and reports `unchanged`.
- **FR-158**: Deduplication of Cairn-owned entries MUST be deterministic: given the same input
  configuration, the same entries survive and the same are removed.

### Change preview

- **FR-159**: Cairn MUST be able to preview any integration change before applying it, and the
  preview MUST make no modification of any kind.
- **FR-160**: A preview MUST classify each planned change as add, update, remove, unchanged, or
  conflict, and MUST name the affected resource and file for each.
- **FR-161**: A preview MUST explicitly show what will not be touched — unrelated MCP servers,
  unrelated lifecycle handlers, unrelated instructions — so the developer can see the blast
  radius.
- **FR-162**: A preview MUST NOT print any credential, token, or key value found in the files it
  inspected.

### Onboarding

- **FR-163**: Cairn MUST provide a guided path that detects installed agents and integration
  managers and proposes a complete integration plan, including which owner is proposed for each
  resource.
- **FR-164**: The proposal MUST be presented for confirmation before anything is changed. In
  non-interactive use, applying it without confirmation MUST require an explicit opt-in, and
  MUST still refuse to proceed where a conflict needs a human decision.
- **FR-165**: Where an integration manager is installed and preferred, Cairn MAY propose
  manager-owned distribution for the resources that manager supports while keeping lifecycle and
  instructions directly owned, and MUST show that split in the proposal.

### Diagnostics

- **FR-166**: Cairn MUST provide an integration health check covering core state — component
  version alignment, daemon reachability, project registration — and, for every detected agent
  and manager, the state of each Cairn-owned resource.
- **FR-167**: The health check MUST report per-resource state using a defined, machine-readable
  set of conditions that at minimum distinguishes: healthy; missing; modified; outdated;
  duplicated; conflicting owner; malformed configuration; installed but not yet activated by the
  user; migrating, with its source and target owner (FR-228); manager action required, with the
  supported path (FR-233); and unknown.
- **FR-168**: The health check MUST report, per agent, its detection state, its version where
  obtainable, its version compatibility classification, its capability level, and which expected
  lifecycle coverage is present and which is absent.
- **FR-169**: The health check MUST report, for the integration manager, whether it is detected,
  which Cairn resources it holds, which applications those resources are bound to, and whether
  that matches Cairn's ownership record.
- **FR-170**: The health check MUST be able to run for one agent or for all of them, MUST make no
  modification, and MUST emit Cairn's existing machine-readable envelope.
- **FR-171**: Diagnostic output MUST redact secrets and MUST NOT include the content of user
  instructions, user Skills, or unrelated configuration beyond what is needed to name the
  problem.

### Repair

- **FR-172**: Cairn MUST provide a repair operation that restores missing Cairn-owned resources,
  upgrades outdated Cairn-owned artifacts to the current contract and Skill versions, and removes
  duplicate Cairn-owned entries.
- **FR-173**: Repair MUST change only resources whose recorded owner is Cairn directly, and MUST
  NOT modify any user setting, unrelated entry, or manager-owned resource.
- **FR-174**: Where a conflict has more than one defensible resolution — conflicting ownership,
  a damaged managed block, a Cairn-named resource Cairn does not own — repair MUST explain the
  conflict and its options and change nothing.
- **FR-175**: Repair MUST be idempotent and MUST support the same preview as connect.
- **FR-176**: Repair MUST be able to run for one agent or for all of them.
- **FR-177**: A Cairn-managed resource that the developer has edited by hand MUST be detected as
  modified, MUST be reported with what differs, and MUST NOT be changed by a default repair.
- **FR-221**: A forced repair MUST restore Cairn's canonical version strictly inside Cairn's
  ownership boundary: only the managed instruction block, only the exact Cairn-owned configuration
  entry, or only a file Cairn generated in full. Content outside Cairn's ownership markers MUST
  never be overwritten, under any option.
- **FR-222**: Before a forced repair replaces modified Cairn-managed content, Cairn MUST preserve
  the previous content somewhere the developer can recover it, and MUST say where. What is
  preserved is the Cairn-owned block, entry, or wholly Cairn-generated file only — never the
  enclosing configuration file and never any setting Cairn does not own (FR-238, FR-239).
- **FR-223**: Cairn MUST compare managed resources by meaning, not by bytes. A resource that
  differs from the canonical version only in formatting, whitespace, or the ordering of
  order-insensitive entries MUST be reported healthy, not modified.
- **FR-224**: No recovery path may require the developer to disconnect and reconnect an agent to
  return a modified resource to a healthy state.
- **FR-225**: Repair MUST NOT adopt a developer's edit as Cairn-managed content. Adopting a
  deliberate customization is a separate, explicit operation and is out of scope for this feature.

### Disconnect

- **FR-178**: Disconnecting one agent MUST remove that agent's Cairn lifecycle integration, its
  Cairn-owned managed instruction block, its Cairn-owned Skill, its directly-owned Cairn MCP
  entry, and its local integration record.
- **FR-179**: Disconnect MUST NOT delete any project, task, session, observation, memory, or
  handoff.
- **FR-180**: Disconnect MUST NOT modify any unrelated MCP server, lifecycle handler, plugin,
  instruction, credential, or setting, and MUST NOT affect any other connected agent.
- **FR-181**: Withdrawing an integration manager's distribution of Cairn resources MUST be a
  separate operation from disconnecting a native agent adapter, and either MUST be possible
  without the other. Where the manager exposes no documented automated removal interface, that
  withdrawal completes as the manager-action-required outcome of FR-233 followed by the
  verification of FR-234 — never by Cairn editing the manager's own state.
- **FR-236**: Migration from direct ownership to manager ownership MAY be automated up to the
  manager's supported import and user-confirmation boundary; Cairn drives the import request,
  the developer confirms it inside the manager, and Cairn verifies the resulting application
  configuration before removing the resource it owns directly (FR-148).
- **FR-237**: Migration from manager ownership to direct ownership MUST NOT delete the
  manager-owned resource behind the manager's back. Cairn installs and verifies the direct
  resource, then reports the manager-side withdrawal as manager action required (FR-233), and
  the resource remains recorded as migrating until verification confirms exactly one owner
  (FR-228).

### Local integration state

- **FR-182**: Cairn MUST keep, on the local machine, a record per connected agent and manager
  covering at least: the agent or manager identity, the integration mode and level, the recorded
  owner **and installation scope** of each resource kind, the adapter version, the installed
  contract version, the installed Skill version, when it was connected, and when it was last
  verified.
- **FR-183**: This record is local machine state. Cairn MUST NOT transmit it, or any agent
  configuration path, file content, or integration health detail, to the shared server.
- **FR-184**: The shared server MAY continue to hold the agent identity already carried by
  Feature 001 session provenance (FR-055) and MUST NOT be extended with integration
  configuration.

### Agent version compatibility

- **FR-185**: Cairn MUST classify each detected agent's version as verified, compatible but
  unverified, or unsupported.
- **FR-186**: Cairn MUST NOT refuse to integrate an agent merely because its version is newer
  than any Cairn has verified. Unknown versions are compatible-but-unverified and are used.
- **FR-187**: Cairn MUST report unsupported only for versions it positively knows are
  incompatible, and MUST state what is incompatible.
- **FR-188**: Adapter behavior MUST degrade by capability detection rather than by version
  string matching, so that a minor agent update cannot silently break an integration.

### Cross-agent memory invariant

- **FR-189**: Agent identity is provenance only. Cairn MUST NOT introduce any memory scope,
  partition, ownership domain, or retrieval filter keyed to the agent that produced a record.
- **FR-190**: Memory scoping and ranking MUST continue to use only project, branch, task, and
  session (FR-017, FR-024).
- **FR-191**: Durable knowledge recorded through one agent MUST be retrievable through every
  other connected agent on the same project, with no export, import, copy, or migration step.
- **FR-192**: One repository MUST resolve to one Cairn project regardless of how many agents are
  connected to it (FR-002, FR-004).

### Non-blocking behavior

- **FR-193**: No Cairn lifecycle integration may block, delay beyond its deadline, or fail the
  agent it is attached to. Capture-class handling MUST keep Feature 001's fail-soft rule
  (FR-015): drop the work, log locally, and return success.
- **FR-194**: Where an agent imposes a shorter deadline on a lifecycle handler than Feature 001's
  defaults, the adapter MUST respect the agent's limit, and Cairn MUST document the resulting
  reduction in what can be completed at that boundary.
- **FR-195**: Context delivery at session open MUST retain Feature 001's bounded fallback
  (FR-046): if the briefing is not ready in time, the agent session still starts, with the
  reduced-context state reported.
- **FR-196**: Configuration operations — connect, repair, migrate, disconnect — MUST NOT fail
  soft. They MUST report failure clearly rather than claiming success on an incomplete result.

### Privacy

- **FR-197**: Cairn MUST NOT transmit agent configuration files, local absolute paths, an
  integration manager's configuration, provider credentials, authentication material, API keys,
  user instruction files, or user Skills.
- **FR-198**: All adapter-captured content MUST pass through Feature 001's existing exclusion,
  redaction, and payload-bounding pipeline before storage (FR-013, FR-048, FR-049, FR-050). A
  new adapter MUST NOT create a path around it.
- **FR-199**: Conversation content newly exposed by any agent's lifecycle payload — assistant
  message text, user prompt text, transcripts, raw tool output — MUST NOT be persisted.
- **FR-200**: Cairn MUST NOT manage, read for storage, or transmit provider credentials, OAuth
  material, API keys, model routing, or usage accounting. Those belong to the agent or to the
  integration manager.

### Desired-state model

- **FR-201**: Cairn MUST have exactly one canonical internal representation of desired integration
  state — which agents are selected, which owner and scope each resource kind has, and which
  capability mode is intended. Guided onboarding, preview, diagnostics, repair, ownership
  migration, and manager distribution MUST all operate on that single model rather than each
  deriving its own view of intent.
- **FR-202**: In this feature, desired state MUST be composed from the developer's explicit
  choices, Cairn's local integration record, and the configuration actually detected on the
  machine. Cairn MUST NOT write a project-local desired-state file and MUST NOT treat any
  committed file as authoritative for integration intent.
- **FR-226**: The desired-state model MUST be versioned, deterministic for identical inputs,
  secret-free, and serializable, so that a later feature can expose it as a committed project
  manifest without changing adapter or reconciliation semantics.
- **FR-227**: Feature 002 MUST NOT implement manifest-versus-reality drift handling, manifest
  merge semantics, or automatic application of intent on clone.

### Verification and evidence

- **FR-203**: Cairn MUST include fixture-based adapter tests that translate realistic vendor
  lifecycle payloads and configuration files into canonical events and configuration changes, for
  every native adapter, the integration manager, and the generic MCP path.
- **FR-204**: Those fixture tests MUST run hermetically in required continuous integration, with
  no installed agent, no authentication, and no network.
- **FR-205**: Tests using a live, authenticated agent MAY remain manual release evidence, and
  MUST NOT be required for continuous integration to pass.
- **FR-206**: The existing browser acceptance suite MUST run in hosted continuous integration
  against a release build of the server, alongside linting, type checking, and the production
  build, and MUST cover both the desktop and mobile viewports already exercised locally.

### Key Entities

- **Agent Adapter**: The translation boundary for one coding agent. Knows that agent's
  configuration surfaces and lifecycle vocabulary; produces canonical Cairn events and canonical
  configuration changes; reports that agent's capability profile. Owns no Cairn semantics.
- **Integration Manager**: A tool that distributes Cairn resources to agents on the developer's
  behalf. Has a detection state, a set of target applications, and a set of Cairn resources it
  holds. Never an agent.
- **Capability Profile**: Per agent, which integration surfaces and lifecycle signals that agent
  actually provides, and whether its lifecycle handlers require user activation.
- **Integration Level**: The honest summary of a connected agent — FULL, MCP_PLUS, MCP_ONLY, or
  UNSUPPORTED — derived from the capability profile, the verified state of installed resources, and
  whether a lifecycle-completion guarantee has been demonstrated.
- **Lifecycle Completion Guarantee**: The demonstrated property that Cairn positively establishes
  that a session terminated and finalizes it, without user intervention and within a bounded
  documented contract, producing that session's durable handoff. A vendor session-end signal is the
  preferred mechanism; another deterministic, bounded, tested mechanism qualifies only where it
  establishes termination with equivalent fidelity. Recovery from silence — the inactivity timeout
  and daemon-start reconciliation — is a safety net that backstops this property and never
  constitutes it.
- **Installation Scope**: Where a Cairn-owned resource lives for one agent — committed project,
  developer-local project, or per-user — recorded alongside its owner and never changed implicitly.
- **Desired Integration State**: The single canonical, versioned, deterministic, secret-free model
  of intended agent selection, per-resource owner and scope, and capability mode, composed from
  the developer's choices, the local integration record, and detected configuration. Every
  integration operation reads from it.
- **Canonical Lifecycle Event**: One of Cairn's own lifecycle boundaries — session opened, tool
  succeeded, tool failed, agent quiesced, context compacting, context compacted, session closed —
  carrying the producing agent session's identity. *Agent quiesced* asserts only that the agent
  stopped working and is waiting: it is the strongest claim every supported agent establishes, and
  it implies nothing about success or about the session being over.
- **Resource Migration State**: The recorded, transient condition of a resource moving between
  owners, naming its source and target. Distinct from duplication and from conflicting ownership,
  and either resumable or reversible.
- **Manager Action Required**: The outcome Cairn reports when completing an operation would need a
  manager interface that manager does not document for third parties. Names the resource, the
  target applications, and the supported path the developer follows inside that manager.
- **Recovery Artifact**: The Cairn-owned content preserved before a forced change — a managed
  block, an owned entry, or a wholly Cairn-generated file. Never an enclosing configuration file,
  never a setting Cairn does not own, and never synchronized or logged.
- **Canonical Tool Category**: The Feature 001 observation type a vendor tool call normalizes to,
  optionally with the raw vendor tool name retained as bounded provenance.
- **Cairn Agent Usage Contract**: The single versioned source that teaches an agent how to use
  Cairn, rendered into an always-on form for instruction surfaces and a compact universal form for
  MCP initialization.
- **Managed Instruction Block**: The marked, versioned region Cairn owns inside an agent's
  instruction file. Everything outside it belongs to the developer.
- **Cairn Skill**: The versioned, progressively disclosed workflow material installed for agents
  that support Skills, generated from one canonical source.
- **Cairn-Owned Resource**: One installed thing Cairn manages for one agent — an MCP entry, a
  lifecycle integration, a managed instruction block, or a Skill — with exactly one recorded
  owner: Cairn directly, an integration manager, or external.
- **Integration Change Plan**: The computed set of changes an operation would make, each
  classified add, update, remove, unchanged, or conflict, with the resource and file named.
- **Local Integration Record**: Per-machine state describing what Cairn installed where, under
  which owner and scope, at which versions, and when it was last verified. Never leaves the
  machine.
- **Integration Health Report**: The machine-readable result of diagnostics: core state, per-agent
  state, per-resource condition, and manager state.
- **Version Compatibility Classification**: Per agent, one of verified, compatible-but-unverified,
  or unsupported.

## Success Criteria *(mandatory)*

Feature 002 success criteria are numbered from **SC-101** so that references to Feature 001
criteria stay unambiguous.

### Measurable Outcomes

- **SC-101**: A developer with a supported agent installed can go from `cairn init` to a
  connected, capturing session in that agent in under 5 minutes, using only documented steps, for
  each of Claude Code, Codex, and OpenCode.
- **SC-102**: Running connect twice for the same agent produces zero configuration changes on the
  second run, reported as `unchanged`, for every supported agent and every resource kind.
- **SC-103**: Upgrading a repository configured under Feature 001 produces exactly one Cairn
  lifecycle entry per registered event and exactly one Cairn MCP entry, and every pre-existing
  non-Cairn entry in every touched file is byte-identical before and after.
- **SC-104**: Across a corpus of at least 20 realistic pre-existing agent configuration files —
  including files with comments, unusual ordering, unrelated servers, and unrelated handlers —
  connect followed by disconnect returns each file to a state where 100% of non-Cairn content is
  byte-identical to the original.
- **SC-105**: The always-on usage contract rendering stays within its documented size bound in
  100% of installations, asserted automatically.
- **SC-106**: Cairn's MCP surface remains exactly six tools; a test fails if a seventh appears.
- **SC-107**: A plain MCP client with no native adapter completes initialization, receives the
  usage contract where the protocol carries it, and successfully exercises all six tools, while
  the reported level is MCP_ONLY and names automatic lifecycle and automatic capture as
  unavailable.
- **SC-108**: In the cross-agent continuity scenario, knowledge recorded in the first agent is
  retrieved by the second and third agents in the same session in which they open the repository,
  with zero manual export, import, or copy steps, and one project exists for the repository.
- **SC-109**: With two supported agents running concurrently in one worktree, Cairn holds exactly
  two active sessions, 100% of observations and memories carry the provenance of the session that
  produced them, zero events are routed to the wrong session, and every unattributable request
  returns an ambiguous-session error rather than a guess.
- **SC-110**: For each supported agent, every canonical lifecycle event the capability profile
  claims is demonstrated by a fixture test that drives a realistic vendor payload through the
  adapter and asserts the canonical result; and for every capability the profile does not claim,
  a test asserts the adapter emits nothing.
- **SC-111**: No adapter maps an idle, quiet, or inactive vendor signal to session closed,
  asserted by test for every adapter.
- **SC-112**: Distributing Cairn's MCP server through the integration manager to a chosen set of
  applications results in exactly one Cairn MCP entry per selected application, zero changes to
  unrelated manager-held configuration, and zero conflicting-ownership findings in diagnostics.
- **SC-113**: After switching provider or configuration inside the integration manager,
  diagnostics report every Cairn resource healthy with zero duplicates.
- **SC-114**: For each of at least 8 distinct introduced defects — a deleted lifecycle entry, a
  damaged managed block, an outdated contract version, an outdated Skill version, a duplicated MCP
  entry, a resource under the wrong owner, a malformed configuration file, and an inactive
  lifecycle handler — diagnostics identify the exact resource and the correct condition in 100% of
  cases.
- **SC-115**: Repair fixes 100% of introduced defects that are Cairn-owned and unambiguous,
  changes zero non-Cairn settings, and reports nothing to do when run a second time.
- **SC-116**: Disconnecting one agent leaves 100% of projects, tasks, sessions, observations,
  memories, and handoffs intact, leaves every other connected agent's configuration unchanged, and
  leaves every unrelated setting in the touched files byte-identical.
- **SC-117**: Migrating a resource between owners never produces a state in which the developer has
  no effective resource, verified by inspecting configuration after every step: where both owners
  write one effective slot, zero intermediate states exist at all; where they write distinct
  locations, every intermediate state has exactly one unambiguous effective resource and is
  recorded as migrating. On completion exactly one owner remains, and on induced failure at each
  step the previously working configuration is intact.
- **SC-118**: Preview mode produces zero filesystem modifications, verified by comparing a
  checksum of every candidate file before and after, across every supported operation.
- **SC-119**: Zero credential, token, or key values appear in preview output, diagnostic output,
  or logs, verified against configuration files seeded with recognizable secrets.
- **SC-120**: Zero agent configuration content, absolute path, or integration health detail
  appears in any outbound sync payload or in the shared server's database, verified by inspecting
  both.
- **SC-121**: Zero assistant message text, user prompt text, or raw tool output from any agent's
  lifecycle payload is present in stored observations, memories, or handoffs, verified with
  payloads seeded with recognizable conversation text.
- **SC-122**: Across at least 200 capture-class lifecycle invocations per adapter using release
  builds, Cairn stays within Feature 001's latency budget (SC-007) and within each agent's own
  handler deadline, and zero Cairn failures abort or visibly disrupt an agent session.
- **SC-123**: An agent version newer than any Cairn has verified is classified
  compatible-but-unverified and integrates successfully; only a positively known-incompatible
  version is reported unsupported.
- **SC-124**: Adapter fixture tests for every native adapter, the integration manager, and the
  generic MCP path pass in required continuous integration with no agent installed, no
  credentials, and no network.
- **SC-125**: The hosted continuous integration pipeline runs linting, type checking, the
  production build, and the browser acceptance suite on both desktop and mobile viewports against
  a release-build server, and a browser regression fails the build.
- **SC-126**: Connecting an agent writes only to the locations the recorded scope for each
  resource kind allows: with default scopes, connecting produces zero committed-file changes for
  lifecycle handlers, and choosing the shared option produces exactly the committed changes it
  described in preview and no others.
- **SC-127**: No agent is reported FULL unless a test demonstrates a mechanism that positively
  establishes that its sessions terminated and finalizes them inside a bounded contract. Recovery
  from silence never counts towards this (SC-131). An agent lacking that demonstration is reported
  below FULL with automatic session completion named as the missing behavior.
- **SC-128**: **Nominal performance.** Across at least 100 session-end boundaries per agent that
  imposes a handler deadline, measured with release builds and a healthy daemon, Cairn's
  session-end work completes inside that agent's own budget in 100% of runs, and the adapter
  exceeds the agent's deadline in none.
- **SC-129**: **Injected-failure recovery**, tested separately from SC-128. For each induced
  condition — handler timeout, handler crash, and daemon unavailable at the boundary — the session
  is subsequently reconciled with a durable handoff, zero sessions are left permanently without
  one, and zero agent sessions are aborted or visibly disrupted by the failure.
- **SC-130**: A Cairn-managed resource that differs from the canonical version only in formatting,
  whitespace, or the ordering of order-insensitive entries is reported healthy in 100% of seeded
  cases, and a resource with a semantic edit is reported modified in 100% of seeded cases, with
  default repair changing neither.
- **SC-131**: No agent reaches FULL on the strength of recovery-from-silence. With the inactivity
  timeout and daemon-start reconciliation as the only routes to a terminal session state, the
  computed level is below FULL in 100% of cases, and the report states that sessions are closed by
  inactivity rather than completed — asserted for OpenCode specifically under currently verified
  capabilities.
- **SC-132**: Withdrawing a manager-owned resource where the manager documents no automated
  removal interface produces zero writes to the manager's own storage, verified by checksumming
  that storage before and after; the outcome is reported as manager action required with the
  supported path; and after the developer acts, verification against the target applications'
  real configuration updates the ownership record in 100% of cases.
- **SC-133**: Across every connect, repair, migrate, and disconnect operation on configuration
  files seeded with recognizable credentials, zero credentials belonging to configuration Cairn
  does not own appear anywhere in Cairn's recovery artifacts, local state, logs, diagnostics, or
  sync payloads; and zero whole-file copies of pre-existing configuration are created as normal
  behavior.
- **SC-134**: An agent quiescence signal that follows an error produces exactly one checkpoint and
  zero synthesized success or failure observations, asserted by fixture for every adapter whose
  quiescence signal can follow a failure.
- **SC-135**: The desired-state model serializes deterministically — identical inputs produce
  byte-identical output across runs and machines — contains zero secrets when serialized from
  configurations seeded with recognizable secrets, and is the single input consumed by onboarding,
  preview, diagnostics, repair, and manager migration, asserted by test.

## Assumptions

### Verified integration surfaces

The requirements above were written against current official sources, checked on 2026-08-11.
Detailed findings belong in the planning research artifact; the assumptions this specification
rests on are:

- **Claude Code** exposes a lifecycle hook system configured through its settings files at user,
  project, and local scope. It distinguishes successful from failed tool calls as separate
  events, distinguishes finishing a turn from ending a session, and signals both before and after
  compaction. It supports injecting additional context at session start. It configures MCP servers
  at local, project, and user scope, with project scope living in a committed file and matching
  duplicates by name across scopes. It reads project instructions from its own instruction file —
  not from a shared agent instruction file — and supports Skills at personal, project, and plugin
  locations. Block-level HTML comments in instruction files are stripped before the content
  reaches the model, so ownership markers in that form cost no context.
- **Codex** exposes a lifecycle hook system with events covering session start, session end, turn
  stop, tool use before and after, permission requests, prompt submission, subagent start and
  stop, and both pre- and post-compaction. Its payloads carry a stable session identifier, the
  working directory, and a turn identifier. It does **not** have a separate tool-failure event:
  success and failure both arrive on the post-tool event and must be classified from the payload's
  own result data. Its session-end handler runs under a very short deadline — far shorter than
  Feature 001's boundary deadline — and its session-end reason is not currently differentiated.
  Externally supplied hooks are trust-gated: a newly written hook does not run until the user
  trusts it inside Codex, and changing an already-trusted hook invalidates that trust. Its
  configuration is TOML, layered with a project layer that overrides the user layer, with MCP
  servers configured under a dedicated table. It supports Skills with user and repository scope.
- **OpenCode** exposes a plugin system whose hooks cover tool execution before and after, chat
  messages, permissions, command execution, shell environment, tool definitions, and — as
  experimental hooks — compaction. Session lifecycle reaches plugins through a general event bus
  whose session events include created, updated, deleted, idle, status, error, and compacted.
  There is **no session-ended event**: idle means the session went quiet, not that it finished. Its
  configuration is JSON, at project and global scope, with MCP servers and plugin registrations as
  separate keys and an additional-instruction-files key. It reads instruction files from the shared
  agent instruction file and also from Claude Code's instruction file, taking the first project
  match rather than stacking them. It discovers Skills from its own directories and also from
  Claude Code's Skill directory, which means one installed Skill can be visible to more than one
  agent.
- **CC Switch** is a desktop configuration manager, not a coding agent. It manages providers, MCP
  servers, prompts, and Skills across a set of applications that includes Claude Code, Codex, and
  OpenCode. Its MCP distribution writes to each target application's **global** configuration, not
  to project configuration. Its Skill installation places Skills in each application's global Skill
  directory, sourced from a Git repository. Its own state lives in a private database that third
  parties must not write to. Its officially documented third-party integration surface is a deep
  link import protocol supporting provider, MCP, prompt, and Skill resources with an explicit
  target application list and a user confirmation dialog.
- **MCP** carries an optional server-instructions field in the initialization result, described by
  the specification as a hint clients may add to the model's system prompt. Whether a given client
  surfaces it is a client decision, so Cairn treats delivery as best-effort and does not claim the
  contract was received.

### Product assumptions

- Feature 001's contracts remain in force. Feature 002 extends them additively: a post-compaction
  canonical event, optional raw-tool-name provenance on observations, MCP server instructions at
  initialization, and integration state that stays local. Nothing in Feature 001's memory model,
  scope model, session model, privacy boundary, or six-tool MCP surface changes.
- "Native integration" means Cairn uses the agent's own officially supported configuration and
  lifecycle surfaces. It does not mean every agent reaches the same integration level; levels are
  computed from what each agent actually provides, and FULL additionally requires a demonstrated
  completion guarantee rather than a vendor event of a particular name.
- Installation scope is chosen per resource kind: instructions project-scoped and commit-safe;
  lifecycle handlers project-scoped but developer-local; the MCP registration per-user by default;
  the Skill per-user by default; manager-distributed resources per-user by that manager's model.
  A shared option exists for teams that want lifecycle and MCP committed. Cloning never activates
  anything.
- Automatic configuration of an agent is performed only through surfaces that agent documents for
  configuration. Where an agent requires the user to confirm, trust, or activate what Cairn
  installed, that step belongs to the user and Cairn reports the integration as incomplete until it
  is done.
- CC Switch is treated strictly as a distribution and configuration manager for Cairn's MCP server
  and Cairn's Skill. Provider switching, credentials, routing, pricing, and usage accounting stay
  entirely with CC Switch.
- Distributing the Cairn Skill through the integration manager requires the Skill to be available
  from a public Git repository, because that is the manager's documented Skill import mechanism.
- Agent versions are read from what the agent exposes locally without authentication. Where an
  agent exposes no version, it is treated as compatible-but-unverified.
- Windows remains out of scope, matching Feature 001's supported platforms.
- Integration management is command-line first. The web interface may continue to display the
  agent identity already present in session provenance, and Feature 002 adds no browser-side claim
  about the health of a local machine's agent configuration, because the server cannot observe it.
- The web package currently has no unit test script; the required web pipeline is linting, type
  checking, the production build, and the browser acceptance suite, with unit tests added to it
  if and when they exist.

## Out of Scope

Explicitly not part of Feature 002, and not to be built speculatively:

- Embeddings, vector databases, semantic retrieval, knowledge graphs, memory decay, confidence
  scoring, automatic fact verification, truth engines, and stale-memory intelligence.
- Agent-to-agent messaging, agent delegation, multi-agent orchestration, autonomous task
  scheduling, and remote code execution.
- Provider and model routing, provider credential management, OAuth handling, billing, pricing,
  and usage accounting — including any reimplementation of the integration manager's provider
  features.
- Writing to any integration manager's private storage, or any integration path that is not a
  documented public interface of that manager.
- Native lifecycle adapters for any agent beyond Claude Code, Codex, and OpenCode — including
  Gemini, Hermes, and every other application a detected manager happens to support. Those reach
  Cairn through the generic MCP path only.
- Any new memory scope, partition, or ownership domain based on agent identity.
- Expanding the MCP tool surface beyond the six Feature 001 tools.
- A second Cairn service, broker, or datastore introduced to support adapters.
- Broad server architecture changes and any large web dashboard expansion; integration management
  is command-line first.
- Automatic resolution of ambiguous ownership conflicts without the developer's decision.
- A committed project-local integration manifest. Feature 002 builds the canonical desired-state
  model behind it (FR-201, FR-226) but writes no such file, treats no committed file as
  authoritative for integration intent, and implements no manifest drift handling, manifest merge
  semantics, or automatic application on clone.
- Adopting a developer's hand-edited version of a Cairn-managed resource as the new Cairn-managed
  content. Repair reports the edit; a deliberate adopt operation belongs to a later feature.
- Relocating existing integrations between scopes automatically. Scope changes are explicit,
  previewable migrations.

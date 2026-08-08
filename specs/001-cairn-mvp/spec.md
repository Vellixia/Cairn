# Feature Specification: Cairn MVP

**Feature Branch**: `001-cairn-mvp`

**Created**: 2026-08-07

**Status**: Draft

**Input**: User description: "Cairn is persistent, project-aware memory for AI coding agents. It connects to coding agents such as Claude Code through MCP and lifecycle hooks. Remember useful project knowledge across AI sessions, preserve session progress, give the next session relevant context automatically, understand where memory applies, provide reliable handoff between sessions, and optionally share project memory with other project members."

## Overview

Today an AI coding session starts blind. Everything the previous session learned —
what the goal was, what was tried, what failed, which decisions were already made,
which conventions this repository follows — is lost when the context window ends.
The developer re-explains, the agent re-discovers, and the same dead ends get walked
twice.

Cairn is a local-first memory layer that sits beside the coding agent. It knows which
repository, branch, and commit the work is happening on; it captures the useful facts
a session produces; it turns the important ones into scoped, durable memory; and it
hands the next session a bounded briefing so work resumes instead of restarting.

Feature 001 delivers a Cairn that is genuinely usable end to end: install it, connect
Claude Code, work normally, and the next session picks up where the last one stopped.
Sharing memory with teammates is opt-in and layered on top of a system that is fully
useful offline.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Session work is captured and handed off (Priority: P1) 🎯 First usable slice

A developer installs Cairn, connects it to Claude Code, and opens a Git repository.
Cairn recognizes the repository, branch, and commit, and starts a session
automatically when the agent session starts. As the agent works — reading files,
editing files, running commands and tests, hitting errors — Cairn records structured
observations rather than raw transcripts. When the session ends, is interrupted, or is
about to compact, Cairn writes a structured handoff: what the work was, what got done,
what changed, what failed, what remains, and the recommended next step. The developer
can read that handoff from the command line. Finishing a single turn is not the end of a
session, so it produces a checkpoint rather than a handoff.

**Why this priority**: This is the foundation of the product and is the first slice
that is usable on its own — a developer gets a reliable written record of what each AI
session actually did. It is the starting point of the MVP, not the MVP itself; Feature
001 is complete only when every capability below is delivered.

**Independent Test**: In a scratch Git repository, connect Claude Code, run a short
session that edits a file and runs a failing test, stop the session, then run
`cairn handoff show`. The handoff must name the changed file, the failing test, and a
next step, without the developer having written any of it.

**Acceptance Scenarios**:

1. **Given** a Git repository that has never been used with Cairn, **When** the
   developer runs `cairn init` and starts a Claude Code session, **Then** Cairn
   registers the project, records repository, branch, commit, and worktree, and
   creates an active session bound to that state.
2. **Given** an active Cairn session, **When** the agent reads files, edits files,
   runs commands, runs tests, and encounters an error, **Then** Cairn stores one
   structured observation per event with its type and bounded structured fields, and
   stores no full conversation transcript.
3. **Given** an active session with recorded observations, **When** the session ends,
   **Then** Cairn marks the session `completed` and produces a handoff containing goal,
   progress, completed work, remaining work, changed files, decisions, failures, tests
   executed, repository state, and a recommended next step.
4. **Given** an active session, **When** the main agent finishes responding to a turn,
   **Then** Cairn records a turn checkpoint, the session **remains `active`**, and no
   durable handoff is produced — the developer may send another prompt, so finishing a turn
   is not the end of a session.
5. **Given** an active session that reaches context compaction, **When** the compaction
   boundary is hit, **Then** Cairn produces a handoff at that boundary and the session
   remains usable afterwards.
6. **Given** a session whose agent exits without a clean stop, **When** the daemon next
   starts, **Then** that session is marked `interrupted` and has a handoff generated from
   the observations it recorded — and if a later event arrives for it, it resumes as
   `active` with that handoff retained.
7. **Given** the Cairn daemon is not running, **When** the agent session starts,
   **Then** Cairn starts the daemon automatically and the agent session is not blocked
   or slowed by Cairn's unavailability.

---

### User Story 2 - The next session starts already informed (Priority: P1)

The developer starts a new AI session on the same repository the next morning. Before
the agent does anything, Cairn assembles a bounded briefing from what it already
knows — project, repository, branch, commit, working-tree state, the previous
session's handoff, relevant memory, decisions already made, known failures, and
remaining work — and gives it to the agent. The agent continues the work instead of
asking what happened yesterday.

**Why this priority**: This is the payoff of capture. Without it, Cairn is a logbook;
with it, Cairn is memory. It is the behavior the product is judged on.

**Independent Test**: Seed a repository with a completed session and handoff, start a
new session, and inspect the briefing the agent receives. It must contain the previous
handoff's remaining work and the current branch and commit, and must fit the context
budget.

**Acceptance Scenarios**:

1. **Given** a repository with a previous completed session and handoff, **When** a new
   agent session starts, **Then** the agent receives a briefing containing project,
   repository, current branch, current commit, working-tree state, previous handoff,
   relevant memory, decisions, known failures, and remaining work.
2. **Given** a briefing is assembled, **When** it is delivered to the agent, **Then**
   its size stays within the configured context budget and lower-priority sections are
   truncated before higher-priority ones.
3. **Given** an agent mid-session that needs more than the briefing contains, **When**
   it calls the context tool again, **Then** it receives a refreshed briefing reflecting
   the current branch, commit, and working-tree state.
4. **Given** a repository Cairn has never seen, **When** a session starts, **Then** the
   briefing is still produced, states that there is no prior history, and does not fail.
5. **Given** the developer switched branches since the last session, **When** the new
   session starts, **Then** the briefing reflects the current branch and prioritizes
   memory scoped to it.

---

### User Story 3 - Durable, scoped memory and recall (Priority: P2)

While working, the agent learns things worth keeping: this project uses a particular
convention, this approach was rejected for a reason, this integration test always
fails without a local service running. The agent records these as memories with an
explicit scope — this project, this branch, this task, or just this session — and a
type. Later, in any session, the agent or the developer can search memory and get
results ranked by relevance and scope, each traceable back to the originating session and
any supporting observations.

**Why this priority**: Handoff carries a session forward; scoped memory carries
knowledge across weeks, branches, and tasks. It is the difference between continuity
and accumulation.

**Independent Test**: Record memories at project, branch, and task scope, then search
from a session bound to one task on one branch. Results must favor task scope, then
branch, then project, and each result must name its originating session.

**Acceptance Scenarios**:

1. **Given** an active session, **When** the agent records a memory with a type and a
   scope, **Then** Cairn stores it with state `active`, its scope key, and provenance
   linking it to the session and zero or more supporting observations.
2. **Given** memories exist at project, branch, and task scope, **When** memory is
   searched from a session on a known branch and task, **Then** results are ordered by
   task scope first, then branch, then project, with lexical relevance and recency
   breaking ties within a scope.
3. **Given** a search query with explicit filters, **When** the search runs, **Then**
   only memories matching the filters are returned.
4. **Given** an existing memory that is later contradicted, **When** the agent records
   the replacement and marks the original superseded, **Then** the original is retained
   in state `superseded`, is excluded from default results, and remains linked to its
   replacement.
5. **Given** a memory whose branch no longer exists, **When** memory is searched,
   **Then** it can be marked `stale` and is excluded from default results while
   remaining retrievable on request.
6. **Given** the central server is unreachable, **When** memory is recorded and
   searched, **Then** both succeed against local storage.

---

### User Story 4 - Work is organized by task (Priority: P2)

The developer names what they are working on: a task with a title, a goal, and
acceptance criteria. Sessions bind to that task, memory can be scoped to it, and the
briefing leads with its goal and criteria. When the work is done or blocked, the task
status reflects it, and the next session on that task resumes with the accumulated
task memory rather than starting from the branch alone.

**Why this priority**: Scope precision is what separates Cairn from a session log.
It is not required for the first useful loop, but it is what makes recall accurate.

**Independent Test**: Create a task with a goal and acceptance criteria, start a
session bound to it, end the session, then start a new session on the same task. The
briefing must lead with the task goal and criteria and include the previous session's
handoff for that task.

**Acceptance Scenarios**:

1. **Given** a project, **When** the developer or agent creates a task with a title,
   goal, and acceptance criteria, **Then** the task is stored with status `todo`.
2. **Given** existing tasks, **When** a session starts, **Then** the developer or agent
   can select an existing task or create one, and the session records the binding.
3. **Given** a session bound to a task, **When** the briefing is assembled, **Then** it
   leads with that task's goal and acceptance criteria and prioritizes task-scoped
   memory.
4. **Given** a task in progress, **When** its status is changed to `in_progress`,
   `done`, or `blocked`, **Then** the change is persisted and visible to later sessions.
5. **Given** a session started without selecting a task, **When** work proceeds,
   **Then** the session remains valid and is scoped to the project and branch only.

---

### User Story 5 - The developer controls what Cairn stores (Priority: P2)

Cairn holds work data, so the developer must be able to see it, limit it, and remove
it. By default nothing leaves the machine, no conversation is persisted, no raw
command output is stored wholesale, and obvious secrets are redacted before anything
is written. Beyond the defaults, the developer can exclude paths and commands from
capture, mark memory as local-only, and delete any observation, memory, session, or
handoff — with deletion scoped to what was named, so removing a session never silently
destroys the durable knowledge it produced.

**Why this priority**: Privacy is a functional requirement of the MVP, not a later
hardening pass. Developers will not attach a memory system to their work if they
cannot bound and undo what it keeps.

**Independent Test**: Configure an exclusion for a secrets directory, run a session
that reads a file in it and echoes an API key, then inspect stored observations. The
excluded path must be absent, the key must be redacted, and deleting the session must
clear the session and its observation content while leaving the memories and handoffs it
produced intact with their origin marked deleted.

**Acceptance Scenarios**:

1. **Given** default configuration, **When** a session runs, **Then** no full
   conversation transcript and no unbounded raw command output are persisted, and every
   stored payload is within the configured size bound.
2. **Given** content matching a known secret pattern, **When** an observation is
   written, **Then** the matching value is redacted before storage.
3. **Given** a configured path or command exclusion, **When** the agent touches it,
   **Then** no observation is recorded for it.
4. **Given** a memory marked local-only, **When** the project is linked to a server and
   sync runs, **Then** that memory is never transmitted.
5. **Given** any stored observation, memory, session, or handoff, **When** the developer
   deletes it, **Then** that record's content is removed from local storage and, if it had
   been shared, its deletion is propagated on the next successful sync.
6. **Given** a session that produced durable memories and a handoff, **When** the developer
   deletes the session, **Then** the session and its observation content are cleared but
   those memories and that handoff survive with their origin marked deleted, and they are
   removed only when the developer explicitly asks for them.

---

### User Story 6 - Project memory shared with teammates (Priority: P3)

A developer decides this project's memory should be shared. They link the project to a
Cairn server, sign in, and from that point project, branch, and task memory, tasks,
sessions, and handoffs sync to the server, where other members of that project can see
and search them. Sync is opt-in per project, tolerates being offline, and never blocks
local work.

**Why this priority**: Shared memory is what turns Cairn from a personal tool into a
team tool, but the local product must be complete and useful before it.

**Independent Test**: Link a project on one machine, record memory, sync, and confirm a
second member of the same project can search and read it. Disconnect the network,
continue working, reconnect, and confirm the queued changes arrive exactly once.

**Acceptance Scenarios**:

1. **Given** an unlinked project, **When** the developer links it to a server and
   authenticates, **Then** the project is registered on the server with the developer as
   a member, and sync begins.
2. **Given** a linked project, **When** local memory, tasks, sessions, or handoffs
   change, **Then** the changes are queued and delivered to the server, and replaying a
   delivery does not create duplicates or change the result.
3. **Given** a linked project and an unreachable server, **When** work continues,
   **Then** all local operations succeed, changes accumulate in the queue, and delivery
   resumes automatically when the server is reachable.
4. **Given** two members of one project, **When** one records shared project memory and
   sync completes, **Then** the other can search and read it — locally and in the web UI —
   with its provenance references intact, while the observation content behind it stays on
   the machine that captured it.
5. **Given** a user who is not a member of a project, **When** they request that
   project's data, **Then** the request is refused.
6. **Given** a project that was never linked, **When** any sync runs, **Then** nothing
   about that project leaves the machine.

---

### User Story 7 - Seeing and managing memory in a browser (Priority: P3)

A developer or teammate opens the Cairn web UI to see what Cairn knows: which projects
exist, what is happening in a project, what tasks are open, what recent sessions did
and what they handed off, and what is in memory — with the ability to search it, read
its provenance, and remove what should not be there.

**Why this priority**: Memory that cannot be inspected cannot be trusted or corrected.
The UI is how a team audits and curates shared memory, but it depends on the server
being in place.

**Independent Test**: With a linked project containing tasks, sessions, handoffs, and
memory, open the web UI and complete these without touching a terminal: find the
project, read the latest handoff, search memory for a known fact, and delete one
memory.

**Acceptance Scenarios**:

1. **Given** an authenticated member, **When** they open the UI, **Then** they see the
   projects they are a member of and can open one.
2. **Given** a project, **When** they open its overview, **Then** they see current
   repository and branch activity, open tasks, and recent sessions.
3. **Given** a project's sessions, **When** they open one, **Then** they see its handoff
   in full, including changed files, decisions, failures, and recommended next step.
4. **Given** a project's memory, **When** they search it with a query and scope filter,
   **Then** they see matching memories with type, scope, state, and originating session,
   and can delete one.
5. **Given** a linked project, **When** they open sync status, **Then** they see whether
   local changes are pending, when the last successful sync happened, and any failure.

---

### Edge Cases

- The directory is not a Git repository, or Git is not installed: Cairn reports this
  clearly and does not create a project or session.
- The repository has no commits yet, or `HEAD` is detached: repository state records
  what exists and the session still starts.
- The developer changes branch, commits, or rebases mid-session: observations and the
  handoff record the repository state at the time each was captured, not only at start.
- Two agent sessions run concurrently — in two worktrees of one repository, or in the
  same worktree: each is a distinct Cairn session, every lifecycle event is routed to the
  session that produced it, and neither corrupts or ends the other.
- The agent produces a very large tool payload or a long-running command's output:
  the stored observation is bounded and summarized rather than truncated arbitrarily
  mid-record.
- The agent session ends without a clean stop, or the machine sleeps or loses power:
  the session is recoverable as `interrupted` with a handoff from what was captured.
- Local storage is corrupt, locked, or the disk is full: Cairn fails soft, reports the
  condition, and does not block or crash the agent session.
- The same handoff or memory is delivered to the server more than once: the server
  result is identical to a single delivery.
- The server rejects a queued change permanently: the failure is surfaced with the
  affected item rather than retried forever in silence.
- A memory's branch or task no longer exists: the memory is marked stale rather than
  deleted, and is excluded from default recall.
- The context budget is exhausted before all sections fit: the briefing degrades in a
  defined order and always states that it was truncated.

## Requirements *(mandatory)*

### Repository and project awareness

- **FR-001**: Cairn MUST detect, for the working directory, whether it is inside a Git
  repository and identify the local repository instance — resolved Git common directory,
  worktree path, current branch, and current commit.
- **FR-002**: Cairn MUST register a local project for a repository instance on first use,
  recording a locally generated identifier, a name, and the local repository instance it
  belongs to, and MUST reuse that project on subsequent use. Filesystem paths identify the
  local instance only; Cairn MUST NOT treat any filesystem path as the identity of a
  shared project (see FR-064).
- **FR-003**: Cairn MUST capture working-tree state (staged, unstaged, and untracked
  changes) as part of repository state.
- **FR-004**: Cairn MUST treat two worktrees of the same repository as the same project
  but distinct working contexts.
- **FR-005**: Cairn MUST report a clear, actionable error when the working directory is
  not a Git repository or Git is unavailable, without creating partial state.

### Sessions

- **FR-006**: Cairn MUST create a session automatically when an agent session starts,
  recording user, agent, project, task if bound, branch, commit, worktree, start time,
  and status.
- **FR-007**: Cairn MUST support session statuses `active`, `completed`, and
  `interrupted`, and MUST record end time when a session leaves `active`.
- **FR-008**: Cairn MUST link a session to its predecessor via an optional
  `previous_session_id` when one exists for the same task or branch, resolved
  deterministically as the most recently ended qualifying session.
- **FR-009**: Cairn MUST NOT claim to detect whether an agent process is still alive — the
  Claude Code hook payloads carry `session_id`, `transcript_path`, `cwd`, and event
  metadata, and no liveness signal (see research D16). Instead, sessions leave `active` at
  deterministic boundaries only: a `SessionEnd` event, an explicit end command, or daemon
  start — a `Stop` event is **not** such a boundary. On daemon start, every session still
  `active` is reconciled to `interrupted` and gets a `recovered` handoff from its recorded
  observations; if a later event arrives for that session it resumes to `active`, and the
  handoff already written stands as a valid boundary record. `cairn status` MAY report how long a session has been idle, but MUST NOT
  reclassify it on that basis.
- **FR-010**: Cairn MUST allow any number of sessions to be active concurrently,
  including two agent sessions in the same worktree. A session's identity is its own
  Cairn session identifier, keyed to the agent session that opened it; the worktree is
  scope and context, never the uniqueness key. Concurrent sessions MUST NOT overwrite or
  terminate one another, and each lifecycle event MUST be routed to the session that
  produced it.

### Observation capture

- **FR-011**: Cairn MUST record observations of these types: `file_read`,
  `file_changed`, `command_run`, `test_run`, `error`, `decision`, `discovery`, and
  `user_instruction`.
- **FR-012**: Cairn MUST store observations as structured fields — such as path,
  command, exit status, test outcome, error identity — rather than raw tool payloads.
- **FR-013**: Cairn MUST bound the size of any stored observation payload to a
  configured maximum and summarize rather than store oversized content.
- **FR-014**: Cairn MUST associate every observation with its session and the repository
  state at the moment of capture.
- **FR-015**: Capture MUST NOT block, delay, or fail the agent's own operation. Capture
  hooks MUST return within a short bounded deadline, MUST drop the observation rather than
  wait when Cairn is slow or unavailable, and MUST never fail the agent session.

### Memory

- **FR-016**: Cairn MUST support memory types `fact`, `decision`, `convention`,
  `failure`, and `procedure`.
- **FR-017**: Cairn MUST support memory scopes `project`, `branch`, `task`, and
  `session`, and every memory MUST carry exactly one scope with its scope key.
- **FR-018**: Cairn MUST support memory states `active`, `stale`, and `superseded`, with
  `active` the default and only `active` memories returned by default.
- **FR-019**: Cairn MUST record, for every memory, an origin session identifier, which is
  mandatory, and zero or more supporting observation references, which are not. A memory
  created where automatic capture is unavailable — manual MCP mode, the command line — is
  valid with no evidence, and Cairn MUST NOT fabricate observations to populate the
  reference set.
- **FR-020**: Cairn MUST allow a memory to be superseded by another, retaining the
  original and the link between them.
- **FR-021**: Cairn MUST allow memory to be created by the agent through its tool
  interface and by the developer through the command line.

### Retrieval

- **FR-022**: Cairn MUST support exact filtering of memory by project, scope, scope key,
  type, and state.
- **FR-023**: Cairn MUST support lexical full-text search over memory content with
  relevance ranking.
- **FR-024**: Cairn MUST rank results by scope precedence — current task, then current
  branch, then project — with lexical relevance and recency applied within a scope.
- **FR-025**: Cairn MUST NOT require embeddings, a vector store, or a knowledge graph
  for any retrieval path.
- **FR-026**: Cairn MUST return search results with enough provenance to identify the
  originating session and any supporting observations — session identifier, zero or more
  local observation identifiers, and an evidence count that may be zero. Observation
  content itself is resolved locally and is never part of the provenance record that leaves
  the machine.

### Context briefing

- **FR-027**: Cairn MUST assemble a briefing at session start, at session continuation,
  and on explicit refresh, and MUST either deliver it or cleanly decline it within the
  configured context deadline (see FR-046).
- **FR-028**: The briefing MUST contain project, repository, current branch, current
  commit, working-tree state, task goal and acceptance criteria when a task is bound,
  relevant project, branch, and task memory, the previous session's handoff, important
  decisions, known failures, and remaining work.
- **FR-029**: The briefing MUST NEVER exceed its configured **Cairn-estimated-token
  budget**, whose default target is 2,000–4,000 estimated tokens. The budget is denominated
  in Cairn's own documented estimator, not in any specific model's tokenizer; Cairn makes no
  claim of exact model-token compliance. Compliance against the estimator is deterministic
  and total: the assembler measures each section with the estimator before emitting it and
  stops at the budget. The estimator MUST be conservative, over- rather than
  under-estimating, and its approximation error against a real tokenizer MUST be measured
  and recorded. The briefing MUST state when content was omitted and name the omitted
  sections.
- **FR-030**: The briefing MUST degrade in a defined priority order when the budget is
  exceeded, dropping lower-priority sections first.
- **FR-031**: The briefing MUST be produced successfully for a project with no prior
  history, stating that no prior history exists.

### Handoff

- **FR-032**: Cairn MUST generate a durable handoff at three boundaries — the compaction
  boundary, session end, and reconciliation of a still-`active` session at daemon start
  (FR-009) — recording which boundary produced it. The end of an agent *turn* is a turn
  checkpoint, not a session boundary: Cairn MUST record it, MUST leave the session `active`,
  and MUST NOT produce a durable handoff for it.
- **FR-033**: A handoff MUST contain goal, progress, completed work, remaining work,
  changed files, important decisions, failures, tests executed, repository state, and a
  recommended next step.
- **FR-034**: Handoff content MUST be derived from Cairn's recorded state and
  observations; any agent-supplied narrative MUST be a bounded, clearly attributed
  addition rather than the source of record.
- **FR-035**: Cairn MUST make handoffs readable from the command line and available to
  the next session's briefing.

### Tasks

- **FR-036**: Cairn MUST support tasks with an identifier, title, goal, acceptance
  criteria, and status, belonging to a project.
- **FR-037**: Cairn MUST support task statuses `todo`, `in_progress`, `done`, and
  `blocked`, and MUST allow status changes.
- **FR-038**: Cairn MUST allow a session to bind to an existing task or to a task created
  at session start, and MUST allow sessions with no task.
- **FR-039**: Cairn MUST NOT implement immutable task revision history in this feature.

### Agent integration

- **FR-040**: Cairn MUST expose an MCP interface limited to the tools `cairn_context`,
  `cairn_search`, `cairn_remember`, `cairn_session`, `cairn_task`, and `cairn_handoff`.
- **FR-041**: Cairn MUST integrate with the Claude Code lifecycle hooks `SessionStart`,
  `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `Stop`, and `SessionEnd`. `PostToolUse`
  fires after a successful tool execution and MUST produce the corresponding success
  observation; `PostToolUseFailure` fires after a failed tool execution, carries the failure
  data, and MUST produce the `error` observation. Cairn MUST NOT infer tool failures from
  `PostToolUse`. `Stop` fires when the main agent finishes responding and MUST be treated as
  a turn checkpoint that leaves the session `active` (FR-032); `SessionEnd` is the session
  lifecycle boundary that completes it. Hooks run under two deadline classes: capture hooks
  — `PostToolUse`, `PostToolUseFailure`, `Stop`, and the fire-and-forget portions of
  `PreCompact` and `SessionEnd` — MUST use a short deadline (default 250 ms) and drop work
  that exceeds it; the context-delivery path `SessionStart` MUST use a larger bounded
  deadline (default 1,500 ms) because it may need to start the daemon, open storage, inspect
  Git, and assemble a briefing.
- **FR-042**: Cairn MUST remain usable by any MCP-compatible agent through its tools
  alone, without lifecycle hooks. In this mode sessions, tasks, memory, context, and
  handoff generation MUST all work; only automatic observation capture is unavailable,
  and Cairn MUST report which mode a repository is operating in.
- **FR-043**: Cairn MUST provide a single command that installs and configures the
  Claude Code integration for a repository, and one that removes it.

### Local operation

- **FR-044**: Cairn MUST provide a local daemon and local storage that support sessions,
  observations, memory, tasks, handoffs, repository state, and search.
- **FR-045**: Cairn MUST function fully offline; no capture, recall, briefing, handoff,
  or search path may require network access.
- **FR-046**: Cairn MUST start its local daemon automatically when an agent session
  begins and MUST tolerate the daemon being restarted mid-session. If the briefing cannot
  be produced within the context deadline, the agent session MUST still start: Cairn
  returns no context or a reduced briefing, reports the reduced-context state, and never
  blocks the agent waiting.
- **FR-047**: Cairn MUST survive process crashes without losing acknowledged writes or
  leaving storage unreadable.

### Privacy

- **FR-048**: Cairn MUST NOT persist full conversations or unbounded raw command output
  by default.
- **FR-049**: Cairn MUST redact values matching common secret patterns before writing
  any observation, memory, or handoff.
- **FR-050**: Cairn MUST allow the developer to exclude paths and commands from capture,
  and MUST honor exclusions before anything is written.
- **FR-051**: Cairn MUST allow memory to be marked local-only and MUST never transmit
  local-only memory.
- **FR-052**: Cairn MUST allow deletion of any observation, memory, session, or handoff,
  with per-entity semantics that never destroy unrelated durable knowledge:
  deleting an **observation** removes its content locally and leaves any provenance
  reference to it resolvable but contentless, marked deleted;
  deleting a **session** clears the session's content and its observations, retaining a
  tombstone so provenance still resolves, and leaves memories and handoffs that session
  produced intact with their origin marked deleted, unless the developer explicitly asks
  for the memories too;
  deleting a **memory** or a **handoff** removes only that record.
  Deletions of records that were already shared MUST propagate as an idempotent deletion
  tombstone on the next successful sync.

### Server and shared memory

- **FR-053**: Sharing MUST be opt-in per project; until a project is explicitly linked,
  nothing about it leaves the machine. Linking MUST either create a new shared project on
  the server or join an existing one identified explicitly by the user.
- **FR-054**: Cairn MUST authenticate the local daemon to the server using a personal
  API token that the user generates after signing in with email and password.
- **FR-055**: The server MUST store only this allowlist: users, projects, project
  members, project repository-link metadata, tasks, shared memories, handoffs, minimal
  session provenance (identifier, agent, user, task, branch, commit, timings, status), and
  synchronization metadata. Provenance for a shared memory or handoff MUST be limited to
  the source session identifier, local observation identifiers, an evidence count, and an
  optional digest. **Raw observations are local. The server MUST NOT accept or store
  observation content, and MUST reject any sync item carrying it** — a memory or handoff
  referencing an observation does not make that observation shareable.
- **FR-056**: Cairn MUST queue local changes for linked projects and deliver them to the
  server such that redelivery is idempotent. Cairn MUST also pull shared records produced by
  other members of a linked project into local storage, read-only, so that local search and
  context include a teammate's memory.
- **FR-057**: The server MUST refuse access to project data from users who are not
  members of that project.
- **FR-058**: Cairn MUST surface sync state — pending changes, last successful sync, and
  permanent failures with the affected item.
- **FR-059**: Cairn MUST NOT introduce a message broker, a distributed cache, CRDTs, or
  distributed locks.
- **FR-064**: A shared project's identity MUST be a server-assigned identifier established
  explicitly by `cairn link`, independent of any filesystem path, so two clones of one
  repository at different paths on different machines can link to the same shared project.
  Normalized remote metadata MAY be offered as a discovery hint that the user confirms, but
  MUST NOT be the sole authority. Each machine keeps its own local project identifier and
  maps it to the shared identifier at the sync boundary.

### Web UI

- **FR-060**: The web UI MUST provide a projects list, a project overview, a tasks view,
  a sessions and handoffs view, memory search and management, and sync status.
- **FR-061**: The web UI MUST allow searching memory with scope and type filters and
  viewing each result's provenance — source session, evidence count, and observation
  identifiers. Observation content is local and MUST NOT be displayed or fetched by the
  UI; the UI states that evidence content is available only on the machine that captured
  it.
- **FR-062**: The web UI MUST allow deleting a memory.
- **FR-063**: The web UI MUST NOT include knowledge-graph visualization or analytics
  dashboards.

### Key Entities

- **Project**: A tracked Git repository under Cairn. Has a local identifier and a local
  repository instance (Git common directory, worktree paths) that never leaves the machine,
  plus an optional server-assigned shared identifier established by linking. Name, members.
  Owns tasks, sessions, and memory.
- **Task**: A named unit of work in a project. Identifier, title, goal, acceptance
  criteria, status (`todo`, `in_progress`, `done`, `blocked`).
- **Session**: One agent working session, uniquely identified by its own Cairn session
  identifier and keyed to the agent session that opened it. User, agent, project, optional
  task, branch, commit, worktree (context, not identity), optional previous session, start
  and end time, status (`active`, `completed`, `interrupted`).
- **Observation**: One structured thing that happened during a session — of type
  `file_read`, `file_changed`, `command_run`, `test_run`, `error`, `decision`,
  `discovery`, or `user_instruction` — with bounded structured fields and the repository
  state at capture.
- **Memory**: A durable piece of knowledge. Type (`fact`, `decision`, `convention`,
  `failure`, `procedure`), scope (`project`, `branch`, `task`, `session`) with its scope
  key, state (`active`, `stale`, `superseded`), content, provenance to its origin session
  and supporting observation identifiers with an evidence count, and a local-only flag. A
  memory survives the deletion of its origin session and of its evidence; only the
  reference becomes contentless.
- **Handoff**: A structured summary produced at a session boundary: goal, progress,
  completed and remaining work, changed files, decisions, failures, tests executed,
  repository state, recommended next step.
- **Repository State**: Repository identity, branch, commit, worktree, and working-tree
  status at a point in time.
- **User**: A person using Cairn, identified for sessions locally and for membership and
  authentication on the server.
- **Project Member**: The link that grants a user access to a project's shared data.
- **Sync Record**: The queued local change and its delivery metadata, sufficient to make
  redelivery idempotent. Carries the shared project identifier rather than the local one,
  and never carries observation content.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer with a Git repository and Claude Code installed can go from
  zero to a running, capturing Cairn session in under 5 minutes using only the documented
  install and connect steps.
- **SC-002**: After a session ends, its handoff names every file the session changed,
  every test it ran, and a next step, with no manual writing by the developer.
- **SC-003**: No briefing ever exceeds its Cairn-estimated-token budget (default
  2,000–4,000 estimated tokens), in 100% of starts, and every truncated briefing names what
  was omitted. The estimator's measured error against a real tokenizer is recorded and
  conservative. Separately, at least 95% of normal starts fit without dropping a
  high-priority section (task goal and criteria, repository state, previous handoff's next
  step and remaining work).
- **SC-004**: Given a fact recorded as memory in an earlier session, a later session can
  retrieve it by lexical search within the top 5 results without embeddings.
- **SC-005**: Memory recall from a session bound to a task returns task-scoped memory
  ahead of branch-scoped, and branch-scoped ahead of project-scoped, in every test case.
- **SC-006**: With the network disabled, every local operation — session start,
  capture, recall, briefing, handoff, search — completes successfully.
- **SC-007**: Across 200 capture-hook invocations using release binaries, Cairn adds no
  more than 10 ms median latency and 25 ms p95 latency per capture hook, no hook exceeds
  its configured 250 ms deadline, and no Cairn failure aborts the agent session.
  End-to-end agent wall-clock overhead is measured and reported where practical, but is
  informational: the same fixed hook cost is a large fraction of a fast synthetic call and
  a negligible fraction of a real one, so a percentage says more about the workload than
  about Cairn.
- **SC-008**: Every stored observation is within the configured payload bound, and no
  stored record contains a value matching a known secret pattern in a seeded test.
- **SC-009**: Replaying a full sync batch a second time produces no duplicate records and
  no change in server state.
- **SC-010**: A project that has not been linked produces zero outbound network requests
  containing its data, and a linked project transmits zero observation content — inspecting
  every sync payload and the server database finds provenance references but no observation
  summary, path, command, or details.
- **SC-011**: A teammate can, in the web UI and without a terminal, find a project, read
  its latest handoff, find a specific memory by search, and delete it.
- **SC-012**: Every memory returned by search or shown in the UI can be traced to the
  session that produced it, including memories created with no supporting observations in
  manual MCP mode.

## Assumptions

- The developer works in Git repositories on a machine where the Git command-line tool
  is available; Cairn shells out to Git rather than reimplementing it.
- Claude Code is the first-class agent integration for this feature; other MCP-compatible
  agents get the tool surface but not automatic lifecycle capture.
- A single user runs the local daemon on their own machine; the local store is not shared
  between users on one host. That daemon may serve several concurrent agent sessions,
  including more than one in the same worktree.
- The central server is self-hosted for this feature; there is no managed multi-tenant
  hosting, billing, or organization hierarchy.
- Project membership is flat: a user is either a member of a project or not. There are no
  roles or per-scope permissions in this feature.
- The token budget for briefings is denominated in Cairn's own documented estimator, not
  the exact tokenizer of any specific model. Cairn guarantees compliance against its
  estimator and reports the estimator's measured error, never exact model-token compliance.
- The supported Claude Code lifecycle events are taken from the official Claude Code hook
  documentation, which is authoritative — `SessionStart`, `PostToolUse`,
  `PostToolUseFailure`, `PreCompact`, `Stop`, `SessionEnd`. Payloads provide `session_id`,
  `transcript_path`, `cwd`, and per-event metadata (`source`, `reason`, `trigger`,
  `tool_name`, `tool_input`, tool result and failure data) — and no agent process identity
  or liveness signal. Cairn's session semantics are built only on what the payloads actually
  carry (research D16); the exact field set is confirmed against the installed Claude Code
  version during integration.
- "Common secret patterns" means a documented, extensible pattern set (API keys, tokens,
  private keys, connection strings), not a guarantee of catching all secrets.
- Sync is one-directional from local to server for the allowlisted data the local machine
  produces, plus read access to shared project data produced by others; there is no offline
  concurrent edit merge problem to solve in this feature.
- Linking two clones of one repository to the same shared project is a deliberate act by
  the user, who supplies or confirms the shared project identifier; Cairn does not guess it
  from paths and does not auto-merge projects.
- English-language content only for search ranking; no language-specific analysis is
  required.

## Out of Scope

Explicitly not part of this feature, and not to be built speculatively:

- Immutable task revision machinery and general continuation graphs beyond
  `previous_session_id`.
- Knowledge graphs, vector databases, embeddings as a required dependency, and semantic
  graph traversal.
- Memory decay, confidence scoring, automatic truth engines, semantic contradiction
  detection, and cross-branch conflict detection.
- Advanced verification engines, actions, leases, routines, signals, sentinels, and
  autonomous multi-agent coordination.
- Organization and workspace hierarchy, roles beyond project membership, S3 or object
  storage, distributed caching, and message brokers.
- Native first-class integrations for coding agents other than Claude Code.
- Knowledge-graph visualization and analytics dashboards in the web UI.

# Historical MVP requirements

Alpha.7 requirement identifiers retained for existing regression citations. Current requirements live in [product/prd.md](../../product/prd.md).

## Requirements

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

## Success Criteria

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

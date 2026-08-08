# Research: Cairn MVP

**Feature**: `001-cairn-mvp` | **Date**: 2026-08-07

Decisions that shape the plan. Each records what was chosen, why, and what was
rejected. Anything not listed here is an ordinary implementation choice.

## D1 — Language and process topology

**Decision**: One Rust workspace produces three binaries: `cairn` (CLI, Claude Code hook
handlers, and the MCP server), `cairnd` (local daemon), and `cairn-server` (central
server). The web UI is a separate Next.js application.

**Why**: The daemon must start fast, run continuously beside the agent, and never be the
reason a tool call is slow — a native binary with no runtime to boot fits that. Hosting
the MCP server and the hook handlers inside the same binary avoids installing a second
runtime purely for integration glue. The central server shares the domain types with the
daemon, which removes a whole class of wire-format drift. The web UI is the one place
where the ecosystem argument goes the other way, so it stays TypeScript.

**Rejected**: A Node/TypeScript daemon (fast to write, but adds a runtime dependency to
every developer machine and a per-hook startup cost on the agent's hot path). A Go daemon
(fine on its own, but then the server either duplicates the domain in a second language
or the workspace gains a second toolchain for no gain).

## D2 — Local storage

**Decision**: SQLite via SQLx, single file per user under Cairn's data directory, WAL
mode, schema owned by versioned migrations.

**Why**: Embedded, offline by construction, crash-safe with WAL, and good enough for the
volumes one developer produces. SQLx gives compile-time-checked queries against the same
types the server uses.

**Rejected**: Embedded key-value stores (no query or search story), a local Postgres
(absurd install burden for a developer tool), plain files (no transactions, no search).

## D3 — Search

**Decision**: SQLite FTS5 over memory content, combined with exact filters, scope
precedence, and recency in the ranking function.

**Why**: FTS5 gives BM25 ranking with zero extra dependencies and no index service to
run. The spec explicitly forbids requiring embeddings, and scope precedence — not
semantic similarity — is what makes Cairn's recall correct.

**Ranking**: results are bucketed by scope (task, then branch, then project, then
session), and within a bucket ordered by a blend of BM25 relevance and recency. Scope
dominates; relevance and recency break ties. This is deliberate: a mediocre match about
*this task* beats an excellent match about an unrelated one.

**Rejected**: Tantivy (a second index to keep consistent with SQLite for no MVP gain),
any vector store (out of scope), naive `LIKE` matching (no ranking).

## D4 — Git awareness

**Decision**: Shell out to the `git` command-line tool for the *local repository
instance* — Git common directory, worktree path, branch, commit, status. The resolved Git
common directory groups worktrees into one local project. It is **not** the shared identity
of a project: see D14.

**Why**: Git's CLI is the definition of correct Git behavior, handles every repository
shape a developer has, and costs a few milliseconds. Reimplementing status and worktree
resolution against a Git library is a large surface for no product value.

**Cost accepted**: Cairn requires `git` on `PATH` and reports clearly when it is absent.
Status is read on session boundaries and briefing assembly, not on every tool call.

**Rejected**: `gitoxide`/`libgit2` bindings (more code, more edge cases, no user-visible
benefit at this stage).

## D5 — Agent integration surface

**Decision**: Two channels. MCP exposes exactly six tools — `cairn_context`,
`cairn_search`, `cairn_remember`, `cairn_session`, `cairn_task`, `cairn_handoff`. Claude
Code lifecycle hooks (`SessionStart`, `PostToolUse`, `PostToolUseFailure`, `PreCompact`,
`Stop`, `SessionEnd`) invoke `cairn hook <event>`, which forwards to the daemon. What each
event means, and which of them is actually a session boundary, is D16.

**Why**: Hooks give automatic capture that a tool-calling agent would never do reliably
on its own; MCP gives the agent deliberate access to context, memory, and handoff. Six
verb-shaped tools keep the agent's tool list small; each takes an action parameter rather
than exploding into one tool per operation.

**Fail-soft rule**: a hook that cannot reach the daemon exits successfully and drops its
work — the observation on the capture path, the briefing on the context path (D15). Cairn
must never be the reason a session breaks.

**Rejected**: One MCP tool per database operation (bloats the agent's tool list and
invites the agent to do bookkeeping instead of work). A hook-only design (other MCP
agents would get nothing). An MCP-only design (capture would depend on the agent
remembering to report, which it will not).

## D6 — What gets captured

**Decision**: Hooks receive tool payloads and convert them to typed observations with
extracted fields — path, command, exit code, test outcome, error identity — plus a
bounded excerpt. Successes arrive on `PostToolUse` and failures on `PostToolUseFailure`,
which carries the failure data — the split is given by the integration, so Cairn does not
infer failure from a success payload (D16). Conversation text and full command output are not persisted. Redaction
runs before the write, not after.

**Why**: The privacy principle is a functional requirement, and structured facts are also
what makes deterministic handoff synthesis possible. Storing transcripts would make both
worse.

**Bound**: a per-observation payload cap (default 4 KB) with summarization above it; the
cap is asserted in tests.

## D7 — Handoff synthesis

**Decision**: Handoffs are computed from recorded state — changed files from
`file_changed` observations reconciled against Git status, tests from `test_run`,
failures from `error`, decisions from `decision` memories and observations, remaining
work from task acceptance criteria not yet satisfied plus open failures. An
agent-supplied narrative is accepted only as a bounded, clearly attributed field.

**Why**: The spec requires handoffs to be trustworthy. A summary the agent writes about
itself is exactly the thing that drifts; the observation record does not.

**Rejected**: Asking the agent to write the handoff and storing it verbatim.

## D8 — Context budgeting

**Decision**: The budget is denominated in **Cairn-estimated tokens** — a documented
character-based estimator that Cairn owns — not in the tokenizer of any particular model.
The briefing is assembled section by section in a fixed priority order; each section is
measured with the estimator *before* it is emitted and the assembler stops at the budget.
Default target 2,000–4,000 estimated tokens. When anything is dropped, the briefing says so
and names what was omitted.

**What is and is not guaranteed**: compliance against Cairn's estimator is deterministic and
100% — a property of the assembly loop, not a statistic. Compliance against a specific
model's tokenizer is *not* claimed, because Cairn does not run that tokenizer. The estimator
is deliberately conservative (it over-counts rather than under-counts) and its error against
a real tokenizer is measured and recorded, so the estimated budget is a safe upper bound in
practice rather than a coincidence. Calling the unit "estimated tokens" everywhere is the
point: a number that pretends to be exact would be a lie the first time a model tokenized
differently.

**Deterministic, not statistical**: the 95% figure in SC-003 measures something different
and more useful — how often a normal start fits *without dropping a high-priority section*.
A briefing that overflowed its budget would be a bug; one that dropped project-scope memory
to fit is working.

**Priority order**: task goal and acceptance criteria → repository state → previous
handoff (remaining work and next step first) → known failures → decisions → task memory →
branch memory → project memory.

**Why**: A bounded, predictable briefing is what makes automatic injection safe. Exact
tokenizer parity with a specific model is not worth a dependency; the approximation is
documented and tested for its error bound.

## D9 — Sync model

**Decision**: A transactional outbox in SQLite. Every syncable local change writes an
outbox row in the same transaction as the change. A background worker drains the outbox
to the server over HTTPS in batches, each item carrying a stable identifier and a
client-assigned idempotency key. The server upserts by that key. Sync is opt-in per
project and never runs for unlinked projects.

**Why**: The outbox is the simplest construction that survives crashes and offline
periods without losing changes or double-applying them, and it needs no broker, no queue
service, and no distributed coordination — all of which the constitution forbids.

**Direction**: local → server for what this machine produced; server → local read access
for shared data produced by others. There is no concurrent-edit merge in this feature,
which is what keeps CRDTs out.

**Allowlist, not blocklist**: the outbox has no observation entity type, so a payload
carrying observation content cannot be built. Evidence travels as identifiers, a count, and
an optional digest — enough to say "three observations support this" and to resolve them on
the machine that captured them, and not enough to reconstruct anything. This is the whole
reason a memory can be shared while the file paths and commands behind it stay local. The
server rejects any item that carries an observation field, so the boundary is enforced on
both sides.

**Project key translation**: outbox rows carry `server_project_id`, never the local project
id (D14). It is the one identifier that is remapped at the boundary; every other id is a
globally unique UUIDv7 that travels unchanged.

**Permanent failures**: an item rejected with a non-retryable status is moved to a failed
state and surfaced with its identity rather than retried forever.

**Rejected**: Dual writes (loses data on crash), a message broker (explicitly out of
scope), CRDTs (no concurrent-edit problem exists here).

## D10 — Server, database, and authentication

**Decision**: Axum on Tokio with Tower middleware, PostgreSQL via SQLx, migrations in
the repository. Users sign in to the web UI with email and password (argon2 password
hashing, server-side session cookie). The daemon authenticates with a personal API token
generated in the UI, stored hashed server-side and kept in the OS keychain or a
0600-permission file locally.

**Why**: One less external dependency than OAuth, works for self-hosting from the first
day, and separates the interactive session from the long-lived machine credential so a
token can be revoked without touching the account.

**Authorization**: every project-scoped request checks membership. Non-members get a
refusal, not an empty result.

**Rejected**: GitHub OAuth (external dependency and app registration for a self-hosted
MVP), a single shared server secret (no real membership, no per-user attribution).

## D11 — Web UI stack

**Decision**: Next.js App Router with TypeScript, Tailwind, shadcn/ui, and TanStack
Query, talking to the Rust server's JSON API.

**Why**: Six product screens with search, filtering, and detail views — this is the
boring, productive choice, and it keeps the server free of any templating concern.

**Rejected**: A Rust-based web stack (slower to build the screens, worse component
ecosystem), a heavyweight SPA build of our own.

## D12 — Concurrency and multiple sessions

**Decision**: A session is identified by its own id, keyed to the agent session that
opened it (`agent_session_key`); the worktree is scope and context, never the uniqueness
key. Any number of sessions may be active concurrently, including two in the same worktree.
The daemon serializes writes through SQLite transactions. Sessions leave `active` only at
the deterministic boundaries in D16 — Cairn does not attempt to detect a live agent.

**Why**: developers run agents in several worktrees *and* run two agents in one checkout.
Keying sessions by worktree would silently make the second agent adopt or clobber the
first's session — the exact failure a memory system must not have. Keying by the agent's
own session identifier makes routing exact and session start idempotent, and it costs one
column plus a unique index.

**Rejected**: leases, locks, or any coordination protocol. Nothing here needs them: two
sessions writing their own rows in one SQLite database is an ordinary transaction, not a
distributed systems problem.

## D14 — Repository identity across machines

**Decision**: Separate the *local repository instance* from the *shared project*. The local
instance is the Git common directory and worktree paths, plus a locally generated project
id; it never leaves the machine. The shared identity is a server-assigned project id,
established explicitly by `cairn link` — `--create` mints one, `--project <id>` joins one.
Normalized remote metadata is offered as a discovery hint the user confirms, never as the
authority. Local ids map to the shared id at the sync boundary and nowhere else.

**Why**: the same repository is `/Users/a/project` on a Mac and `/home/a/project` on Linux.
Any scheme that hashes a path splits one project into two. Remote URL alone is no better: a
fork shares the upstream remote, a mirror duplicates it, and plenty of repositories have no
remote at all. Linking is a decision, so the design makes it one.

**Cost accepted**: one indirection — the outbox stores both ids, and the daemon refuses to
sync a project it has not linked. That refusal is the same mechanism that satisfies
"unlinked means nothing leaves the machine".

**Rejected**: deriving identity from the path or the Git common directory (breaks across
machines), deriving it from the remote alone (breaks on forks, mirrors, and remote-less
repositories), auto-merging projects that look alike (silently joins unrelated work).

## D15 — Hook deadline classes

**Decision**: Two classes, not one number. Capture hooks — `PostToolUse`,
`PostToolUseFailure`, `Stop`, and the fire-and-forget portions of `PreCompact` and
`SessionEnd` — parse, write to the daemon socket, and return, with a 250 ms deadline and a
dropped observation as the failure mode. `SessionStart` gets 1,500 ms because it must
actually answer: possibly starting the daemon, opening SQLite, shelling out to Git, and
assembling a briefing. If it runs out of time, the agent session starts anyway with no or
partial context and a reported reduced-context state.

**Why**: a single 250 ms budget was wrong in both directions. It is far more than a
fire-and-forget socket write needs, and far less than a cold daemon start plus Git
inspection plus briefing assembly can honestly do — so it would have turned the most
valuable moment in the product into a coin flip. Splitting them keeps the hot path
untouched — SC-007 bounds that path absolutely, at 10 ms median and 25 ms p95 across 200
invocations, inside the 250 ms deadline — while giving the
once-per-session path room to succeed.

**Why 1,500 ms**: a cold `cairnd` start plus SQLite open is tens of milliseconds; `git
status` on a large repository is the variable cost, typically well under 200 ms and
occasionally seconds on a cold cache. 1,500 ms covers the normal case with margin and still
bounds the worst case to something a developer will not notice at session start. Both
numbers are configuration; the value is validated against real repositories in Polish and
revised there if the measurement disagrees.

**Rejected**: blocking `SessionStart` until context is ready (Cairn becomes the reason a
session hangs), and giving capture hooks the larger budget (multiplies across every tool
call for no benefit — nothing reads their answer).

## D17 — How capture cost is stated

**Decision**: SC-007 bounds Cairn's capture overhead **absolutely** — median ≤10 ms and p95
≤25 ms per hook across 200 release-binary invocations, inside the 250 ms deadline — rather
than as a percentage of the agent's wall-clock time.

**Why**: the percentage was not a property of Cairn. Measured with and without Cairn over a
200-call session, the same fixed ~4 ms hook was +27% against a 13 ms synthetic tool call and
would be 0.8% against a 500 ms one. The number moved with the workload, not with the code,
so it could neither guide optimisation nor gate a release. An absolute latency budget is
directly attributable: it changes when Cairn changes, and only then.

**Where the cost is**: process startup dominates. A bare `/usr/bin/true` spawn costs ~1–2 ms
on the reference machine, `cairn --version` ~4.8 ms, and a full capture hook ~4.5 ms — so
Cairn's own connect-and-write is within noise of zero. That cost is **not** subtracted: a
developer pays the whole hook, so the whole hook is measured.

**Rejected**: keeping the percentage with a stated agent baseline (the baseline would be an
assumption doing the work the measurement should do); subtracting spawn cost (flatters the
number and hides a real cost); raising the bar to fit (the instruction, correctly, was not
to weaken the criterion — so the criterion was made measurable instead).

**Kept informational**: end-to-end wall-clock overhead with and without Cairn is still
measured and reported, because it is what a developer feels. It is not a pass/fail gate.

## D16 — What the Claude Code integration actually provides

The **official Claude Code hook documentation is the authority** for which events exist. An
earlier draft of this decision inferred the supported event set from the hook configurations
installed on one machine and concluded that `PostToolUseFailure` did not exist. That was
wrong: absence from a sample of third-party plugin configs is not evidence that an event is
unsupported. The event set below is the documented one.

**Events used**: `SessionStart`, `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `Stop`,
`SessionEnd`. Payloads carry `session_id`, `transcript_path`, `cwd`, and per-event metadata
(`source`, `reason`, `trigger`, `tool_name`, `tool_input`, tool result and failure data).

### Success and failure are separate events

`PostToolUse` fires after a tool succeeds; `PostToolUseFailure` fires after one fails and
carries the failure data. Cairn maps them straight across — `PostToolUse` to the success
observation for its tool type, `PostToolUseFailure` to the `error` observation — and does
**not** try to detect failure by inspecting a `PostToolUse` payload. The integration already
made the distinction; re-deriving it would be guessing at something it tells us directly, and
would misclassify every tool whose result shape Cairn does not recognize.

### `Stop` is a turn boundary, not a session boundary

`Stop` fires when the main agent finishes responding. The session is still open — the
developer can send another prompt and the same session continues. Treating `Stop` as session
end would have been a real product bug: sessions marked `completed` while still running,
`previous_session_id` chains fragmenting one session into many, and a "final" handoff written
mid-work.

**Decision**: `SessionEnd` is the only hook that completes a session. `Stop` is recorded as a
**turn checkpoint** — it updates `last_turn_ended_at` and flushes pending capture, leaving
the session `active`. It does **not** produce a durable handoff.

**Why no handoff on `Stop`**: a durable handoff per turn would write dozens of near-identical
records for one session, and buy nothing — everything a handoff derives from is already in
the observation record, so the `session_end` and `recovered` handoffs reconstruct it whenever
it is actually needed. The checkpoint exists only so a recovery handoff can say when the last
completed turn was.

**Durable handoff triggers** are therefore `pre_compact`, `session_end`, and `recovered`.

### There is no liveness signal

No payload carries an agent process id, and nothing reports that a session has died.
`SessionEnd` fires with a `reason` on ordinary exits, but a crash, a `SIGKILL`, or a lost
machine produces no event at all. `transcript_path` exists, but its modification time cannot
distinguish a dead session from an idle one — and treating idle as dead is exactly the
mistake that would mark a developer's live session `interrupted` while they were at lunch.

**Decision**: Cairn does not detect liveness, and does not pretend to. Sessions leave
`active` only at deterministic boundaries:

1. `SessionEnd` arrives → `completed`, with the payload's `reason` recorded.
2. The developer or agent ends it explicitly → `completed` or `interrupted` as asked.
3. The daemon starts → every session still `active` belongs to a previous `daemon_run_id`
   and is reconciled to `interrupted` with a `recovered` handoff.
4. An event arrives later for a session reconciled at (3) → it resumes to `active` under the
   current run. The handoff already written stands as a valid boundary record, which is why
   this costs nothing and satisfies "tolerate the daemon being restarted mid-session"
   (FR-046).

`Stop` is not on that list. Idle time is *reported* by `cairn status` and never used to
reclassify.

**Why this rather than heartbeats**: a heartbeat, a lease, or a PID table would each be new
infrastructure invented to approximate a fact the integration will not tell us — and each
would be wrong in the same cases (a suspended laptop, a long-running build, a developer at
lunch). Rules 1–4 need no new mechanism: a `daemon_run_id` column and the events already
flowing. The cost is that a session interrupted by a crash is not noticed until the daemon
next starts. That is honest, bounded, and produces the same handoff either way.

**Rejected**: parent-process inspection via `getppid()` (the hook is usually spawned through
a shell, so the parent is the shell; and PIDs are reused), `transcript_path` mtime polling
(cannot separate idle from dead), heartbeat/lease infrastructure (invented machinery for an
unavailable fact, and explicitly out of scope), and treating `Stop` as session end (wrong by
the documented semantics).

## D13 — Testing approach

**Decision**: Behavior-level tests against real temporary Git repositories and a real
SQLite file for the local paths; the server is tested against a real PostgreSQL instance.
Each user story has an end-to-end test matching its Independent Test. Bounded outputs
(observation payload cap, briefing budget) and redaction are asserted, not assumed. A
seeded-secret fixture verifies nothing sensitive reaches storage.

**Why**: The constitution requires behavior to be verified through user-observable
behavior, and every one of Cairn's interesting failure modes lives at the boundary
between Git, storage, and the agent — where mocks are useless.

## Open questions carried into implementation

- Exact secret-pattern set: start from a documented list (API keys, bearer tokens,
  private key blocks, connection strings, `.env` assignments) and extend as real cases
  appear; the mechanism, not the list, is what this feature fixes.
- Token approximation constant: measure against representative briefings during US2 and
  record the observed error bound in the quickstart. The approximation must be conservative
  — it may overestimate, never underestimate, so the hard budget in D8 stays hard.
- The 1,500 ms context deadline (D15) is a starting value validated in Polish against real
  repositories, including a large one with a cold filesystem cache.
- The exact Claude Code hook payload field names (D16) are confirmed against the official
  documentation and the installed version while implementing the hook entry point, and the
  contract updated if they differ. The documentation is authoritative over what any local
  configuration happens to register.

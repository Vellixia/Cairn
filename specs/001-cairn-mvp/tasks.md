---

description: "Task list for Cairn MVP (001-cairn-mvp)"
---

# Tasks: Cairn MVP

**Input**: Design documents from `/specs/001-cairn-mvp/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/)

**Tests**: Included. The constitution requires user-observable behavior to be verified,
and [quickstart.md](./quickstart.md) defines the acceptance walkthrough each story's
tests must satisfy. Tests are behavior-level, not per-function.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story the task belongs to

## Path Conventions

Rust workspace at repository root: `crates/{cairn-core,cairn-git,cairn-store,cairnd,cairn,cairn-server}`.
Web UI in `web/`. Workspace end-to-end tests in `tests/`.

---

## Phase 1: Setup

**Purpose**: A workspace that builds.

- [x] T001 Create the Cargo workspace at `Cargo.toml` with the six member crates, pin the toolchain in `rust-toolchain.toml`, and add `rustfmt.toml`
- [x] T002 [P] Add `.gitignore` entries for Rust, Node, and local Cairn data; add `docker-compose.yml` with PostgreSQL for server development
- [x] T003 [P] Add `.github/workflows/ci.yml` running fmt, clippy, and the workspace test suite on macOS and Linux
- [x] T004 Rewrite `README.md` to describe Cairn as persistent project-aware memory for AI coding agents, with the install-and-connect flow from `quickstart.md`

**Checkpoint**: `cargo build --workspace` succeeds on a clean checkout.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The domain, the two I/O adapters, and the daemon/CLI skeleton that every
story builds on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [x] T005 [P] Define domain types and enums in `crates/cairn-core/src/domain.rs` — Project, Task, Session, Observation, Memory, Handoff and their status/type/scope/state enums per `data-model.md`
- [x] T006 [P] Define the IPC and sync wire types (serde, `ok`/`error` envelope, error codes) in `crates/cairn-core/src/wire.rs` per `contracts/agent-integration.md`
- [x] T007 [P] Implement the Git adapter in `crates/cairn-git/src/lib.rs` — **local repository instance** identity from the resolved Git common directory, plus worktree path, branch, commit, working-tree status, and normalized remote as a discovery hint only; clear errors when Git is missing or the directory is not a repository (FR-001, FR-003, FR-005, FR-064, D14)
- [x] T008 [P] Behavior tests for the Git adapter in `crates/cairn-git/src/lib.rs` (behaviour tests inline) against temporary repositories: no commits, detached HEAD, two worktrees of one repository, no remote, a clone at a different path resolving to a different local instance, not a repository (FR-004, FR-064, edge cases)
- [x] T009 Create the SQLite schema and migration runner in `crates/cairn-store/migrations/0001_init.sql` and `crates/cairn-store/src/migrate.rs` — all entities, `projects.server_project_id`, `sessions.agent_session_key` with its unique index, `sessions.daemon_run_id`, `sessions.last_event_at`, `sessions.last_turn_ended_at`, `sessions.end_reason`, handoff trigger enum `pre_compact|session_end|recovered`, zero-or-more `memory_evidence`, indexes, WAL, foreign keys, schema-version guard (data-model.md, FR-010, FR-019, FR-032, FR-064, D16)
- [x] T010 Implement store repositories (projects, tasks, sessions, observations, memories, handoffs) in `crates/cairn-store/src/repo.rs` with UUIDv7 identity and soft-delete Local writes go through one contention-aware mechanism (`crates/cairn-store/src/tx.rs`): `BEGIN IMMEDIATE` so no read snapshot is ever upgraded, plus bounded retry of `SQLITE_BUSY`/`SQLITE_LOCKED` only, proven by `tests/tests/storage_contention.rs` on macOS and Linux (FR-044, FR-047)
- [x] T011 Implement the daemon skeleton in `crates/cairnd/src/main.rs` — local socket IPC server, request routing, `tracing` setup, graceful shutdown, single-instance lock
- [x] T012 Implement the CLI skeleton in `crates/cairn/src/main.rs` — `clap` command tree, the `--json` envelope, exit codes 0/1/2, daemon auto-start (FR-046)
- [x] T013 Implement `cairn init` and `cairn status` end to end across CLI → daemon → store → Git (FR-002)
- [x] T014 Workspace test in `tests/tests/foundation.rs`: `cairn init` in a fresh repository is idempotent, `cairn status` reports branch/commit/working-tree state, and a non-repository directory fails cleanly with no partial state

**Checkpoint**: `cairn init` and `cairn status` work against a real repository.

---

## Phase 3: User Story 1 — Session work is captured and handed off (P1) 🎯 First usable slice

**Goal**: Sessions start automatically, work is captured as structured observations, and
session boundaries produce a derived handoff.

**Independent Test**: Run a short Claude Code session that edits a file and runs a failing
test; `cairn handoff show` names the file, the test, and a next step.

- [x] T015 [US1] Implement session lifecycle in `crates/cairn-store/src/repo.rs` and `crates/cairnd/src/handlers.rs` — create on start with repository state; identity is the session id keyed by `agent_session_key`, so start is idempotent per agent session and any number of sessions may be active concurrently, including two in one worktree; route every lifecycle event to its own session; `completed`/`interrupted` transitions, `ended_at` (FR-006, FR-007, FR-010, D12)
- [x] T016 [US1] Resolve `previous_session_id` deterministically as the most recently ended non-active session for the same task, else the same branch, with `id` breaking ties (FR-008)
- [x] T017 [US1] Implement deterministic session-boundary reconciliation in `crates/cairnd/src/recover.rs` — on daemon start every still-`active` session (necessarily from a previous `daemon_run_id`) becomes `interrupted` with a handoff, and resumes to `active` if a later event arrives for it; no process tracking, heartbeat, or lease (FR-009, D16)
- [x] T018 [P] [US1] Implement redaction in `crates/cairn-core/src/redact.rs` with the documented secret-pattern set, applied to any string before storage (FR-049)
- [x] T019 [P] [US1] Implement payload bounding and summarization in `crates/cairn-core/src/bound.rs` with the configurable cap (default 4 KB) and a `truncated` flag (FR-013)
- [x] T020 [US1] Implement the capture pipeline in `crates/cairnd/src/capture.rs` — exclusion filter, redaction, structured field extraction, bound, write with repository state at capture, in that order; `PostToolUse` produces the typed success observation and `PostToolUseFailure` produces the `error` observation from its failure data, never inferred from a success payload (FR-011, FR-012, FR-014, FR-041, D16)
- [x] T021 [US1] Implement `cairn hook <event>` in `crates/cairn/src/hook.rs` — stdin payload parsing (`session_id`, `transcript_path`, `cwd`, `source`, `reason`, `trigger`, `tool_name`, `tool_input`, tool result and failure data), mapping all six Claude Code events to daemon calls with `Stop` handled as a turn checkpoint that leaves the session active, two configurable deadline classes (capture 250 ms fire-and-forget; context 1,500 ms for `SessionStart`), always exit 0; confirm the payload field names against the official documentation and the installed version, updating `contracts/agent-integration.md` if they differ (FR-015, FR-032, FR-041, D15, D16)
- [x] T022 [US1] Implement `cairn connect claude-code` and `cairn disconnect claude-code` writing and removing MCP + hook configuration for the repository, registering all six events in FR-041 (FR-041, FR-043, D16)
- [x] T023 [US1] Implement handoff synthesis in `crates/cairn-core/src/handoff.rs` — derive changed files, tests executed, failures, decisions, completed and remaining work, repository state, and next step from observations and Git status, recording evidence as observation identifiers and a count rather than content (FR-032, FR-033, FR-034, FR-055, D7)
- [x] T024 [US1] Wire durable handoff generation to the `pre_compact`, `session_end`, and `recovered` triggers, keeping the session active after `pre_compact`; wire `Stop` to the turn checkpoint instead — flush pending capture, set `last_turn_ended_at`, leave the session `active`, produce no handoff (FR-009, FR-032, D16)
- [x] T025 [US1] Implement `cairn session list|start|show|end [--session <id>]` with `ambiguous_session` when a worktree has several active sessions and idle time reported per session (reported only, never reclassifying), and `cairn handoff show`, both including `--json` (FR-009, FR-010, FR-035)
- [x] T026 [US1] End-to-end test in `tests/tests/us1_capture_handoff.rs`: simulated hook sequence over a real repository — including a `PostToolUseFailure` event — produces typed success and `error` observations, no transcript, bounded payloads, and a `session_end` handoff naming the changed file, the failing test, and a next step (SC-002, FR-041)
- [x] T027 [US1] Test in `tests/tests/us1_sessions.rs`: `Stop` leaves the session `active`, records `last_turn_ended_at`, and produces no handoff, and a second turn continues the same session rather than starting a new one; a session still active at daemon start reconciles to `interrupted` with a `recovered` handoff and resumes to `active` when a later event arrives, keeping that handoff; `SessionEnd` completes the session and records its `reason`; idle time is reported but never reclassifies; a daemon restart mid-session loses no acknowledged writes; a locked or unwritable store makes hooks drop work and exit 0 rather than failing the agent; two concurrent sessions in one worktree and a third in a second worktree stay distinct, route their own observations, and do not end one another (FR-009, FR-010, FR-015, FR-047, D16)

**Checkpoint**: A real Claude Code session produces a readable, accurate handoff. Usable and demoable — not the finished MVP.

---

## Phase 4: User Story 2 — The next session starts already informed (P1)

**Goal**: Session start delivers a bounded briefing assembled from repository state,
prior handoff, and memory.

**Independent Test**: With a prior handoff present, the briefing contains its remaining
work and the current branch and commit, within budget.

- [x] T028 [US2] Implement the Cairn token estimator in `crates/cairn-core/src/budget.rs` — documented, character-based, deliberately conservative so it over-counts rather than under-counts; the budget unit is estimated tokens, and no exact model-tokenizer claim is made (FR-029, D8)
- [x] T029 [US2] Implement briefing assembly in `crates/cairn-core/src/context.rs` — fixed section order, each section measured with the estimator before it is emitted so the estimated-token budget can never be exceeded, degradation from the bottom, `truncated` and `omitted_sections` always populated (FR-027, FR-028, FR-029, FR-030, D8)
- [x] T030 [US2] Handle the empty case: a project with no prior history returns `no_prior_history: true` and succeeds (FR-031)
- [x] T031 [US2] Implement the `cairn_context` MCP tool in `crates/cairn/src/mcp.rs` per `contracts/mcp-tools.md`
- [x] T032 [US2] Implement the MCP server host (`cairn mcp`, stdio transport, tool registration) in `crates/cairn/src/mcp.rs` (FR-040)
- [x] T033 [US2] Inject the briefing at `SessionStart` through the hook path within the context deadline, with the reduced-context fallback (`degraded: true`, session starts anyway, state reported) when the daemon is cold or storage is busy; implement `cairn context [--budget N]` (FR-027, FR-046, D15)
- [x] T034 [US2] End-to-end test in `tests/tests/us2_context.rs`: briefing contains prior handoff remaining work, current branch and commit, and working-tree state; **never** exceeds the estimated-token budget, including against a project with far more memory than fits; ≥95% of normal starts keep every high-priority section; reports omissions honestly; branch switch changes prioritization; a stopped daemon still starts the session with `degraded: true` (SC-003, FR-046)

**Checkpoint**: A second session opens knowing what the first one did.

---

## Phase 5: User Story 3 — Durable, scoped memory and recall (P2)

**Goal**: Knowledge outlives sessions, carries scope and provenance, and is retrievable
by lexical search ranked scope-first.

**Independent Test**: Memories at three scopes are returned task → branch → project from a
task-bound session, each naming its origin session.

- [x] T035 [US3] Add the FTS5 virtual table and sync triggers over memory content in `crates/cairn-store/migrations/0002_memory_fts.sql`
- [x] T036 [US3] Implement memory persistence in `crates/cairn-store/src/repo.rs` — type, scope + scope key, state, `local_only`, a mandatory `origin_session_id`, and a `memory_evidence` join that is zero-or-more, carries `content_digest`, and survives observation deletion; never synthesize evidence to satisfy the schema (FR-016, FR-017, FR-018, FR-019, FR-052)
- [x] T037 [US3] Implement supersede and stale marking — original retained and linked, stale reconciliation for scope keys that no longer resolve (FR-020, US3 scenarios 4–5)
- [x] T038 [US3] Implement retrieval in `crates/cairn-store/src/search.rs` — exact filters, BM25 from FTS5, scope bucketing, recency tiebreak, provenance in every result as session id, observation ids, and evidence count, with deleted evidence resolving as `evidence deleted` (FR-022, FR-023, FR-024, FR-026, FR-052, D3)
- [x] T039 [P] [US3] Implement the `cairn_remember` MCP tool (`create`, `supersede`, `forget`) per `contracts/mcp-tools.md` (FR-021)
- [x] T040 [P] [US3] Implement the `cairn_search` MCP tool per `contracts/mcp-tools.md`
- [x] T041 [US3] Implement `cairn memory add|search|show|forget` with `--json`
- [x] T042 [US3] End-to-end test in `tests/tests/us3_memory.rs`: scope precedence ordering, filters, default-`active` behavior, supersede retention, provenance on every result including memories created with `evidence_count: 0` in manual mode, and full operation with the network disabled (SC-004, SC-005, SC-006, SC-012, FR-019)

**Checkpoint**: A fact learned last week is recalled today, correctly scoped.

---

## Phase 6: User Story 4 — Work is organized by task (P2)

**Goal**: Tasks carry goal and acceptance criteria; sessions bind to them; context and
memory sharpen accordingly.

**Independent Test**: A second session on the same task opens with that task's goal,
criteria, and prior handoff.

- [x] T043 [US4] Implement task persistence and status transitions in `crates/cairn-store/src/repo.rs` (FR-036, FR-037, FR-039 — no revision history)
- [x] T044 [US4] Implement session-to-task binding, including binding at session start and sessions with no task (FR-038, US4 scenario 5)
- [x] T045 [US4] Extend briefing assembly to lead with task goal and acceptance criteria and to prioritize task-scoped memory (US4 scenario 3)
- [x] T046 [P] [US4] Implement the `cairn_task` MCP tool per `contracts/mcp-tools.md`
- [x] T047 [US4] Implement `cairn task list|show|new|set-status` and the `cairn_session` MCP tool (`current`, `start`, `bind_task`, `end`), keyed by `agent_session_key` so start is idempotent per agent session and `current` reports ambiguity rather than guessing (FR-010)
- [x] T048 [US4] End-to-end test in `tests/tests/us4_tasks.rs`: task creation with criteria, binding, status changes visible to later sessions, task-led briefing, and unbound sessions remaining valid

**Checkpoint**: Work is scoped to what the developer is actually doing.

---

## Phase 7: User Story 5 — The developer controls what Cairn stores (P2)

**Goal**: Defaults are private, capture is bounded and filterable, and anything stored can
be removed.

**Independent Test**: An excluded path produces no observation, a seeded API key never
reaches storage, and deleting a session clears the session and its observation content while
the memories and handoff it produced survive with origin marked deleted.

- [x] T049 [US5] Implement configuration for exclusions, payload cap, and context budget in `crates/cairn-core/src/config.rs` with a documented file location and defaults
- [x] T050 [US5] Implement path and command exclusion matching applied before any write, and `cairn privacy exclude|list|unexclude` (FR-050)
- [x] T051 [US5] Enforce the no-transcript, no-raw-output default across every capture path and assert the payload bound at the write boundary (FR-048)
- [x] T052 [US5] Implement `local_only` on memory and guarantee it produces no outbox row (FR-051)
- [x] T053 [US5] Implement `cairn delete observation|memory|session|handoff <id> [--with-memories]` with the per-entity semantics in `data-model.md` — tombstones that clear content but keep identity, deleting a session never removes the memories it produced unless `--with-memories` is given, deleting a memory or handoff touches nothing else, and shared records queue an idempotent deletion (FR-052)
- [x] T054 [US5] End-to-end test in `tests/tests/us5_privacy.rs` with a seeded-secret fixture: excluded path absent, secret redacted everywhere in the database file, every payload within the cap, `local_only` never queued; and each deletion checked separately against one session that produced both a memory and a handoff — deleting an observation leaves both intact with its reference resolving as deleted, deleting the session leaves **both the memory and the handoff readable** with origin marked deleted, `--with-memories` removes the memories, and memory and handoff deletes each remove exactly one record (SC-008, FR-052)

**Checkpoint**: A developer can bound and undo everything Cairn keeps.

---

## Phase 8: User Story 6 — Project memory shared with teammates (P3)

**Goal**: Opt-in per project, idempotent outbox sync, membership-gated server.

**Independent Test**: Two members of one project share memory; a replayed batch changes
nothing; an unlinked project emits nothing.

- [x] T055 [US6] Create the PostgreSQL schema and migrations in `crates/cairn-server/migrations/` — users, projects, project members, project repository-link metadata, tasks, minimal session provenance, memories, handoffs, API tokens, sync state. **No observations table of any kind**; evidence lives on memories and handoffs as identifiers, a count, and an optional digest (FR-055)
- [x] T056 [US6] Implement the Axum server skeleton in `crates/cairn-server/src/main.rs` — Tower middleware, JSON error envelope, health, migrations on start
- [x] T057 [US6] Implement authentication — registration, email/password login with argon2, session cookie, `GET /api/auth/me` (FR-054, D10)
- [x] T058 [US6] Implement API tokens — create (plaintext shown once), list, revoke, and bearer authentication for the daemon (FR-054)
- [x] T059 [US6] Implement the membership guard so non-members receive `403` on every project-scoped route, and the linking routes — create a shared project, join one by identifier, and remote-based lookup returning candidates for the user to confirm (FR-057, FR-064, `contracts/server-api.md`)
- [x] T060 [US6] Implement `POST /api/sync/batch` with per-item idempotency claimed atomically (`INSERT … ON CONFLICT DO NOTHING` in the same transaction as the change, so concurrent deliveries of one key yield exactly one `applied` and `duplicate` thereafter), independent application, `applied`/`duplicate`/`rejected` results, and tombstone propagation; reject any item carrying observation content or a rejected session field, and reject batches for unknown or non-member projects, while a server-side fault is omitted from `results` for retry rather than reported as a permanent rejection (FR-052, FR-055, FR-056, `contracts/server-api.md`)
- [x] T061 [US6] Implement `GET /api/sync/changes` for reading shared records produced by other members
- [x] T062 [US6] Implement the transactional outbox in `crates/cairn-store/src/outbox.rs` — rows written in the same transaction as the change, never created for unlinked projects or `local_only` memory, no observation entity type so observation content cannot be enqueued, and local project ids translated to `server_project_id` at the boundary (FR-053, FR-055, FR-064, D9)
- [x] T063 [US6] Implement the sync worker in `crates/cairnd/src/sync.rs` — claim outbox rows into `in_flight` before sending so the background worker and `cairn sync now` never drain the same row twice, reclaim a claim left stale by an interrupted send (and release every standing claim at daemon start), drain in batches, retry with backoff, release the claim on a transient failure, `failed` state on permanent rejection, resume when the server becomes reachable; and pull shared records from `GET /api/sync/changes` into local read-only storage so search and context include a teammate's memory (FR-056, FR-058, FR-059)
- [x] T064 [US6] Implement `cairn link [--create] [--project <id>]|unlink` against the server's create/join/lookup routes, storing the returned `server_project_id`; remote-based candidates are offered for confirmation, never applied silently. Implement `cairn auth login|token set|logout` with keychain or 0600-file token storage, and `cairn sync status|now` (FR-053, FR-064)
- [x] T065 [US6] End-to-end test in `tests/tests/us6_sync.rs` against a real PostgreSQL: link and first sync, replayed batch is a no-op, offline accumulation then drain with no duplicates, concurrent manual drains racing the background worker over a backlog with every record arriving once and nothing failing, concurrent deliveries of one idempotency key returning one `applied` and duplicates thereafter, a claim abandoned mid-send being taken back and delivered, non-member `403`, unlinked project produces zero outbox rows, two clones at different paths linking to one shared project, a second member's shared memory becoming locally searchable after a pull, and a server database containing provenance references but zero observation content (SC-009, SC-010, FR-055, FR-056, FR-064) Fixture setup is checked throughout (`Sandbox::must`, seeded-state assertion), so a failed seed fails where it happens rather than as a later sync assertion

**Checkpoint**: A teammate can see and search this project's memory.

---

## Phase 9: User Story 7 — Seeing and managing memory in a browser (P3)

**Goal**: Six product screens that let a teammate inspect and curate what Cairn knows.

**Independent Test**: Without a terminal — find the project, read a handoff, search memory,
delete a memory.

- [x] T066 [US7] Scaffold `web/` — Next.js App Router, TypeScript, Tailwind, shadcn/ui, TanStack Query, typed API client, and sign-in against the server session cookie
- [x] T067 [US7] Implement the read API routes the UI needs in `crates/cairn-server/src/api.rs` — projects, project overview, tasks, sessions, session handoff, memory search, memory detail, memory delete, sync status (`contracts/server-api.md`)
- [x] T068 [P] [US7] Build the projects list and project overview screens (repository, active branches, open tasks, recent sessions)
- [x] T069 [P] [US7] Build the tasks screen with status filtering
- [x] T070 [P] [US7] Build the sessions and handoff screens showing the full handoff structure
- [x] T071 [US7] Build memory search and management — query, scope and type filters, provenance on each result shown as origin session, evidence count, and observation identifiers with a clear statement that evidence content is local to the capturing machine, and delete (FR-060, FR-061, FR-062)
- [x] T072 [US7] Build the sync status screen — pending changes, last successful sync, permanent failures with the affected item (FR-058)
- [x] T073 [US7] Playwright acceptance test in `web/e2e/us7.spec.ts` covering the four Independent Test actions end to end (SC-011)

**Checkpoint**: Shared memory is inspectable and correctable by a human. With Polish complete, Feature 001 — the Cairn MVP — is done.

---

## Phase 10: Polish

- [x] T074 Measure capture-hook latency over 200 release-binary invocations in `tests/tests/polish_performance.rs`; assert median ≤10 ms, p95 ≤25 ms and every invocation inside the 250 ms deadline, repeat the run to detect instability, and record the numbers in `quickstart.md`. Process-startup cost is included, not subtracted. End-to-end agent wall-clock overhead is reported alongside, informationally (SC-007, D17)
- [x] T075 Measure the estimator's error against a real tokenizer, confirm it is conservative, and record it in `quickstart.md` (FR-029, D8)
- [x] T076 [P] Write install and connect documentation, verify a cold start reaches a capturing session in under 5 minutes (SC-001)
- [x] T077 [P] Audit the MCP tool list against FR-040 (exactly six), the sync allowlist against FR-055 (no observation content on the wire or in PostgreSQL), and the out-of-scope list against the spec; remove anything that crept in
- [x] T078 Run the full `quickstart.md` walkthrough on macOS and Linux and fix what it surfaces
- [x] T079 Verify manual MCP mode in `tests/tests/manual_mcp_mode.rs` — with hooks removed, an MCP-only agent can start and end sessions, manage tasks, record and search memory, get context, and generate a handoff; `cairn status` reports the reduced-capture mode (FR-042)

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (Phase 1)**: no dependencies
- **Foundational (Phase 2)**: depends on Setup — blocks every user story
- **US1 (Phase 3)**: depends on Foundational
- **US2 (Phase 4)**: depends on US1 for handoff content, though briefing assembly can be
  built and tested against seeded state in parallel with T023–T025
- **US3 (Phase 5)**: depends on Foundational; independent of US1/US2 except that briefing
  memory sections light up once both exist
- **US4 (Phase 6)**: depends on Foundational; sharpens US2 and US3
- **US5 (Phase 7)**: depends on US1's capture pipeline
- **US6 (Phase 8)**: depends on US3 (memory), US4 (tasks), US1 (sessions and handoffs)
- **US7 (Phase 9)**: depends on US6
- **Polish (Phase 10)**: depends on everything intended to ship

### Within a story

- Tests are written against the story's Independent Test and must fail before the
  implementation lands
- Domain and store changes before daemon logic; daemon logic before CLI and MCP surface
- Story complete and demonstrable before moving to the next priority

### Parallel opportunities

- T002–T003 in Setup
- T005–T008 in Foundational (distinct crates)
- T018–T019 in US1 (independent pure-logic modules)
- T039–T040 in US3, T046 in US4 (distinct MCP tool modules)
- T068–T070 in US7 (distinct screens)
- Once Foundational lands, US3 and US4 can proceed alongside US1/US2 with separate people

## Implementation Strategy

**Feature 001 is the Cairn MVP.** It is complete when capture, context, memory, tasks,
privacy, optional shared-server sync, and the web UI are all delivered — every phase in
this file. The checkpoints below are demoable states along the way, not places where the
feature is finished.

**Build order**: Setup → Foundational → US1. At that point Cairn already does something no
agent does today: it writes an accurate, structured account of what a session did. Validate
it, demo it, then keep going — US2 makes the record useful to the next session, US3 makes
knowledge durable, US4 makes it precise, US5 makes it safe to leave running, US6 and US7
make it a team tool.

**Stopping early is a scope decision, not a completed MVP.** Each checkpoint leaves Cairn
in a state a developer can use, so work can pause without breakage — but Feature 001 stays
open until Polish passes and the full `quickstart.md` walkthrough runs end to end.

## Notes

- 79 tasks across 10 phases; every phase ends with something runnable, and the MVP is all of them
- Every task names concrete paths from `plan.md`'s structure decision
- FR and SC references trace each task back to the spec
- Out-of-scope items from `spec.md` are not tasks and must not appear as "while we're
  here" work

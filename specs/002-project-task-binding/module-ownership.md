# Module Ownership Map

## Dependency direction

Feature 002 keeps policy in focused crates and leaves applications as adapters:

```text
depender → dependency

cairn-storage-local → cairn-domain
cairn-events        → cairn-storage-local, cairn-domain
cairn-project       → cairn-events, cairn-storage-local, cairn-domain
cairn-session       → cairn-events, cairn-storage-local, cairn-domain
cairn-protocol      → cairn-domain
daemon              → cairn-project, cairn-session, cairn-events,
                      cairn-storage-local, cairn-protocol
CLI                 → cairn-protocol
```

The actual Cargo dependency graph must remain acyclic. The daemon composes
`cairn-project` and `cairn-session`; those two crates do not depend on each other.

## Ownership

| Location | Owns | Must not own |
|---|---|---|
| `crates/cairn-domain` | Typed IDs, `ProjectStatus`, binding mode, goal-contract value types and validation primitives | SQL, IPC routing, CLI formatting |
| `crates/cairn-project` | Project, association, task, immutable-revision, goal canonicalization, archive rules, and command orchestration | Generic framework, daemon transport, direct CLI output |
| `crates/cairn-session` | Existing lifecycle plus bind command and optional bound-start validation/orchestration | Project metadata mutation, duplicate session-start path |
| `crates/cairn-storage-local` | Migration, SQL records, bounded `BEGIN IMMEDIATE` writer, global operation-idempotency registry, uniqueness, projection queries | User-facing messages, process-local-only correctness |
| `crates/cairn-events` | Aggregate envelope/types, story-owned typed payloads, replay dispatcher/handlers, projection comparison | Project/task mutation policy or constructing every story payload in the foundation phase |
| `crates/cairn-protocol` | Typed IPC DTOs, stable errors, JSON Schemas, golden fixtures, compatibility tripwires | Database access, untyped payload escape hatches |
| `apps/daemon` | Handler wiring, validation-to-error mapping, existing JSON-lines router | Business invariants duplicated from crates |
| `apps/cli` | Clap commands, daemon client calls, human/JSON rendering, bounded human-name resolution | SQLite access, silent ambiguous selection |
| `tests/fixtures` or crate test fixtures | Frozen Feature 001 database fixture plus manifest and expected counts/hashes | Raw resume tokens or sensitive repository contents |

## Proposed source layout

```text
crates/cairn-domain/src/
├── project.rs
├── task.rs
├── goal_contract.rs
└── session.rs                  # additive binding mode/types

crates/cairn-project/src/
├── lib.rs
├── error.rs
├── project_service.rs
├── task_service.rs
├── goal_contract.rs
└── tests/

crates/cairn-storage-local/
├── migrations/0002_project_task_binding.sql
├── src/projects.rs
├── src/tasks.rs
├── src/session_bindings.rs
├── src/operation_idempotency.rs
├── src/aggregate_events.rs
└── tests/migration_0002.rs

crates/cairn-events/src/
├── catalog.rs                  # aggregate envelope/types; story payloads added with stories
├── aggregate.rs
└── replay.rs                   # dispatcher before US1; handlers alongside story events

crates/cairn-protocol/src/
├── project.rs
├── task.rs
├── session.rs                  # additive session scope
└── error.rs

apps/daemon/src/handlers/
├── projects.rs
├── tasks.rs
└── sessions.rs                # extend, do not duplicate

apps/cli/src/commands/
├── project.rs
├── task.rs
└── session.rs                 # extend
```

Names may be adjusted during task generation to fit the repository, but ownership and
dependency boundaries are normative.

## Transaction ownership

Storage exposes a narrow closure-based `BEGIN IMMEDIATE` transaction that:

1. resolves the global raw key or reserves its method/fingerprint/result locator;
2. validates persisted relationships and archive state;
3. reserves aggregate sequences;
4. inserts derived-key typed events;
5. updates projections;
6. commits once.

The writer configures `busy_timeout=5,000 ms`, performs zero application retries, maps exhaustion to `STORAGE_BUSY`, and rolls back on all errors or cancellation before returning the connection. A test-only timeout override supports deterministic lock tests. Services supply policy and their story-specific typed event payloads. Process-local locks may optimize contention but cannot establish correctness.

## Cross-module workflows

### Repository association

`cairn-project` validates project status and repository existence, then storage
serializes on repository identity and atomically inserts the association event and
projection. It never uses a path or remote URL for identity.

### Revision creation

`cairn-project` normalizes and validates the contract before opening a transaction. Storage resolves the global operation registry, atomically increments the task counter, inserts the immutable revision, appends a payload containing the complete revision plus complete Task post-state, updates the Task projection, and commits. Rollback after allocation leaves the next number gap-free.

### Existing-session binding

`cairn-session` accepts typed IDs. Storage resolves global idempotency, validates the session's worktree repository, association, task/project ownership, project status, and existing binding, then appends `session.bound` and updates projections atomically. Barrier-driven integration tests pause watcher reconciliation and recovery/reattach to prove lifecycle state remains independent from exactly-one binding.

### Bound session start

The existing session-start service accepts typed scope and first enforces bootstrap eligibility: unbound is rejected with `PROJECT_SCOPE_REQUIRED` once valid project/task-revision scope exists. A bound start inserts `session.started` (always replay-unbound), then `session.bound` (sole bind), session row, and binding row in one commit; none is visible before commit. Watcher readiness and authoritative Git reconciliation stay in the existing service.

### Replay ownership

The typed dispatcher and aggregate envelope land before US1. US1 owns project payload construction and replay, US2 task/revision payloads and replay, US3 binding payload/replay, and US4 mixed bound-start handling. The later replay phase owns mixed-ledger equality, corruption, and unknown-version integration only.

## Review guardrails

- No `sqlx` dependency in the CLI.
- No JSON `Value` for contractually typed Feature 002 payloads or errors.
- No direct application writes to projection tables.
- No process-local mutex as the sole revision or aggregate ordering guarantee.
- No second event ledger, session lifecycle enum, or watcher implementation.
- No network, server, account, PostgreSQL, AI-memory, MCP, or Feature 003 module.

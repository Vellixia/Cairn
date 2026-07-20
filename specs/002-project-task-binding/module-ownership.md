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
| `crates/cairn-storage-local` | Migration, SQL records, `BEGIN IMMEDIATE` writers, uniqueness, idempotency persistence, projection queries | User-facing messages, process-local-only correctness |
| `crates/cairn-events` | Closed event types, typed payloads, aggregate metadata, replay and projection comparison | Project/task mutation policy |
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
├── src/aggregate_events.rs
└── tests/migration_0002.rs

crates/cairn-events/src/
├── catalog.rs                  # additive typed Feature 002 events
├── aggregate.rs
└── replay.rs                   # Feature 001 + Feature 002 replay

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

Storage exposes narrow closure-based `BEGIN IMMEDIATE` transactions capable of:

1. validating persisted relationships and archive state;
2. resolving or allocating the idempotent result;
3. reserving aggregate sequences;
4. inserting typed events;
5. updating projections;
6. committing once.

Services supply policy and typed event payloads. They never issue separate
event/projection commits. Process-local aggregate locks may reduce contention but
cannot be the only serialization mechanism.

## Cross-module workflows

### Repository association

`cairn-project` validates project status and repository existence, then storage
serializes on repository identity and atomically inserts the association event and
projection. It never uses a path or remote URL for identity.

### Revision creation

`cairn-project` normalizes and validates the contract before opening a transaction.
Storage checks idempotency, locks the write boundary, atomically increments the task's
latest revision number, inserts the immutable revision and event, and commits.

### Existing-session binding

`cairn-session` accepts typed IDs, storage validates the session's worktree repository,
association, task/project ownership, project status, and existing binding, then
appends `session.bound` and updates both projections atomically.

### Bound session start

The existing session-start service accepts an optional typed binding scope and invokes
the same relationship validation and binding write path inside its creation
transaction. Watcher readiness and Git reconciliation stay in the existing service.

## Review guardrails

- No `sqlx` dependency in the CLI.
- No JSON `Value` for contractually typed Feature 002 payloads or errors.
- No direct application writes to projection tables.
- No process-local mutex as the sole revision or aggregate ordering guarantee.
- No second event ledger, session lifecycle enum, or watcher implementation.
- No network, server, account, PostgreSQL, AI-memory, MCP, or Feature 003 module.

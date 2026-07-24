# Implementation Plan: Project and Task Binding Foundation

**Branch**: `feature/002-project-task-binding` | **Date**: 2026-07-20 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/002-project-task-binding/spec.md`

## Summary

Extend Cairn's converged local foundation with local projects, exclusive repository
associations, stable tasks, immutable goal-contract revisions, and one-time session
bindings. The implementation remains a Rust modular monolith: pure invariants and
canonicalization in `cairn-domain`, focused project/task policy in a small
`cairn-project` crate, session binding inside the existing `cairn-session` lifecycle
boundary, SQLx/SQLite projections and migrations in `cairn-storage-local`, typed
events/replay in `cairn-events`, additive JSON-lines contracts in `cairn-protocol`,
and thin daemon/CLI adapters.

Feature 002 adds migration `0002_project_task_binding.sql` to the exact Feature 001
schema. Existing event rows are never updated: legacy rows keep their current nullable
repository/worktree/session columns, while every event appended after migration carries
an explicit aggregate type, aggregate ID, and per-aggregate sequence. The existing
`events.seq INTEGER PRIMARY KEY AUTOINCREMENT` remains the global replay order. A
new immutable `operation_idempotency` registry owns raw request keys across methods.
Registry reservation, project/task/binding projections, aggregate heads, and their events
commit in one SQLite `BEGIN IMMEDIATE` transaction; unique constraints are the
cross-process correctness backstop and process-local aggregate mutexes are only an
optimization.

## Technical Context

**Language/Version**: Rust stable, edition 2021, workspace MSRV 1.85

**Primary Dependencies**: Tokio 1; SQLx 0.8 with SQLite, migrations, and macros; Serde
1 + serde_json 1; schemars 0.8; Clap 4; uuid 1 with UUIDv7; BLAKE3 1; tracing 0.1.
Existing Feature 001 dependencies for Git inspection, notify watchers, local IPC, token
hashing, and platform security remain unchanged.

**Storage**: One per-user SQLite database in WAL mode with foreign keys enabled,
`synchronous=FULL`, a five-second connection busy timeout, zero application-level busy
retries, and a five-second maximum contention wait. Exhaustion returns typed
`STORAGE_BUSY`; cancellation rolls back before a connection returns to the pool. Tests
may override the timeout deterministically. SQLx versioned migrations, append-only event
triggers, immutable snapshots, and transactional projections remain local; no PostgreSQL
or network datastore is introduced.

**Testing**: `cargo test --workspace --all-targets`; focused crate/unit suites;
multi-connection SQLite concurrency tests; real Feature 001 database migration fixture;
event replay equality; daemon/CLI integration; checked-in JSON Schema and golden
tripwires; privacy-log audit; OS-level offline validation; platform-specific Windows,
macOS, and Linux acceptance where IPC or filesystem behavior differs. Quality gates:
`cargo fmt --check` and
`cargo clippy --workspace --all-targets -- -D warnings`.

**Target Platform**: Windows, macOS, and Linux developer machines at ordinary-user
privilege; Unix-domain sockets on macOS/Linux and DACL-restricted named pipes on Windows
remain the local transport.

**Project Type**: Multi-crate Rust workspace with daemon and CLI binaries plus focused
library crates

**Performance Goals**: At the SC-010 fixture size (100 projects, 1,000 tasks, five
revisions per task), at least 95% of create/list/show/update/revise/associate/bind
operations complete within two seconds. Pagination and indexes keep list operations
bounded; canonical goal hashing is linear in the submitted contract size.

**Constraints**: Fully offline; no central server or PostgreSQL; append-only historical
events; immutable task revisions; exact one-time binding; no transfer, deletion,
unbinding, or rebinding; machine selectors use IDs; goal-contract values never enter
diagnostic logs or full error payloads; raw resume tokens remain absent from storage and
logs; Feature 001 watcher readiness and Git-authoritative reconciliation are unchanged.

**Scale/Scope**: One local user and database; hundreds of projects, thousands of tasks,
multiple revisions per task, multiple repositories per project, all worktrees inheriting
repository membership, and an append-only metadata event history. Event and list APIs
remain paginated at bounded limits.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle / gate | Design response | Pre-design |
|---|---|---|
| I. Observable reality authoritative | Repository/worktree ownership uses Feature 001 IDs; events and transaction results, not names/paths, are authoritative. | PASS |
| II. Exact execution scope | `local_unbound` and `project_bound` are explicit record modes. Historical migrated unbound sessions remain valid; new unbound creation is allowed only while valid project/task scope is unavailable, otherwise `PROJECT_SCOPE_REQUIRED`. A bound session references exactly one project and immutable revision through an append-only binding. | PASS |
| III. Append-only history | No Feature 001 event is updated. New events use existing append-only triggers and rebuildable projections. | PASS |
| IV. Evidence before confidence | Migration, replay, contract, concurrency, restart, privacy, and offline claims require executable evidence. | PASS |
| V. Automatic operation | Migration, scope validation, idempotency, replay, and recovery are automatic; no routine human repair path is introduced. | PASS |
| VI. Goal stability | Goal contracts are versioned, canonical, fingerprinted, and immutable per task revision. | PASS |
| VII. Local repository truth | The daemon and Feature 001 repository IDs remain the only source of local repository/worktree truth. | PASS |
| VIII. Deterministic before AI | UUIDv7, BLAKE3, typed canonical JSON, SQLite sequences, constraints, and replay are deterministic; no AI is used. | PASS |
| IX. Minimal reliable infrastructure | One focused crate and additive SQLite tables reuse the modular monolith; no server, broker, cache, or service framework is added. | PASS |
| X. Privacy and secret containment | Contracts persist only user-authored project/task content; logging and errors expose IDs, versions, counts, and fingerprints, never complete contracts or tokens. | PASS |
| Stored-state workflow gate | Versioned transactional migration, exact Feature 001 fixture, restart and compatibility tests are designed. | PASS |
| Protocol workflow gate | Additive v1 DTOs, closed typed errors, checked-in schemas, IPC/CLI goldens, and breaking-change tripwires are designed. | PASS |
| Technical constraints | Rust/Tokio/Serde/SQLx/tracing apply. PostgreSQL, Axum/Tower HTTP, MCP, and server clients are not used because this is an explicitly local-only feature with no HTTP/MCP/server surface. | PASS |

**Gate result before Phase 0**: PASS. No constitutional violation or unresolved
clarification exists.

**Post-Phase-1 re-check (2026-07-20)**: The completed data model, migration design,
event catalog, contracts, compatibility matrix, and quickstart retain the same boundaries.
The design does not fabricate project/task records during migration, does not rewrite
legacy events, does not add synchronization, and preserves Feature 001 session lifecycle,
watcher, snapshot, lease, recovery, IPC-security, and token rules. PASS.

## Project Structure

### Documentation (this feature)

```text
specs/002-project-task-binding/
├── spec.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── migration-design.md
├── event-catalog.md
├── module-ownership.md
├── compatibility-matrix.md
├── testing-evidence.md
├── contracts/
│   ├── ipc-contract.md
│   ├── cli-json-contract.md
│   └── migrations/
│       └── 0002_project_task_binding.sql
└── checklists/
    └── requirements.md

# tasks.md is Phase 2 output from /speckit-tasks and is not created here.
```

### Source Code (repository root)

```text
crates/
├── cairn-domain/
│   └── src/
│       ├── ids.rs                 # add ProjectId, AssociationId, TaskId, TaskRevisionId
│       ├── project.rs             # ProjectStatus and project invariants
│       ├── task.rs                # task/revision types and parent/number invariants
│       ├── goal_contract.rs       # v1 normalization, canonical JSON, BLAKE3
│       └── session.rs             # extend existing session types with binding mode
├── cairn-project/                 # focused policy crate; no transport
│   └── src/
│       ├── project.rs             # create/update/archive/restore/associate
│       ├── task.rs                # create/revise/list/show policy
│       └── lib.rs
├── cairn-session/
│   └── src/
│       ├── service.rs             # extend existing start transaction with optional binding
│       └── binding.rs             # explicit one-time bind policy
├── cairn-storage-local/
│   ├── migrations/
│   │   └── 0002_project_task_binding.sql
│   └── src/
│       ├── projects.rs
│       ├── tasks.rs
│       ├── bindings.rs
│       ├── events.rs              # aggregate fields/sequence allocation
│       ├── records.rs
│       └── writer.rs              # aggregate-key optimization + DB-backed serialization
├── cairn-events/
│   └── src/
│       ├── catalog.rs             # six new typed event payloads
│       └── replay.rs              # project/task/association/binding projections
└── cairn-protocol/
    ├── src/
    │   ├── dto.rs
    │   ├── errors.rs
    │   └── methods.rs
    ├── schemas/
    └── goldens/

apps/
├── daemon/
│   ├── src/handlers/
│   │   ├── project.rs
│   │   ├── task.rs
│   │   └── session.rs             # extend, do not duplicate, readiness path
│   └── tests/
└── cli/
    ├── src/commands/
    │   ├── project.rs
    │   ├── task.rs
    │   └── session.rs
    └── tests/

fixtures/
├── databases/
│   ├── feature-001-v1.sqlite3
│   └── feature-001-v1.manifest.json
└── repositories/
```

**Structure Decision**: Add one focused `cairn-project` policy crate rather than
placing project/task rules in daemon handlers or expanding `cairn-session` into a broad
application service. Pure representations remain in `cairn-domain`; SQL remains in
`cairn-storage-local`; the existing session service alone owns lifecycle and binding.
This preserves the established dependency direction and adds no infrastructure.

See [module-ownership.md](module-ownership.md) for the authoritative ownership and
dependency map.

## Core Design

### Transaction boundary

All Feature 002 mutations use one database write transaction that begins with
`BEGIN IMMEDIATE`. The database write lock, global raw-key registry, unique constraints,
and conditional updates serialize correctness across connections and daemon processes;
an in-memory aggregate mutex only reduces contention. The transaction performs:

1. lookup the raw idempotency key;
2. return the committed result for the same method/fingerprint or reject reuse with
   `IDEMPOTENCY_CONFLICT`;
3. validate current projections, scope availability, and project status;
4. preallocate result identities and reserve the immutable operation record;
5. allocate aggregate sequence(s);
6. append event(s);
7. insert/update projection(s);
8. commit once.


The operation record points to an immutable result event whenever an event was appended, so exact retries reproduce the original post-state and original `created/updated` flag after later projection changes. A distinct-key request that finds the same immutable association/binding already present stores that projection locator and first returns `created:false`; retries of that key remain false.

The connection busy timeout is five seconds with zero application retries. Timeout
exhaustion returns `STORAGE_BUSY`; cancellation and every error roll back the operation
record, event rows, sequence heads, counters, and projections before the connection can
be reused. Tests inject a deterministic shorter timeout. Detailed SQL and restart
behavior are in [migration-design.md](migration-design.md).

### Goal contract

`GoalContractV1` is a typed struct with fixed field order:
`schema_version`, `goal`, `included_scope`, `excluded_scope`,
`acceptance_criteria`, and `constraints`. Every string converts CRLF/CR to LF and
trims only surrounding Unicode whitespace; internal whitespace and list order remain
unchanged. Empty lists are valid; the goal and any supplied list entry must be non-empty
after normalization. Compact UTF-8 JSON from the typed struct is canonical. The
fingerprint is lowercase `BLAKE3(canonical_bytes)` hex, and schema version 1 is inside
the hashed bytes.
Deserialization and validation map missing required fields, malformed structure, empty
goals, empty supplied entries, and unsupported versions to the closed bounded
`INVALID_GOAL_CONTRACT` violation union; errors never echo submitted content.

### Session binding and start

Binding mode is orthogonal to `SessionState`:

```text
SessionState: active | recovering | stopped | interrupted
BindingMode: local_unbound | project_bound(project_id, task_revision_id)
```

Explicit bind runs in the existing session transaction domain:

```text
load session
→ require local_unbound or exact existing binding
→ require active project owns session.repository_id
→ require session.worktree_id belongs to that repository
→ require task revision's task belongs to project
→ append session.bound
→ insert session_bindings projection
→ set sessions.binding_mode = project_bound
→ commit
```

An identical triple returns the existing binding without another event. Any other triple
maps the typed domain error `SessionAlreadyBound` to the spec wire code
`SESSION_BINDING_CONFLICT`; there is no unbind/rebind path.

Bound start extends `SessionService::start`. A new explicit or omitted unbound request
succeeds only while the repository has no active project association or its active
project has no selectable active task revision; otherwise it returns
`PROJECT_SCOPE_REQUIRED` without creating a session. Historical migrated unbound sessions
remain valid. A newly bound session commits the existing `session.started` event first,
then `session.bound`, the session row, and binding projection atomically before the
watcher request. `session.started` always initializes replay scope as `local_unbound`;
`session.bound` is the sole event establishing `project_bound`, and neither event is
externally visible before commit. The handler then uses the unchanged readiness order:

```text
session transaction
→ watcher installation request
→ watcher-ready acknowledgement
→ authoritative post-install Git reconciliation
→ success response
```

Watcher or reconciliation failure still interrupts the created session through the
Feature 001 legal transition and never returns success. A live-session collision succeeds
only when requested and stored binding modes are identical; otherwise it returns
`SESSION_SCOPE_CONFLICT` without conversion. Recovery, leases, token hashes, snapshots,
and watcher reinstallation operate on the same session ID and preserve the binding.

### Event and projection model

The existing `events.seq` is retained as the global total order. Migration adds nullable
`aggregate_type`, `aggregate_id`, and `aggregate_seq`; legacy rows remain byte-for-byte
unchanged in their original columns. All post-migration builders populate the new fields.
`event_aggregate_heads` allocates positive per-aggregate sequences transactionally with
a unique `(aggregate_type, aggregate_id, aggregate_seq)` index.

The replay dispatcher and unchanged Feature 001 handlers are established before US1.
Each story adds its handler alongside its event implementation. `task.revision_created`
carries the complete immutable revision plus complete resulting `Task` post-state,
including `latest_revision_number` and `updated_at`. Later mixed-ledger verification
consumes `ORDER BY seq`, derives legacy scope in memory from real foreign keys, validates
that `session.started` initializes unbound and `session.bound` alone binds, and compares
every stable projection field. See [event-catalog.md](event-catalog.md) and
[data-model.md](data-model.md).

## Implementation Phase Ordering

1. **Compatibility baseline and fixture**: freeze the exact Feature 001 migration/schema,
   create the real v1 database fixture and manifest, record baseline row/event hashes,
   and add regression test scaffolding.
2. **Pure domain foundation**: add UUIDv7 identity types, statuses, binding mode,
   goal-contract validation/canonicalization/fingerprint, and unit tests.
3. **Migration and storage primitives**: add migration 0002, projection rows,
   global operation-idempotency registry, `BEGIN IMMEDIATE` primitive, explicit bounded
   busy/cancellation behavior, aggregate allocation, immutable constraints, and actual-
   runner migration/retry tests.
4. **Replay foundation**: establish the compatibility dispatcher and unchanged Feature
   001 handlers before US1; story-specific handlers arrive with their event types.
5. **US1 project slice**: project create/list/show/update/archive/restore, repository
   association, project event construction and replay, duplicate names, inherited
   membership, global idempotency, contracts, CLI, and tests.
6. **US2 task slice**: task creation plus revision 1, serialized concurrent revise,
   parent validation, canonical fingerprints, replay-complete task post-state events,
   immutable retrieval, contracts, CLI, and tests.
7. **US3 explicit binding slice**: bind existing sessions atomically, identical retry,
   conflicting bind rejection, deterministic watcher/recovery-transition races, session
   binding replay, restart persistence, contracts, CLI, and tests.
8. **US4 bound-start slice**: extend the current start collision logic and DTOs, enforce
   constitutional unbound availability, apply `session.started` then `session.bound`
   semantics, and preserve watcher readiness, recovery, leases, tokens, and lifecycle.
9. **US5 mixed replay/migration slice**: real fixture upgrade, interruption/retry,
   checksum/future-version rejection, strict corruption handling, and field-for-field
   mixed-ledger equality.
10. **Acceptance and evidence**: schema/golden tripwires, privacy, performance, offline
    isolation, platform runs, Feature 001 exactly-100-kill and ignored performance
    acceptances, preliminary evidence commit, analysis/verification/convergence, final
    gate report, and separate final evidence/declaration commit.

Later phases may depend on earlier phases; no phase may begin Feature 003, synchronization,
PostgreSQL, MCP, web UI, or AI features.

## Testing and Evidence Strategy

The authoritative matrix is [testing-evidence.md](testing-evidence.md). Required gates:

- unit tests for every domain invariant and canonicalization vector, including every
  closed goal-contract violation without content leakage;
- independent SQLite pools/connections proving the global operation registry, revision
  allocation, aggregate serialization, first-committer outcomes, and same-key conflicts;
- deterministic busy exhaustion, cancellation rollback, starvation-boundary, and
  counter-allocation rollback tests with a test-only timeout override;
- migration against the committed Feature 001 fixture, plus checksum-mismatch and
  unsupported-newer-version actual-runner failures, restart, and second-open behavior;
- append-only and immutable-row trigger tests;
- failure injection proving operation registry + event + projection all-or-none;
- story-specific replay tests and later mixed-ledger field-for-field equality;
- barrier-controlled binding during paused reconciliation and recovery/reattach;
- Feature 001 session/watch/recovery suites unchanged and passing;
- IPC request/response and CLI-envelope goldens for every success/error path, including
  `PROJECT_SCOPE_REQUIRED`, `STORAGE_BUSY`, and every goal violation;
- checked-in schema equality and compatibility tripwires;
- log/error/DB audits for goal contracts, secrets, ignored contents, environment values,
  and raw resume tokens;
- OS-level Linux no-network execution with local filesystem and IPC proof;
- Windows/macOS/Linux runs where local IPC, migration, or filesystem behavior differs;
- exactly 100 Feature 001 forced daemon kills and explicit Feature 001 ignored performance
  execution on the frozen Feature 002 implementation SHA.

Evidence ordering is normative: freeze one implementation commit; complete exact-SHA
platform, isolation, performance, and Feature 001 compatibility executions; commit
preliminary evidence; run analyze, verify, verify-tasks, and converge; resolve every
appended task; then create the final-gate report and final evidence/declaration commit.
No committed document is required to contain its own commit SHA. The final evidence
commit SHA is recorded by workflow metadata, an annotated tag, or a later external
manifest.

## Compatibility and Rollout

[compatibility-matrix.md](compatibility-matrix.md) records every Feature 001 table,
event, API, lifecycle guarantee, and test suite. The rollout remains additive within
`v1.*`: old session-start requests that omit scope decode as requests for
`local_unbound`, but after valid repository project/task scope exists they receive the
explicit additive `PROJECT_SCOPE_REQUIRED` policy error rather than silently creating
unbound work. Responses add a tagged scope object, existing event filters remain valid,
and new aggregate fields are optional only for legacy stored events. Breaking field
removal or reinterpretation requires `v2` and is not planned.

## Complexity Tracking

No Constitution Check violation requires an exception. The focused `cairn-project`
crate is code organization inside the existing workspace, not new infrastructure; it
prevents transport handlers and the session lifecycle crate from becoming mixed-purpose.

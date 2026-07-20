# Feature Specification: Project and Task Binding Foundation

**Feature Branch**: `feature/002-project-task-binding`

**Created**: 2026-07-19

**Status**: Draft

**Input**: User description: "Build Feature 002: introduce the minimum local project, immutable task-revision, goal-contract, and append-only session-binding model needed to convert explicit Feature 001 `local_unbound` sessions into correctly scoped `project_bound` sessions without rewriting historical execution evidence."

## Clarifications

### Session 2026-07-19

- Q: How are project identity, status, repository membership, and worktrees scoped? → A: IDs are authoritative; duplicate names are allowed; projects are `active` or readable `archived`; restoring is explicit; repository-ID association is exclusive and inherited by all worktrees; removal and transfer are out of scope.
- Q: How are task identity, revisions, and goal contracts represented? → A: Task IDs are permanent within one project and titles may duplicate; revisions use transactionally serialized positive sequential numbers, immutable content, idempotency keys, normal previous-revision parents, versioned canonical goal contracts, preserved list order, limited whitespace normalization, and fingerprints.
- Q: What session binding and revision-continuity transitions are allowed? → A: A `local_unbound` session binds exactly once to one project and immutable revision, preserves its ID and prior history, retries identically, rejects rebind or unbind, and requires another session to continue under a newer revision.
- Q: How must migration and project/task event history behave? → A: Migration classifies every Feature 001 session as `local_unbound` without fabricated records, preserves IDs, events, snapshots, leases, timestamps, and states, is restart-safe and idempotent, and replay uses one deterministically ordered ledger with explicit aggregate scopes and no fake worktree IDs.
- Q: What selection, deletion, privacy, and synchronization boundaries apply? → A: Machine interfaces require IDs; human displays include IDs and ambiguous name selection returns a stable error with candidates; hard deletion and task archival are out of scope; goal contracts may persist locally but never appear whole in diagnostics or errors; central synchronization remains out of scope.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Create a project and associate its repository (Priority: P1)

A developer creates a local project, associates an already-registered Cairn
repository with it, and can later find the project using the stable repository
identity even if the repository directory has moved.

**Why this priority**: Project ownership of a repository is the root scope
needed before tasks or sessions can become project-aware.

**Independent Test**: Register a repository through Feature 001, create a
project, associate the repository, move the repository directory, and verify
that project inspection still reports the same association and stable
identities.

**Acceptance Scenarios**:

1. **Given** no project with the requested identity, **When** a user creates a project with valid metadata, **Then** Cairn creates one project with a stable authoritative identifier, active status, created timestamp, and updated timestamp.
2. **Given** multiple projects with the same name, **When** a user lists or inspects them, **Then** Cairn displays their identifiers so they remain distinguishable without requiring global name uniqueness.
3. **Given** an active project, **When** a user changes its name, description, or status, **Then** Cairn preserves its identifier and created timestamp while recording the new metadata and updated timestamp.
4. **Given** an archived project, **When** it is inspected, **Then** its metadata, repository associations, tasks, revisions, and existing bound sessions remain readable.
5. **Given** an archived project, **When** an explicit status update restores it to active, **Then** new allowed mutations may resume without rewriting existing history.
6. **Given** a registered repository and an active project, **When** the user associates the repository with the project, **Then** the association records the Cairn repository identity rather than its path or remote and every worktree of that repository inherits project membership.
7. **Given** the same repository is already associated with the same project, **When** the association request is repeated, **Then** Cairn returns the existing association and records no duplicate projection or logical event.
8. **Given** the repository is associated with another project, **When** a conflicting association is requested, **Then** Cairn returns `REPOSITORY_PROJECT_CONFLICT`, changes neither project, and preserves the existing association.
9. **Given** an associated repository directory is moved, **When** Cairn resolves its existing repository identity at the new path, **Then** the project association remains intact.
10. **Given** a request to remove or transfer an association, **When** it is submitted, **Then** Cairn does not perform that out-of-scope mutation.

---

### User Story 2 - Create immutable task revisions and goal contracts (Priority: P2)

A developer creates a task inside a project with a structured goal contract.
When the intended work changes, the developer creates a new immutable revision
while earlier revisions remain available exactly as recorded.

**Why this priority**: A session cannot be task-aware until its goal, scope,
acceptance criteria, and constraints have a stable immutable reference.

**Independent Test**: Create a task and revision 1, revise it to revision 2,
then inspect both revisions and verify revision 1 is unchanged while the task
points to revision 2 as its latest intent.

**Acceptance Scenarios**:

1. **Given** an active project and a valid goal contract, **When** a user creates a task, **Then** Cairn atomically creates a stable task identity with immutable revision 1.
2. **Given** tasks with duplicate titles inside one project, **When** they are listed or selected, **Then** their authoritative IDs distinguish them and no ambiguous name is selected silently.
3. **Given** a valid task revision, **When** it is read repeatedly, **Then** its revision identifier, positive sequential number, canonical goal contract, fingerprint, timestamp, and parent reference remain unchanged.
4. **Given** an existing task, **When** the user revises its intent, **Then** Cairn transactionally creates the next numbered immutable revision and leaves every prior revision untouched.
5. **Given** concurrent revision requests, **When** they complete, **Then** accepted revisions have unique sequential numbers; retrying the same idempotency key returns the existing revision.
6. **Given** revision 1 and revision 2 of one task, **When** the task is shown without a revision selector, **Then** Cairn returns the latest revision while an explicit ID selector can retrieve either revision.
7. **Given** the same goal-contract values with equivalent line endings and insignificant surrounding whitespace, **When** Cairn emits their versioned representation, **Then** canonical output and fingerprint are deterministic while list order is preserved.
8. **Given** a non-empty goal and empty included-scope, excluded-scope, acceptance-criteria, or constraint lists, **When** task creation or revision is attempted, **Then** the goal contract remains valid.
9. **Given** an empty or whitespace-only primary goal or malformed goal-contract structure, **When** task creation or revision is attempted, **Then** Cairn returns `INVALID_GOAL_CONTRACT` and creates neither a task nor revision.
10. **Given** an archived project, **When** a new task or task revision is requested, **Then** Cairn rejects the mutation while preserving existing task history for inspection.

---

### User Story 3 - Bind an existing bootstrap session explicitly (Priority: P3)

A developer selects an existing `local_unbound` session and explicitly binds
it to one project and one immutable task revision. The session keeps its
original identity and complete Feature 001 history, and the binding is
recorded as a new append-only fact.

**Why this priority**: Explicit binding is the constitutional bridge from
local bootstrap execution to correctly scoped project and task execution.

**Independent Test**: Create a project and task revision for a repository,
start a `local_unbound` session, record Feature 001 events, bind the session,
and verify its identifier and earlier event records are unchanged while
exactly one `session.bound` event and one binding projection are added.

**Acceptance Scenarios**:

1. **Given** a `local_unbound` session whose worktree belongs to a repository associated with an active project, and a task revision belonging to that project, **When** binding is requested, **Then** the same session becomes `project_bound` to exactly that project and revision.
2. **Given** a successful binding, **When** the session and event history are inspected, **Then** the original session identifier and every earlier session/execution event remain unchanged and a later `session.bound` event records the binding.
3. **Given** an already bound session, **When** the identical binding request is repeated, **Then** Cairn returns the existing binding and appends no duplicate logical binding event.
4. **Given** an already bound session, **When** another project or task revision, unbinding, or rebinding is requested, **Then** Cairn returns `SESSION_BINDING_CONFLICT` and preserves the original binding.
5. **Given** a project that does not own the session worktree's repository, **When** binding is requested, **Then** Cairn returns `REPOSITORY_NOT_ASSOCIATED` and records no partial event or projection.
6. **Given** a task revision belonging to another project, **When** binding is requested, **Then** Cairn returns `TASK_REVISION_PROJECT_MISMATCH` and leaves the session unbound.
7. **Given** a stopped, interrupted, recovering, or active `local_unbound` session, **When** all binding invariants are satisfied, **Then** explicit binding records scope without changing the session lifecycle state or creating a new session.
8. **Given** an archived project, **When** a new binding is requested, **Then** Cairn rejects the mutation and leaves the session unchanged while existing bound sessions remain inspectable.

---

### User Story 4 - Start and recover correctly scoped sessions (Priority: P4)

A caller can start either an explicit `local_unbound` bootstrap session or a
`project_bound` session that references a valid project and immutable task
revision. Bound sessions retain all Feature 001 lifecycle, watcher, snapshot,
lease, uniqueness, and recovery behavior.

**Why this priority**: New work should start with correct scope when project and
task context already exist, while offline bootstrap use must remain available.

**Independent Test**: Start a bound session for an associated repository and
valid task revision, restart the daemon, and verify the same session remains
bound to the same immutable revision while Feature 001 recovery and snapshot
behavior still succeeds.

**Acceptance Scenarios**:

1. **Given** a worktree whose repository is associated with an active project and a revision of a task in that project, **When** a bound session is started, **Then** its successful result is explicitly `project_bound` and contains exactly that project and revision.
2. **Given** no selected project or task revision, **When** bootstrap session creation is explicitly requested, **Then** the result is `local_unbound` and contains no implied project or task scope.
3. **Given** either session scope, **When** human-readable or machine-readable session output is requested, **Then** the scope classification is unambiguous and bound output includes project and task-revision identifiers.
4. **Given** a bound session and a newly created revision of its task, **When** the session is inspected, **Then** it still references its original immutable revision and continuing under the newer revision requires another session.
5. **Given** a bound session that was active before daemon restart, **When** recovery and valid reattachment complete, **Then** the session retains its binding and all Feature 001 recovery guarantees.
6. **Given** a bound-start request whose worktree, repository, project, task, or revision relationship is invalid, **When** start is attempted, **Then** Cairn returns the corresponding stable error and creates no session.
7. **Given** a healthy existing session for the same Feature 001 uniqueness key, **When** a start request asks for a different scope or binding, **Then** Cairn never silently returns or converts that session under the requested scope and instead requires an explicit compatible operation.

---

### User Story 5 - Migrate and replay without losing historical truth (Priority: P5)

A developer upgrades an existing Feature 001 installation. Every existing
repository, worktree, snapshot, session, and event survives; old sessions are
explicitly classified as `local_unbound`; and replay reconstructs project,
task, and binding state exactly.

**Why this priority**: The feature is unacceptable if it gains project scope by
rewriting or losing the local execution evidence Feature 001 established.

**Independent Test**: Upgrade a real Feature 001 database fixture, compare all
pre-upgrade identifiers and event records, create project/task/binding data,
restart, replay the event ledger into empty projections, and compare the
rebuilt state with live state.

**Acceptance Scenarios**:

1. **Given** a valid Feature 001 database, **When** the versioned migration completes, **Then** every prior repository, worktree, snapshot, session, lease, timestamp, lifecycle state, event, sequence, and projection remains available with the same historical identity and content.
2. **Given** pre-existing Feature 001 sessions, **When** migration completes or is safely run again, **Then** each is explicitly classified as `local_unbound` without fabricating a project, task, revision, association, or binding event and the second run changes nothing.
3. **Given** a migration interruption or failure, **When** Cairn restarts, **Then** it either completes from a valid boundary or reports `MIGRATION_FAILED`; it never exposes a partially migrated state as healthy.
4. **Given** project, task, revision, repository-association, and session-binding events with explicit aggregate scopes, **When** projections are rebuilt from the complete ordered append-only ledger, **Then** the rebuilt state equals the live project, task, and binding state.
5. **Given** any failed project, task, association, start, or binding operation, **When** persisted state is inspected, **Then** neither a partial event sequence nor partial projection exists.
6. **Given** concurrent operations affecting the same repository, task, or session, **When** accepted operations complete, **Then** their event order is deterministic, idempotent retries add no duplicate logical event, and projections match ledger replay.
7. **Given** a project-only or task-only operation, **When** its event is appended, **Then** it has an explicit aggregate scope and deterministic ledger position without a fabricated worktree identifier.
8. **Given** a successful migration and bound session, **When** the daemon restarts repeatedly, **Then** the binding, immutable revision reference, and historical events remain stable.
9. **Given** external network access is unavailable, **When** the complete migration and binding demonstration is executed, **Then** all required behavior succeeds using only local filesystem and IPC access.

### End-to-End Success Demonstration

1. Register a repository using Feature 001.
2. Create a project.
3. Associate the registered repository with the project.
4. Create a task with immutable revision 1.
5. Start or select an existing `local_unbound` session.
6. Bind the session to the project and revision 1.
7. Confirm all earlier Feature 001 events remain unchanged.
8. Restart the daemon.
9. Confirm the same session remains `project_bound`.
10. Create task revision 2.
11. Confirm the existing session still references revision 1.
12. Replay events and reproduce the same project, task, repository-association, and session-binding state.

### Edge Cases

- A project name is empty, whitespace-only, or duplicated.
- Multiple projects or tasks share the same human-readable name or title and a human selector is ambiguous.
- Two project-creation requests use the same idempotency key.
- A repository path changes after association while its Cairn identity remains stable.
- A manually copied repository presents a new Feature 001 repository identity.
- Two requests concurrently try to associate one repository with different projects.
- A project is archived while it has bound active or recovering sessions, then explicitly restored.
- A worktree session attempts binding through a project associated with a different repository identity.
- Two task revisions are requested concurrently from the same latest revision or retry the same idempotency key.
- A task revision references a parent from another task, a nonexistent revision, or a non-earlier revision.
- A goal contract has an empty primary goal, empty optional lists, different line endings, or surrounding whitespace.
- A bound-start request collides with a healthy `local_unbound` session for the same agent instance and repository.
- An unbound-start request collides with an existing `project_bound` session.
- A session is bound while a watcher reconciliation or recovery transition is in progress.
- An identical bind retry arrives after the first request committed but before its response reached the caller.
- A bound session's task gains a new revision while the session remains on its original revision.
- Migration encounters a valid but very large Feature 001 event history.
- Migration is interrupted before, during, or after its transactional boundary, or is run again after success.
- Replay encounters an unknown future event type, invalid aggregate scope, invalid binding reference, or project/task event with a fabricated worktree identifier.
- A deletion, repository-transfer, task-move, unbind, rebind, or session-revision-transition request is attempted.
- Machine-readable output is requested for historical `local_unbound` and current `project_bound` sessions in one list.

## Requirements *(mandatory)*

### Functional Requirements

**Project management and repository association**

- **FR-001**: Users MUST be able to create a local project with a stable authoritative project identifier, non-empty display name, optional description, created timestamp, updated timestamp, and status. Project names need not be globally unique.
- **FR-002**: Project status MUST initially be exactly `active` or `archived`. Archived projects and their existing bound sessions MUST remain inspectable but MUST reject new repository associations, tasks, revisions, bindings, and bound-session starts until an explicit update restores the project to `active`.
- **FR-003**: Users MUST be able to list projects, inspect one project, and update its name, optional description, or status without changing its stable identifier or created timestamp.
- **FR-004**: A project MAY contain one or more repositories already registered through Feature 001. Every worktree belonging to an associated repository MUST inherit that project membership.
- **FR-005**: A project-repository association MUST reference only the stable Cairn repository identity as authority, never a path or remote URL, and MUST survive repository path changes.
- **FR-006**: One repository identity MUST have at most one active project association. Cross-project sharing, removal, transfer, and silent reassignment are prohibited in this feature.
- **FR-007**: Repeating an identical project-repository association request MUST return the existing association without duplicate events or projections.
- **FR-008**: A conflicting association MUST return `REPOSITORY_PROJECT_CONFLICT` and MUST NOT change either project or the existing association.

**Tasks, immutable revisions, and goal contracts**

- **FR-009**: Users MUST be able to create a stable task identity with a display title inside one active project. Task titles need not be unique, IDs are authoritative, and a task MUST remain owned permanently by exactly one project.
- **FR-010**: Task creation MUST atomically create immutable revision 1; a task without an initial valid revision MUST NOT become visible.
- **FR-011**: Each immutable revision MUST contain a stable revision ID, task ID, positive sequential revision number scoped to that task, structured goal contract, created timestamp, and optional parent revision ID. Revision creation MUST be serialized transactionally so concurrent requests cannot create duplicate numbers; retrying the same idempotency key MUST return the existing revision.
- **FR-012**: Revision 1 has no parent by default. A later revision normally references the immediately previous revision; any explicit parent MUST reference an existing earlier immutable revision of the same task.
- **FR-013**: Revising task intent MUST create a new immutable revision and MUST NOT modify, replace, move, or delete earlier revisions.
- **FR-014**: A task's default current view MUST identify its latest accepted revision, while explicit revision selection MUST retrieve any retained revision.
- **FR-015**: A goal contract MUST contain a required non-empty primary goal and ordered lists for included scope, excluded scope, acceptance criteria, and non-negotiable constraints. Empty lists are valid.
- **FR-016**: Goal contracts MUST use a versioned deterministic machine-readable representation and stored fingerprint. Canonicalization MUST preserve list order and normalize only line endings and insignificant surrounding whitespace.
- **FR-017**: A missing required field, empty or whitespace-only primary goal, malformed representation, or invalid canonical version MUST return `INVALID_GOAL_CONTRACT` and MUST NOT create a task, revision, event, or projection; empty optional lists alone MUST NOT fail validation.

**Session scope and explicit binding**

- **FR-018**: Every session MUST explicitly identify its scope as exactly one of `local_unbound` or `project_bound`.
- **FR-019**: A `local_unbound` session MUST have no project or task-revision association and MUST NOT be represented as project-aware, task-aware, synchronized project truth, or authoritative project memory.
- **FR-020**: Users MUST be able to bind an existing `local_unbound` session exactly once and explicitly to exactly one active project and exactly one immutable task revision.
- **FR-021**: Binding MUST verify that the session worktree belongs to the repository identity associated with the selected project and that the selected revision belongs to a task in that project.
- **FR-022**: Successful binding MUST preserve the original session identifier, lifecycle state, snapshots, leases, watcher behavior, recovery behavior, timestamps, and all earlier Feature 001 events.
- **FR-023**: Successful binding MUST append exactly one `session.bound` event after existing history and MUST record the project and task-revision identifiers in the binding projection.
- **FR-024**: Repeating an identical successful binding request MUST return the existing binding without appending another logical `session.bound` event.
- **FR-025**: Any request to bind the session to another project or task revision MUST return `SESSION_BINDING_CONFLICT` and preserve the original binding; rebinding and unbinding are prohibited in this feature.
- **FR-026**: Binding MAY apply to any Feature 001 lifecycle state but MUST NOT itself change that lifecycle state or create a replacement session.
- **FR-027**: Users MUST be able to start a new session already `project_bound` when its worktree, repository, active project, task, and revision relationships are valid.
- **FR-028**: Starting a `local_unbound` bootstrap session MUST remain supported through an explicit unbound choice.
- **FR-029**: Human-readable and machine-readable session outputs MUST distinguish `local_unbound` from `project_bound`; bound output MUST include the project and immutable task-revision identifiers.
- **FR-030**: Creating or retrieving a session under Feature 001 uniqueness and idempotency rules MUST NOT silently change or misrepresent its binding; an incompatible scope collision MUST return a stable machine-readable conflict requiring explicit resolution.
- **FR-031**: A bound session MUST continue referencing its selected immutable revision after newer revisions of the same task are created. Continuing under a newer revision MUST start another session in this feature.
- **FR-032**: Bound sessions MUST retain all Feature 001 uniqueness, lease, recovery, watcher-readiness, reconciliation, snapshot, interruption, and event guarantees.

**Migration, append-only events, and replay**

- **FR-033**: The feature MUST introduce a versioned migration of the existing Feature 001 local data store.
- **FR-034**: Migration MUST preserve every existing repository, worktree, snapshot, session, lease, timestamp, lifecycle state, event, event sequence, and projection with identical historical identifiers and contents and without rewriting historical events.
- **FR-035**: Migration MUST explicitly classify every existing Feature 001 session as `local_unbound` without fabricating project, task, revision, association, or binding records or events. Re-running the migration MUST be safe and idempotent.
- **FR-036**: Migration MUST be transactional and restart-safe; failure MUST return `MIGRATION_FAILED` and MUST NOT expose partial migrated state as healthy.
- **FR-037**: Cairn MUST append events for at least `project.created`, `project.updated`, `project.repository_associated`, `task.created`, `task.revision_created`, and `session.bound`.
- **FR-038**: Every new event MUST use the same append-only local ledger and carry an explicit aggregate scope and identifier. Project- and task-only operations MUST use project or task scope without fabricated worktree identifiers. Accepted events MUST have a deterministic ledger position; operations affecting the same aggregate MUST be serialized; and an idempotency-key retry MUST not append a duplicate logical event.
- **FR-039**: Project, task, revision, repository-association, and session-binding projections MUST be rebuildable deterministically from the complete ordered event stream, including events that are not worktree-specific.
- **FR-040**: Operations that require multiple events or projection changes MUST commit atomically; failure MUST leave neither partial event sequences nor partial projections.
- **FR-041**: Daemon restart and valid Feature 001 recovery MUST preserve each session's scope classification and immutable binding.

**Local interfaces and failure contracts**

- **FR-042**: Users MUST have human-readable and machine-readable local commands equivalent to `project create`, `project list`, `project show`, `project update`, and `project repository add`. Machine requests MUST select projects and repositories by ID; human output MUST show project IDs beside names so duplicate names remain distinguishable.
- **FR-043**: Users MUST have human-readable and machine-readable local commands equivalent to `task create`, `task revise`, `task list`, and `task show`, including explicit historical revision selection. Machine requests MUST select tasks and revisions by ID; human output MUST show task IDs beside titles.
- **FR-044**: Users MUST have human-readable and machine-readable local commands equivalent to the extended `session start`, `session bind`, and `session show`, including an explicit scope choice and ID-based machine selection.
- **FR-045**: All user commands MUST use the daemon's local request interface and MUST NOT modify persistent storage directly.
- **FR-046**: Every new request, response, error, and machine-readable command envelope MUST have a typed, closed contract with compatibility and golden-example validation.
- **FR-047**: The stable machine-readable error set MUST include `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`, `TASK_NOT_FOUND`, `TASK_REVISION_NOT_FOUND`, `TASK_REVISION_CONFLICT`, `REPOSITORY_NOT_ASSOCIATED`, `REPOSITORY_PROJECT_CONFLICT`, `TASK_REVISION_PROJECT_MISMATCH`, `SESSION_BINDING_CONFLICT`, `SESSION_SCOPE_CONFLICT`, `AMBIGUOUS_NAME`, `INVALID_GOAL_CONTRACT`, and `MIGRATION_FAILED`. If a human name-based selector is supported and matches multiple records, Cairn MUST return `AMBIGUOUS_NAME` with candidate IDs and MUST NOT select silently.
- **FR-048**: Failed requests MUST return one stable error, MUST NOT emit a success envelope, and MUST NOT leak partial projections, partial event sequences, internal paths, ignored content, secrets, environment values, resume tokens, or complete goal-contract content.

**Offline operation, privacy, and compatibility**

- **FR-049**: All required project, task, migration, binding, replay, restart, and inspection behavior MUST work with external network access disabled while local filesystem and IPC remain available.
- **FR-050**: The feature MUST NOT require or perform central-server synchronization, network accounts, cloud authentication, or external services; project-aware synchronization remains prohibited until a later feature defines its protocol and lifecycle.
- **FR-051**: Goal contracts are user-authored project content and MAY be persisted locally, but complete goal contracts MUST NOT appear in diagnostic logs or error responses. Persisted data, events, logs, and command output MUST NOT contain ignored-file contents, secrets, environment-variable values, raw resume tokens, or unnecessary repository content.
- **FR-052**: Every Feature 001 contract and behavior MUST remain compatible unless this specification explicitly extends it.
- **FR-053**: Platform-specific local IPC, migration, restart, and persistence behavior MUST be validated on Windows, macOS, and Linux where behavior differs.

### Key Entities

- **Project**: Stable local scope identified authoritatively by ID, with a non-unique display name, optional description, `active` or `archived` status, created timestamp, and updated timestamp.
- **Project Repository Association**: Non-transferable link between one project and one stable Feature 001 repository identity; independent of mutable paths and remotes and inherited by every worktree of that repository.
- **Task**: Stable logical identity with a non-unique display title, owned permanently by exactly one project; editable intent exists only through immutable revisions.
- **Task Revision**: Immutable positive sequential version of one task's intent, with an idempotency identity, optional same-task parent revision, structured goal contract, and creation timestamp.
- **Goal Contract**: Versioned deterministic representation containing a required non-empty primary goal and ordered included-scope, excluded-scope, acceptance-criteria, and constraint lists, plus a fingerprint of the canonical content.
- **Event Aggregate Scope**: Explicit event scope and aggregate identifier for project, task, repository, worktree, or session operations; project-only and task-only events never carry fabricated worktree identifiers.
- **Session Scope**: Discriminator identifying a session as `local_unbound` or `project_bound`.
- **Session Binding**: Immutable one-time association from one session to exactly one project and one task revision; created explicitly and never removed or replaced.
- **Binding Event**: Append-only `session.bound` record that adds scope without rewriting prior session or execution events.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The complete 12-step success demonstration finishes with one project, one associated repository whose worktrees inherit membership, one task with two immutable revisions, and one session still bound to revision 1 after restart and replay.
- **SC-002**: Across 100 identical repository-association retries, 100 identical task-revision idempotency-key retries, and 100 identical binding retries, 100% return the original result with exactly one logical association event, revision, and binding event.
- **SC-003**: Migrating at least one real Feature 001 database fixture plus representative edge fixtures preserves 100% of historical repository, worktree, snapshot, session, lease, timestamp, lifecycle-state, projection, and event data; every pre-existing session is classified `local_unbound`; and a second migration run changes nothing.
- **SC-004**: Across at least 100 repeated reads before and after creating later revisions, 100% of prior task revisions remain byte-for-byte unchanged in their versioned canonical machine-readable form and retain the same fingerprint.
- **SC-005**: Replaying the complete deterministically ordered event ledger, including project- and task-scoped events without fabricated worktree identifiers, reproduces project, task, revision, repository-association, and session-binding projections with 100% equality to live projections.
- **SC-006**: In every tested invalid association, task, revision, start, migration, or binding operation, and under concurrent revision creation, zero partial events, zero partial projections, and zero duplicate revision numbers remain.
- **SC-007**: After at least 20 daemon restart and recovery cycles, 100% of bound sessions retain the same project, immutable task revision, session identifier, lifecycle history, and Feature 001 recovery behavior.
- **SC-008**: With external networking disabled at the operating-system level, the complete success demonstration passes using only local filesystem and IPC access.
- **SC-009**: Automated consumers validate and parse 100% of success and failure envelopes for every new or extended command against the published typed contracts; machine requests identify projects, tasks, revisions, repositories, and sessions by ID.
- **SC-010**: On a typical developer machine containing 100 projects, 1,000 tasks, and five revisions per task, 95% of create, list, show, revise, associate, and bind operations complete within 2 seconds.
- **SC-011**: Privacy auditing after migration, revision creation, binding, restart, and replay finds zero ignored-file contents, secret values, environment-variable values, raw resume tokens, or complete goal contracts in diagnostic logs or error responses.
- **SC-012**: Platform-specific acceptance runs pass on Windows, macOS, and Linux with no behavioral difference in identities, immutability, binding, migration preservation, or replay results.

## Out of Scope

- Central-server or multi-device synchronization
- User accounts, project membership, roles, or complex permissions
- Cross-project repository sharing or silent repository reassignment
- Project-repository removal or transfer workflows
- Hard deletion of projects, tasks, revisions, associations, or bindings
- Task archival
- Session unbinding, rebinding, or transition to a newer task revision
- Task checkpoints or continuation across agents
- MCP integration
- Context compilation
- AI planning or memory extraction
- Truth claims
- Embeddings or semantic indexing
- Drift detection
- Completion verification
- Web dashboard functionality
- Multi-agent orchestration

## Dependencies

- Feature 001 local repository and worktree identities, snapshots, sessions, event history, watcher reconciliation, recovery, local persistence, daemon request interface, and command interface are present and remain authoritative.
- Constitution v1.1.0 governs `local_unbound`, explicit binding, immutable task-revision scope, append-only binding events, migration, and the prohibition on project-aware synchronization before this feature's binding rules exist.

## Assumptions

- Project status initially supports only `active` and `archived`; no task-level archival state is introduced.
- IDs are authoritative for projects and tasks. Names and titles are display metadata and may be duplicated.
- A later task revision normally uses the immediately previous revision as its parent; an explicitly selected parent must still be an earlier revision of the same task.
- Goal-contract list ordering is semantically meaningful and therefore preserved in canonical output and fingerprinting.
- Any lifecycle state of a `local_unbound` session may be bound if all repository, worktree, project, task, and revision invariants pass; binding changes scope only, not lifecycle state.
- Archiving a project never rewrites or detaches existing repository associations, tasks, revisions, sessions, bindings, or events.
- This feature introduces local project/task truth only. Even `project_bound` sessions are not synchronized because central synchronization remains out of scope.
- Existing Feature 001 events remain immutable; new scope is represented only by later events and derived projections.

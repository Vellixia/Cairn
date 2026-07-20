# Feature 002 Event Catalog

## Purpose

Feature 002 extends the Feature 001 append-only SQLite ledger. It does not create a
second event store and does not rewrite existing rows. The ledger's existing
`events.seq` remains the authoritative total order for replay.

Every Feature 002 event carries:

- an immutable UUIDv7 event ID;
- an event-specific idempotency key derived deterministically from the operation's
  stable idempotency identity and the event's fixed operation position;
- an event type from the closed catalog below;
- `aggregate_type`, `aggregate_id`, and positive `aggregate_seq`;
- a versioned typed payload;
- an RFC 3339 UTC occurrence timestamp;
- the existing global SQLite sequence allocated on insertion.

## Aggregate envelope

```json
{
  "schema_version": 1,
  "aggregate_type": "project",
  "aggregate_id": "019...",
  "aggregate_seq": 1
}
```

Allowed Feature 002 aggregate types are `project`, `repository`, `task`, and
`session`. Feature 001 worktree-scoped events retain their existing representation.
Legacy rows are interpreted from their real repository, worktree, or session
relationships during replay; no fake aggregate identifier is backfilled.

Per-aggregate numbering covers the post-migration suffix only: the first new event for
an aggregate with only legacy rows receives `aggregate_seq:1`. Replay validates the
numbered suffix as contiguous and keeps the unnumbered legacy prefix in global `seq`
order.

A transaction reserves the next aggregate sequence by updating
`event_aggregate_heads` under `BEGIN IMMEDIATE`, inserts the event, and updates its
projection before commit. Unique constraints on
`(aggregate_type, aggregate_id, aggregate_seq)` and `idempotency_key` make database
state—not a process-local mutex—the concurrency boundary.

Single-event Feature 002 methods may use the caller's operation UUID directly.
Multi-event methods derive distinct lowercase BLAKE3 keys from a fixed domain
separator, the operation UUID, and the zero-based event position. Bound session start
uses the existing Feature 001 stable start/session identity and derives the
`session.bound` key from the created session ID; old clients need no new field. The
operation identity is persisted where a retry must resolve multiple events. This keeps
the existing globally unique event-key constraint while making retries deterministic.

## Catalog

### `project.created`

- Aggregate: `project/{project_id}`
- Aggregate sequence: 1
- Projection: inserts one `projects` row
- Idempotency result: returns the original project

Payload:

```json
{
  "schema_version": 1,
  "project": {
    "project_id": "019...",
    "name": "Cairn",
    "description": null,
    "status": "active",
    "created_at": "2026-07-20T00:00:00Z",
    "updated_at": "2026-07-20T00:00:00Z"
  }
}
```

### `project.updated`

- Aggregate: `project/{project_id}`
- Projection: replaces mutable project metadata with the event's complete post-state
- Idempotency result: returns the original updated project
- Archiving and restoration use this event; there is no delete event

Payload contains `schema_version`, the complete post-update `project`, and a
sorted, closed `changed_fields` list containing any of `name`, `description`, or
`status`. Replaying never computes state from an untyped patch.

### `project.repository_associated`

- Aggregate: `repository/{repository_id}`
- Projection: inserts one immutable `project_repository_associations` row
- The repository aggregate serializes the one-project exclusivity rule
- An identical retry returns the association without another event
- A different project returns `REPOSITORY_PROJECT_CONFLICT`

Payload:

```json
{
  "schema_version": 1,
  "association": {
    "association_id": "019...",
    "project_id": "019...",
    "repository_id": "019...",
    "associated_at": "2026-07-20T00:00:00Z"
  }
}
```

Paths and remote URLs are intentionally absent because repository identity is the
Feature 001 repository ID.

### `task.created`

- Aggregate: `task/{task_id}`
- Aggregate sequence: 1
- Projection: inserts one `tasks` row
- The task is permanently owned by the selected project

Payload:

```json
{
  "schema_version": 1,
  "task": {
    "task_id": "019...",
    "project_id": "019...",
    "title": "Bind local sessions",
    "created_at": "2026-07-20T00:00:00Z"
  }
}
```

Task creation and revision 1 creation are one transaction containing
`task.created` followed by `task.revision_created`. Their global and aggregate
positions are deterministic, and neither is visible if either projection update
fails.

### `task.revision_created`

- Aggregate: `task/{task_id}`
- Projection: inserts an immutable revision and advances
  `tasks.latest_revision_number`
- Revision number is positive and sequential for the task
- An idempotency retry returns the original revision
- Revision content is sufficient for replay and is never diagnostic-log content

Payload:

```json
{
  "schema_version": 1,
  "revision": {
    "revision_id": "019...",
    "task_id": "019...",
    "revision_number": 1,
    "parent_revision_id": null,
    "goal_contract_schema_version": 1,
    "goal_contract": {
      "schema_version": 1,
      "goal": "Bind a session",
      "included_scope": ["Local binding"],
      "excluded_scope": ["Server synchronization"],
      "acceptance_criteria": ["Binding survives restart"],
      "constraints": ["Preserve Feature 001 events"]
    },
    "goal_contract_fingerprint": "blake3-lowercase-hex",
    "created_at": "2026-07-20T00:00:00Z"
  }
}
```

The ledger necessarily contains the user-authored contract so the projection is
rebuildable. The privacy boundary prohibits emitting that content to diagnostics or
error envelopes; it does not prohibit its intentional local persistence.

### `session.bound`

- Aggregate: `session/{session_id}`
- Projection: inserts one immutable `session_bindings` row and changes the session's
  binding mode from `local_unbound` to `project_bound`
- Identical retry returns the original binding
- Any different project or revision returns `SESSION_BINDING_CONFLICT`

Payload:

```json
{
  "schema_version": 1,
  "binding": {
    "session_id": "019...",
    "project_id": "019...",
    "task_id": "019...",
    "task_revision_id": "019...",
    "repository_id": "019...",
    "worktree_id": "019...",
    "bound_at": "2026-07-20T00:00:00Z"
  }
}
```

Repository, worktree, and task IDs are recorded as validated provenance. The
projection key remains the original session ID. No earlier event is changed.

## Bound session start

A bound start uses the existing Feature 001 session-start transaction and appends the
existing `session.started` event followed by `session.bound` in one commit. The
session row and binding projection become visible together. The existing watcher
installation, readiness acknowledgement, authoritative post-install Git
reconciliation, and interruption behavior then proceed unchanged.

A watcher or reconciliation failure may append the existing interruption/failure
events, but it never removes the committed binding or reports a successful start.

## Idempotency ownership

| Operation | Idempotency scope | Retry result |
|---|---|---|
| Create project | `project.created` event key | Original project |
| Update project | `project.updated` event key | Original post-state |
| Associate repository | Association request key plus repository uniqueness | Original association |
| Create task | One request key; two position-derived event keys; request key on revision 1 | Original task and revision 1 |
| Create revision | Revision request key | Original revision |
| Bind session | Binding request key plus unique session projection | Original binding |

Reusing an idempotency key for a different operation body is a typed conflict and
never reinterprets the original event.

## Replay algorithm

1. Read all events by ascending global `seq`.
2. Interpret unchanged Feature 001 events with the Feature 001 replay handlers.
3. Validate each Feature 002 event's type, payload schema version, aggregate scope,
   and contiguous per-aggregate sequence.
4. Apply the typed event to an empty in-memory projection.
5. Compare rebuilt projects, associations, tasks, revisions, and bindings with the
   transactional SQLite projections.
6. Report corruption without mutating the ledger when an unknown version, sequence
   gap, invalid relationship, or projection mismatch appears.

Replay must reproduce duplicate names, list ordering in goal contracts, immutable
revision references, and local-unbound/project-bound classifications exactly.

## Compatibility rules

- Existing global event IDs, sequences, payload bytes, timestamps, and idempotency
  keys remain unchanged.
- New event payload schemas are closed and versioned.
- Unknown event types or payload schema versions fail replay as incompatible data;
  they are not silently ignored.
- There are no project/task delete, repository-transfer, session-unbind, or
  session-rebind events in Feature 002.
- No event authorizes network synchronization or project-memory truth while a
  session is `local_unbound`.

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

A mutation transaction first resolves or reserves the raw caller key in the global `operation_idempotency` registry, then reserves aggregate sequence(s), inserts event(s), and updates projection(s) under one `BEGIN IMMEDIATE` commit. Unique constraints on `(aggregate_type, aggregate_id, aggregate_seq)`, derived event `idempotency_key`, and raw registry key make database state—not a process-local mutex—the concurrency boundary.

Every event key is derived as lowercase BLAKE3 over a fixed domain separator, the raw operation key (or the existing stable Feature 001 start identity), method, and zero-based event position. Raw operation keys are never used directly as event keys. The registry stores method, canonical request fingerprint, result kind/locator, and creation timestamp. The same key/method/request returns that result; any other reuse returns `IDEMPOTENCY_CONFLICT`. The first committed concurrent operation wins; waiters reread the committed record. Bound session start derives `session.bound` from the existing stable start/session identity, so old clients need no new caller field.

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
- Projection: inserts the complete immutable revision and replaces the Task projection with the complete resulting post-state
- Revision number is positive and sequential for the task
- An identical global operation retry returns the original revision
- Revision and Task post-state are sufficient for exact field-for-field replay and are never diagnostic-log content

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
  },
  "task": {
    "task_id": "019...",
    "project_id": "019...",
    "title": "Bind local sessions",
    "latest_revision_number": 1,
    "created_at": "2026-07-20T00:00:00Z",
    "updated_at": "2026-07-20T00:00:00Z"
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

`session.started` always initializes replay scope as `local_unbound`; it never encodes project/task scope. `session.bound` is the sole event that establishes `project_bound`. A bound start uses the existing Feature 001 session-start path and appends `session.started` followed by `session.bound`, plus the session/binding projections, in one database transaction. Neither event nor projection is externally visible unless that transaction commits. The watcher installation/readiness acknowledgement and authoritative post-install Git reconciliation then proceed unchanged.

A watcher or reconciliation failure may append the existing interruption/failure events, but it never removes the committed binding or reports a successful start. Binding mode remains independent of lifecycle state.

## Idempotency ownership

| Operation | Registry method | Result kind / locator |
|---|---|---|
| Create project | `project.create` | `event` / `project.created` event ID |
| Update project | `project.update` | `event` / `project.updated` event ID |
| Associate repository | `project.repository_associate` | `event` / association event ID when created; immutable association ID when a distinct key finds it already present |
| Create task | `task.create` | `event` / `task.revision_created` event ID |
| Create revision | `task.revise` | `event` / `task.revision_created` event ID |
| Bind session | `session.bind` | `event` / `session.bound` event ID when created; immutable session-binding ID when a distinct key finds it already present |

The raw key is globally unique. Same key/method/fingerprint returns the exact first result, including its original `created`/`updated` flag, by rereading immutable event payload or immutable projection. A distinct new key for an identical association/binding may first return `created:false`; retries of that key return the same false result. Another method/request returns `IDEMPOTENCY_CONFLICT`. Registry, derived events, heads, and projections commit or roll back together.

## Replay algorithm

1. Establish the typed dispatcher before US1 can append a Feature 002 event.
2. Read all events by ascending global `seq`.
3. Interpret unchanged Feature 001 events with Feature 001 handlers.
4. Apply project handlers delivered with US1, task/revision handlers with US2, session-binding handlers with US3, and bound-start mixed handlers with US4.
5. Validate type, payload version, aggregate scope, and contiguous per-aggregate sequence.
6. Treat every `session.started` as local-unbound and only `session.bound` as project-bound.
7. For `task.revision_created`, apply both the complete immutable revision and complete Task post-state, including `latest_revision_number` and `updated_at`.
8. Compare every stable field of rebuilt projects, associations, tasks, revisions, and bindings with live projections.
9. Report corruption without mutating the ledger for unknown versions, gaps, invalid relationships, or any field mismatch.

The later replay phase tests mixed Feature 001/002 ledgers, corruption, and equality; it does not introduce the first story handlers. Replay must reproduce duplicate names, goal-contract list ordering, immutable references, timestamps, and binding classifications exactly.

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

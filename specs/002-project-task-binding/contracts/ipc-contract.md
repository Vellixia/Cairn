# Feature 002 IPC Contract

**Feature**: 002-project-task-binding
**Protocol**: existing local JSON-lines `v1.*` namespace
**Transport/security**: unchanged from Feature 001
**Schema source**: typed Rust DTOs in `cairn-protocol`, generated with `schemars`

## Compatibility policy

Feature 002 adds methods and response fields without changing JSON-lines framing,
request correlation, peer authentication, socket/pipe permissions, or the closed
Feature 001 error behavior. Unknown input fields remain ignored and are never emitted.
A breaking field/type change requires a `v2.*` method.

All identifiers are UUID strings and all timestamps are RFC 3339 UTC. Machine
requests use IDs. List limits default to 50 and are bounded to 1–100. Pagination uses
stable ID cursors and deterministic ascending ID order unless a method explicitly
states another order.

## Shared typed objects

### Project

```json
{
  "project_id": "019...",
  "name": "Cairn",
  "description": null,
  "status": "active",
  "created_at": "2026-07-20T00:00:00Z",
  "updated_at": "2026-07-20T00:00:00Z"
}
```

`status` is exactly `active|archived`. Duplicate names are valid.

### ProjectRepositoryAssociation

```json
{
  "association_id": "019...",
  "project_id": "019...",
  "repository_id": "019...",
  "associated_at": "2026-07-20T00:00:00Z"
}
```

### GoalContractV1

```json
{
  "schema_version": 1,
  "goal": "Bind a local session",
  "included_scope": ["Local project association"],
  "excluded_scope": ["Server synchronization"],
  "acceptance_criteria": ["Binding survives restart"],
  "constraints": ["Preserve prior events"]
}
```

The object is closed. List order is meaningful. The goal and each supplied list entry
must remain nonempty after line-ending and surrounding-whitespace normalization.


Invalid input returns `INVALID_GOAL_CONTRACT` with a closed violation union. The array has 1–32 entries. Variants are: `missing_required_field` with one of the six required field names; `malformed_structure` with `field:"goal_contract"`; `empty_goal`; `empty_list_entry` with one list field and index 0–999; and `unsupported_version` with an integer version 0–65535. No submitted scalar/list content is allowed in error data or message.

### Task and TaskRevision

```json
{
  "task_id": "019...",
  "project_id": "019...",
  "title": "Bind sessions",
  "latest_revision_number": 2,
  "created_at": "2026-07-20T00:00:00Z",
  "updated_at": "2026-07-20T00:00:00Z"
}
```

```json
{
  "revision_id": "019...",
  "task_id": "019...",
  "revision_number": 2,
  "parent_revision_id": "019...",
  "goal_contract": {"schema_version": 1, "goal": "...", "included_scope": [], "excluded_scope": [], "acceptance_criteria": [], "constraints": []},
  "goal_contract_fingerprint": "64-lowercase-hex",
  "created_at": "2026-07-20T00:00:00Z"
}
```

### SessionScope

The scope is a closed discriminated union separate from lifecycle state:

```json
{"mode": "local_unbound"}
```

or

```json
{
  "mode": "project_bound",
  "project_id": "019...",
  "task_revision_id": "019..."
}
```

Every Feature 002 session result includes `scope`. Existing lifecycle `state` remains
unchanged.


### OperationIdempotency

Every method carrying `idempotency_key` uses one global immutable `{idempotency_key,method,request_fingerprint,result_kind,result_locator,created_at}` registry. Fingerprints are lowercase BLAKE3 over deterministic typed requests. An operation that appends stores `result_kind:event` and the immutable result event ID; a distinct-key identical association/binding no-op stores its immutable projection kind/ID. Same key/method/request returns the exact original result, including its original `created`/`updated` flag. Another method/request returns `IDEMPOTENCY_CONFLICT`. Registry, heads, events, and projections share one transaction; first commit wins, competitors reread it. Any mutation may return `STORAGE_BUSY` after the bounded 5,000 ms SQLite wait; no application retry occurs.

## Methods

### `v1.project.create`

Params:

```json
{
  "idempotency_key": "uuid",
  "name": "Cairn",
  "description": null
}
```

Result:

```json
{"project": {"project_id": "019..."}, "created": true}
```

An identical raw-key retry returns the exact original object with `created:true`; a different key creates a different project even when names match.
Duplicate project names do not conflict.

Errors: `INVALID_PROJECT`, `IDEMPOTENCY_CONFLICT`, `INTERNAL`.

### `v1.project.list`

Params:

```json
{"status": "active", "after_project_id": null, "limit": 50}
```

Every field is optional. Result:

```json
{"projects": [], "next_after_project_id": null}
```

Archived projects are readable. No name selector appears in the machine contract.

### `v1.project.get`

Params: `{"project_id":"019..."}`

Result includes `project`, its repository associations, and bounded summary counts for
tasks and bound sessions. It does not inline goal-contract content.

Errors: `PROJECT_NOT_FOUND`.

### `v1.project.update`

Params:

```json
{
  "idempotency_key": "uuid",
  "project_id": "019...",
  "name": null,
  "description": null,
  "clear_description": false,
  "status": "archived"
}
```

At least one mutable field must be present. `description` and
`clear_description:true` are mutually exclusive. Restoration is
`status:"active"` and is always explicit.

Result: `{"project": Project, "updated":true}`; an identical raw-key retry returns the exact original post-state with `updated:true` even if later updates changed the live projection.

Errors: `PROJECT_NOT_FOUND`, `INVALID_PROJECT`, `IDEMPOTENCY_CONFLICT`.

### `v1.project.repository_associate`

Params:

```json
{
  "idempotency_key": "uuid",
  "project_id": "019...",
  "repository_id": "019..."
}
```

Result:

```text
{"association": ProjectRepositoryAssociation, "created": true}
```

An identical raw-key retry returns its exact original `created:true|false`. A distinct new key for the same project/repository returns the existing association with `created:false` and records no second event; retries of that key remain false. Paths/remotes are not accepted.

Errors: `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`, `NOT_REGISTERED`, `REPOSITORY_PROJECT_CONFLICT`, `IDEMPOTENCY_CONFLICT`.

### `v1.task.create`

Params:

```text
{
  "idempotency_key": "uuid",
  "project_id": "019...",
  "title": "Bind sessions",
  "goal_contract": GoalContractV1
}
```

Result:

```text
{"task": Task, "revision": TaskRevision, "created": true}
```

Task and revision 1 commit atomically. An identical raw-key retry returns both original objects with `created:true`. Duplicate titles under a different key are valid and create distinct tasks.

Errors: `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`, `INVALID_TASK`,
`INVALID_GOAL_CONTRACT`, `IDEMPOTENCY_CONFLICT`.

### `v1.task.revise`

Params:

```text
{
  "idempotency_key": "uuid",
  "task_id": "019...",
  "parent_revision_id": null,
  "goal_contract": GoalContractV1
}
```

If `parent_revision_id` is omitted/null, the service uses the immediately previous revision. An explicit parent must belong to the same task. Concurrent requests serialize in SQLite and receive unique sequential revision numbers. A failure/cancellation after counter allocation rolls back; the next success receives the gap-free number.

Result: `{"task":Task,"revision":TaskRevision,"created":true}`; an identical raw-key retry returns that exact result and does not allocate another number.

Errors: `TASK_NOT_FOUND`, `PROJECT_ARCHIVED`, `TASK_REVISION_NOT_FOUND`, `TASK_REVISION_CONFLICT`, `INVALID_GOAL_CONTRACT`, `IDEMPOTENCY_CONFLICT`.

### `v1.task.list`

Params:

```json
{"project_id":"019...","after_task_id":null,"limit":50}
```

Result: `{"tasks":[],"next_after_task_id":null}`. Entries include latest revision
number and fingerprint but do not inline goal contracts.

Errors: `PROJECT_NOT_FOUND`.

### `v1.task.get`

Params:

```json
{"task_id":"019...","revision_id":null}
```

When revision ID is absent, the latest immutable revision is returned. Result contains
`task` and one `revision`. Historical selection is always by revision ID.

Errors: `TASK_NOT_FOUND`, `TASK_REVISION_NOT_FOUND`.

### `v1.session.bind`

Params:

```json
{
  "idempotency_key": "uuid",
  "session_id": "019...",
  "project_id": "019...",
  "task_revision_id": "019..."
}
```

Result:

```json
{
  "session_id": "019...",
  "scope": {
    "mode": "project_bound",
    "project_id": "019...",
    "task_revision_id": "019..."
  },
  "bound_at": "2026-07-20T00:00:00Z",
  "created": true
}
```

The daemon validates repository association and task-revision ownership. An identical raw-key retry returns its exact original `created:true|false`. A distinct new key for the same binding returns the immutable binding with `created:false` and no event; retries of that key remain false. Binding never changes the session ID or earlier events.

Errors: `SESSION_NOT_FOUND`, `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`,
`TASK_REVISION_NOT_FOUND`, `REPOSITORY_NOT_ASSOCIATED`,
`TASK_REVISION_PROJECT_MISMATCH`, `SESSION_BINDING_CONFLICT`,
`IDEMPOTENCY_CONFLICT`.

### Extended `v1.session.start`

Feature 001 params gain an optional closed `scope`:

```json
{
  "repository_id": "019...",
  "agent_type": "codex",
  "agent_instance_id": "uuid",
  "scope": {"mode":"local_unbound"}
}
```

or:

```json
{
  "repository_id": "019...",
  "agent_type": "codex",
  "agent_instance_id": "uuid",
  "scope": {
    "mode": "project_bound",
    "project_id": "019...",
    "task_revision_id": "019..."
  }
}
```

Omission by a Feature 001 client decodes as a request for `local_unbound`; new CLI requests send the discriminator. That start is allowed only if the repository has no active project association or its associated active project has no selectable active task revision. Otherwise explicit or omitted unbound scope returns `PROJECT_SCOPE_REQUIRED` and creates no session/event/projection. Historical migrated unbound sessions remain valid.

The result's existing `session` object gains `scope`. Collision returns `existing` only when requested and persisted scopes are identical; otherwise `SESSION_SCOPE_CONFLICT`. Normatively, `session.started` always initializes replay as `local_unbound`, and `session.bound` alone establishes `project_bound`. A bound start appends them in that order with the session/binding projections in one database transaction; neither event is externally visible unless it commits. Watcher readiness and post-install Git reconciliation follow unchanged.

Additional errors: `PROJECT_SCOPE_REQUIRED`, `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`, `TASK_REVISION_NOT_FOUND`, `REPOSITORY_NOT_ASSOCIATED`, `TASK_REVISION_PROJECT_MISMATCH`, `SESSION_SCOPE_CONFLICT`. Existing `WATCHER_START_FAILED` remains typed exactly as in Feature 001.

### Extended `v1.session.get` and `v1.session.list`

Every full session and summary gains `scope`. Existing filters and ambiguity behavior
remain unchanged. Optional `project_id` and `task_revision_id` list filters are
additive and compose with existing filters.

### Extended `v1.events.list`

Params add optional `aggregate_type` and `aggregate_id`; they must be supplied together
and compose with existing filters. New event results include
`aggregate_type`, `aggregate_id`, and `aggregate_seq`. Those fields are null only for
stored pre-migration Feature 001 rows and required for every post-migration row. Global
`seq` ordering, bounded pagination, and all existing filters remain unchanged.

## Stable errors

The `error.data` object is a closed discriminated union. It never accepts arbitrary
JSON for a typed Feature 002 code.

| Wire code | Typed data |
|---|---|
| `PROJECT_NOT_FOUND` | `{"kind":"project_not_found","project_id":"uuid"}` |
| `PROJECT_ARCHIVED` | `{"kind":"project_archived","project_id":"uuid"}` |
| `PROJECT_SCOPE_REQUIRED` | `{"kind":"project_scope_required","repository_id":"uuid","project_id":"uuid"}` |
| `INVALID_PROJECT` | `{"kind":"invalid_project","field":"name|description|status","rule":"required|empty|conflicting_fields"}` |
| `TASK_NOT_FOUND` | `{"kind":"task_not_found","task_id":"uuid"}` |
| `INVALID_TASK` | `{"kind":"invalid_task","field":"title","rule":"required|empty"}` |
| `TASK_REVISION_NOT_FOUND` | `{"kind":"task_revision_not_found","revision_id":"uuid"}` |
| `TASK_REVISION_CONFLICT` | `{"kind":"task_revision_conflict","task_id":"uuid","reason":"parent_mismatch"}` |
| `REPOSITORY_NOT_ASSOCIATED` | `{"kind":"repository_not_associated","repository_id":"uuid","project_id":"uuid"}` |
| `REPOSITORY_PROJECT_CONFLICT` | `{"kind":"repository_already_associated","repository_id":"uuid","existing_project_id":"uuid","requested_project_id":"uuid"}` |
| `TASK_REVISION_PROJECT_MISMATCH` | `{"kind":"task_revision_project_mismatch","revision_id":"uuid","expected_project_id":"uuid"}` |
| `SESSION_BINDING_CONFLICT` | `{"kind":"session_already_bound","session_id":"uuid","existing_project_id":"uuid","existing_revision_id":"uuid"}` |
| `SESSION_SCOPE_CONFLICT` | `{"kind":"session_scope_conflict","session_id":"uuid","existing_mode":"local_unbound|project_bound","requested_mode":"local_unbound|project_bound"}` |
| `AMBIGUOUS_NAME` | `{"kind":"ambiguous_name","entity":"project|task","candidate_ids":["uuid"],"truncated":false}` |
| `INVALID_GOAL_CONTRACT` | `{"kind":"invalid_goal_contract","violations":[GoalContractViolation]}` |
| `MIGRATION_FAILED` | `{"kind":"migration_failure","target_version":2}` |
| `IDEMPOTENCY_CONFLICT` | `{"kind":"idempotency_conflict","idempotency_key":"uuid","existing_method":"project.create|project.update|project.repository_associate|task.create|task.revise|session.bind","requested_method":"same closed union","reason":"method_mismatch|request_mismatch"}` |
| `STORAGE_BUSY` | `{"kind":"storage_busy","max_elapsed_ms":5000}` |

`GoalContractViolation` is a closed discriminated union:

- `{"violation":"missing_required_field","field":"schema_version|goal|included_scope|excluded_scope|acceptance_criteria|constraints"}`;
- `{"violation":"malformed_structure","field":"goal_contract"}`;
- `{"violation":"empty_goal","field":"goal"}`;
- `{"violation":"empty_list_entry","field":"included_scope|excluded_scope|acceptance_criteria|constraints","index":0}`;
- `{"violation":"unsupported_version","version":2}`.

The violations array is 1–32; `index` is 0–999 and `version` is 0–65535.
`AMBIGUOUS_NAME.candidate_ids` contains at most 20 deterministic ascending IDs.
Complete goal text, list entries, raw SQL errors, paths, tokens, and internal details
are forbidden from error data and messages.

The Rust domain variants may be named `RepositoryAlreadyAssociated` and
`SessionAlreadyBound`, but their canonical wire codes are
`REPOSITORY_PROJECT_CONFLICT` and `SESSION_BINDING_CONFLICT`.

## Schema and golden requirements

Checked-in JSON Schemas and goldens cover:

- one request and success response for every method;
- identical global retry plus same-key different-method and different-request conflicts;
- project archive/restore;
- revision 1, later revision, historical get, and complete Task post-state;
- `local_unbound` and `project_bound` scopes;
- old omitted and explicit unbound requests in both bootstrap-eligible and `PROJECT_SCOPE_REQUIRED` states;
- bound-start `session.started` then `session.bound` order/atomic invisibility and watcher failures at install/reconcile;
- each missing goal field, malformed structure, empty goal, every list's empty-entry case, unsupported version, 1–32 bound, and no contract-content leakage;
- every closed error-data discriminator, including `STORAGE_BUSY`;
- bounded `AMBIGUOUS_NAME`;
- absence of goal text, internal paths, raw tokens, checksums, and raw migration details.

Compatibility tripwires fail if an ID changes type, a status/scope/stage enum widens or
shrinks incompatibly, a required discriminator disappears, arbitrary error JSON is
accepted, an immutable revision field becomes optional, or a Feature 001 golden stops
validating.

Daemon tests replay all goldens over the real local socket or named pipe. Contract
tests validate both serialization directions and every golden against generated
schemas.

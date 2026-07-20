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

### Task and TaskRevision

```json
{
  "task_id": "019...",
  "project_id": "019...",
  "title": "Bind sessions",
  "latest_revision_number": 2,
  "created_at": "2026-07-20T00:00:00Z"
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

An identical idempotency retry returns the original object with `created:false`.
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

Result: `{"project": Project, "updated":true}`; an identical retry returns the original
post-state with `updated:false`.

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

```json
{"association": ProjectRepositoryAssociation, "created": true}
```

An identical same-project retry returns `created:false`. The repository ID is the
Feature 001 Cairn identity; path and remote URL are not accepted.

Errors: `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`, `NOT_REGISTERED`,
`REPOSITORY_PROJECT_CONFLICT`.

### `v1.task.create`

Params:

```json
{
  "idempotency_key": "uuid",
  "project_id": "019...",
  "title": "Bind sessions",
  "goal_contract": GoalContractV1
}
```

Result:

```json
{"task": Task, "revision": TaskRevision, "created": true}
```

Task and revision 1 are committed atomically. An identical retry returns both original
objects with `created:false`. Duplicate titles are valid.

Errors: `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`, `INVALID_TASK`,
`INVALID_GOAL_CONTRACT`, `IDEMPOTENCY_CONFLICT`.

### `v1.task.revise`

Params:

```json
{
  "idempotency_key": "uuid",
  "task_id": "019...",
  "parent_revision_id": null,
  "goal_contract": GoalContractV1
}
```

If `parent_revision_id` is omitted/null, the service uses the immediately previous
revision. An explicit parent must belong to the same task. Concurrent requests are
serialized in SQLite and receive unique sequential revision numbers.

Result: `{"task":Task,"revision":TaskRevision,"created":true}`.

Errors: `TASK_NOT_FOUND`, `PROJECT_ARCHIVED`, `TASK_REVISION_NOT_FOUND`,
`TASK_REVISION_CONFLICT`, `INVALID_GOAL_CONTRACT`, `IDEMPOTENCY_CONFLICT`.

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

The daemon validates the session worktree's repository association and task-revision
ownership. An identical retry returns the original binding with `created:false`.
Binding never changes the session ID or any earlier event.

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

Omission by a Feature 001 client decodes exactly as `local_unbound`. New CLI requests
always send the explicit discriminator.

The result's existing `session` object gains `scope`. Collision returns `existing`
only if requested and persisted scopes are identical. Otherwise it returns
`SESSION_SCOPE_CONFLICT`. A successful bound start still guarantees Feature 001
watcher readiness and post-install Git reconciliation.

Additional errors: `PROJECT_NOT_FOUND`, `PROJECT_ARCHIVED`,
`TASK_REVISION_NOT_FOUND`, `REPOSITORY_NOT_ASSOCIATED`,
`TASK_REVISION_PROJECT_MISMATCH`, `SESSION_SCOPE_CONFLICT`.
Existing `WATCHER_START_FAILED` remains typed exactly as in Feature 001.

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
| `INVALID_PROJECT` | `{"kind":"invalid_project","field":"name|description|status","rule":"required|empty|conflicting_fields"}` |
| `TASK_NOT_FOUND` | `{"kind":"task_not_found","task_id":"uuid"}` |
| `INVALID_TASK` | `{"kind":"invalid_task","field":"title","rule":"required|empty"}` |
| `TASK_REVISION_NOT_FOUND` | `{"kind":"task_revision_not_found","revision_id":"uuid"}` |
| `TASK_REVISION_CONFLICT` | `{"kind":"task_revision_conflict","task_id":"uuid","reason":"parent_mismatch|idempotency_mismatch"}` |
| `REPOSITORY_NOT_ASSOCIATED` | `{"kind":"repository_not_associated","repository_id":"uuid","project_id":"uuid"}` |
| `REPOSITORY_PROJECT_CONFLICT` | `{"kind":"repository_already_associated","repository_id":"uuid","existing_project_id":"uuid","requested_project_id":"uuid"}` |
| `TASK_REVISION_PROJECT_MISMATCH` | `{"kind":"task_revision_project_mismatch","revision_id":"uuid","expected_project_id":"uuid"}` |
| `SESSION_BINDING_CONFLICT` | `{"kind":"session_already_bound","session_id":"uuid","existing_project_id":"uuid","existing_revision_id":"uuid"}` |
| `SESSION_SCOPE_CONFLICT` | `{"kind":"session_scope_conflict","session_id":"uuid","existing_mode":"local_unbound|project_bound","requested_mode":"local_unbound|project_bound"}` |
| `AMBIGUOUS_NAME` | `{"kind":"ambiguous_name","entity":"project|task","candidate_ids":["uuid"],"truncated":false}` |
| `INVALID_GOAL_CONTRACT` | `{"kind":"invalid_goal_contract","violations":[{"field":"goal|included_scope|excluded_scope|acceptance_criteria|constraints","rule":"missing_goal|empty_goal|empty_list_item|unsupported_schema_version"}]}` |
| `MIGRATION_FAILED` | `{"kind":"migration_failure","target_version":2}` |
| `IDEMPOTENCY_CONFLICT` | `{"kind":"idempotency_conflict","operation":"project.create|project.update|project.repository_associate|task.create|task.revise|session.bind"}` |

`AMBIGUOUS_NAME.candidate_ids` contains at most 20 deterministic ascending IDs.
Complete goal text, list entries, raw SQL errors, paths, tokens, and internal details
are forbidden from error data and messages.

The Rust domain variants may be named `RepositoryAlreadyAssociated` and
`SessionAlreadyBound`, but their canonical wire codes are
`REPOSITORY_PROJECT_CONFLICT` and `SESSION_BINDING_CONFLICT`.

## Schema and golden requirements

Checked-in JSON Schemas and goldens cover:

- one request and success response for every method;
- idempotent retry and every stable error branch;
- project archive/restore;
- revision 1, later revision, and historical get;
- `local_unbound` and `project_bound` session scopes;
- old Feature 001 start request omission;
- bound-start watcher failure at install and reconcile;
- all closed error-data discriminators;
- bounded `AMBIGUOUS_NAME`;
- absence of goal text, internal paths, raw tokens, and raw migration details.

Compatibility tripwires fail if an ID changes type, a status/scope/stage enum widens or
shrinks incompatibly, a required discriminator disappears, arbitrary error JSON is
accepted, an immutable revision field becomes optional, or a Feature 001 golden stops
validating.

Daemon tests replay all goldens over the real local socket or named pipe. Contract
tests validate both serialization directions and every golden against generated
schemas.

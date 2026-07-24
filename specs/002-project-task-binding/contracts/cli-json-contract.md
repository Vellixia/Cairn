# Feature 002 CLI JSON Contract

**Binary**: `cairn`
**Machine mode**: existing global `--json`
**Envelope schema**: additive `cairn.cli.v1`

## Envelope

Every machine-mode invocation writes exactly one JSON object to stdout:

```json
{
  "schema": "cairn.cli.v1",
  "ok": true,
  "command": "project.create",
  "data": {}
}
```

or:

```json
{
  "schema": "cairn.cli.v1",
  "ok": false,
  "command": "session.bind",
  "error": {
    "code": "SESSION_BINDING_CONFLICT",
    "message": "session is already bound",
    "data": {
      "kind": "session_already_bound",
      "session_id": "019...",
      "existing_project_id": "019...",
      "existing_revision_id": "019..."
    }
  }
}
```

Machine mode requires IDs and never resolves names. Diagnostics go only to stderr.
Human mode may accept exact names where documented, but ambiguity returns an error
rather than choosing by recency or list order.

All command data and error objects reuse the typed IPC DTOs in
[ipc-contract.md](ipc-contract.md).

## Commands

| CLI command | Required selection | IPC method | Success data |
|---|---|---|---|
| `cairn project create --name NAME [--description TEXT]` | none | `v1.project.create` | `{project, created}` |
| `cairn project list [--status active|archived]` | none | `v1.project.list` | `{projects, next_after_project_id}` |
| `cairn project show --project-id ID` | project ID | `v1.project.get` | project detail |
| `cairn project update --project-id ID ...` | project ID | `v1.project.update` | `{project, updated}` |
| `cairn project repository add --project-id ID --repository-id ID` | both IDs | `v1.project.repository_associate` | `{association, created}` |
| `cairn task create --project-id ID --title TITLE --goal-contract FILE|-` | project ID | `v1.task.create` | `{task, revision, created}` |
| `cairn task revise --task-id ID --goal-contract FILE|- [--parent-revision-id ID]` | task ID | `v1.task.revise` | `{task, revision, created}` |
| `cairn task list --project-id ID` | project ID | `v1.task.list` | `{tasks, next_after_task_id}` |
| `cairn task show --task-id ID [--revision-id ID]` | task ID/revision ID | `v1.task.get` | `{task, revision}` |
| `cairn session bind --session ID --project-id ID --task-revision-id ID` | all three IDs | `v1.session.bind` | binding result |
| `cairn session start --local-unbound ...` | explicit mode | `v1.session.start` | existing start data with typed scope |
| `cairn session start --project-id ID --task-revision-id ID ...` | both binding IDs | `v1.session.start` | existing start data with typed scope |
| `cairn session show ...` | existing selectors | `v1.session.get` | existing result with typed scope |

`--local-unbound` conflicts with project-binding flags; project and revision flags are paired. It succeeds only when the repository has no active project association or its associated active project has no selectable active task revision. Otherwise it returns `PROJECT_SCOPE_REQUIRED` without creating state. The CLI generates a UUID idempotency key for Feature 002 mutating commands unless a machine caller supplies `--idempotency-key`; one raw key is global across methods and another-method/request reuse returns `IDEMPOTENCY_CONFLICT`.

Goal contracts are read from a file or stdin as typed JSON. They are never accepted as individual shell arguments, repeated in diagnostics, or copied into error messages.

## Human name resolution

Human mode may additionally accept `--project NAME` or `--task TITLE --project-id ID`. It resolves internally through typed ID-list responses using exact, case-sensitive normalized display text:

- zero matches renders the relevant human not-found error;
- one match proceeds using its ID;
- multiple matches render internal typed `AMBIGUOUS_NAME` data with bounded candidate IDs and exit 4.

At most 20 deterministic candidate IDs are rendered. JSON mode rejects every name selector as exit-2 usage error before IPC and requires IDs. Therefore `AMBIGUOUS_NAME` is an internal typed human-rendering shape, not a valid JSON-mode name-selection operation or reachable JSON command result.

## Human output requirements

- Project list/show prints the stable project ID beside every possibly duplicate name.
- Task list/show prints the stable task ID and revision ID beside every title.
- Archived projects are visibly marked `archived`.
- Session start/show prints exactly one of `local_unbound` or `project_bound`.
- Bound output prints project and immutable task-revision IDs.
- Unbound output never prints inferred/fabricated project or task fields.
- Binding retry is labeled unchanged/idempotent rather than a second binding.
- Human output never prints a resume token or complete goal contract unless the user
  explicitly invokes task show for that content.

## JSON examples

Project creation:

```json
{
  "schema": "cairn.cli.v1",
  "ok": true,
  "command": "project.create",
  "data": {
    "project": {
      "project_id": "019...",
      "name": "Cairn",
      "description": null,
      "status": "active",
      "created_at": "2026-07-20T00:00:00Z",
      "updated_at": "2026-07-20T00:00:00Z"
    },
    "created": true
  }
}
```

Task revision:

```json
{
  "schema": "cairn.cli.v1",
  "ok": true,
  "command": "task.revise",
  "data": {
    "task": {"task_id":"019...","project_id":"019...","title":"Binding","latest_revision_number":2,"created_at":"2026-07-20T00:00:00Z","updated_at":"2026-07-20T00:00:00Z"},
    "revision": {
      "revision_id":"019...",
      "task_id":"019...",
      "revision_number":2,
      "parent_revision_id":"019...",
      "goal_contract":{"schema_version":1,"goal":"...","included_scope":[],"excluded_scope":[],"acceptance_criteria":[],"constraints":[]},
      "goal_contract_fingerprint":"64-lowercase-hex",
      "created_at":"2026-07-20T00:00:00Z"
    },
    "created": true
  }
}
```

Explicit unbound session (only from a bootstrap-eligible repository):

```json
{
  "schema": "cairn.cli.v1",
  "ok": true,
  "command": "session.start",
  "data": {
    "session": {
      "session_id":"019...",
      "state":"active",
      "scope":{"mode":"local_unbound"}
    },
    "resume_token":"base64...",
    "outcome":"created"
  }
}
```

Bound session:

```json
{
  "schema": "cairn.cli.v1",
  "ok": true,
  "command": "session.show",
  "data": {
    "resolution":"single",
    "session":{
      "session_id":"019...",
      "state":"active",
      "scope":{
        "mode":"project_bound",
        "project_id":"019...",
        "task_revision_id":"019..."
      }
    }
  }
}
```

Internal `AMBIGUOUS_NAME` typed rendering fixture (schema/unit-test reference only; no JSON-mode name selector can produce this):

```json
{
  "code":"AMBIGUOUS_NAME",
  "data":{
    "kind":"ambiguous_name",
    "entity":"project",
    "candidate_ids":["019...","019..."],
    "truncated":false
  }
}
```

A JSON-mode invocation containing a name selector instead produces a normal exit-2 usage envelope and sends no IPC request.

## Exit codes

| Exit | Meaning |
|---|---|
| 0 | Success, including identical idempotent retry |
| 1 | Operation rejected or failed, including archive, association, revision, binding, scope, goal, and watcher errors |
| 2 | Invalid command syntax, missing paired IDs, invalid JSON input |
| 3 | Repository/worktree missing or not registered |
| 4 | `AMBIGUOUS_NAME` or existing ambiguous session selection |
| 5 | Daemon unavailable after existing spawn/retry behavior |
| 6 | `STATE_CORRUPTED` or `MIGRATION_FAILED` |

The same code applies in human and JSON mode. A failure never produces a success
envelope.

## Stable error handling

The CLI preserves daemon error codes and typed data without adding internal context.
In particular:

- `PROJECT_SCOPE_REQUIRED`, `REPOSITORY_PROJECT_CONFLICT`, `SESSION_BINDING_CONFLICT`, `SESSION_SCOPE_CONFLICT`, `IDEMPOTENCY_CONFLICT`, and `STORAGE_BUSY` map to exit 1;
- every `INVALID_GOAL_CONTRACT` violation maps to exit 1 and prints only bounded violation/field/index/version metadata;
- `AMBIGUOUS_NAME` internal human rendering maps to exit 4; JSON-mode name flags are exit 2;
- `MIGRATION_FAILED` maps to exit 6;
- `WATCHER_START_FAILED` retains the Feature 001 install/reconcile payload and exit 1.

No error envelope contains complete goal content, a goal fingerprint's source bytes,
internal paths, raw SQL, environment values, ignored-file contents, secrets, or
resume tokens.

## Golden and integration coverage

Checked-in goldens include:

- success and stable-error envelopes for every new command;
- duplicate project/task names with ID-only JSON access plus separate internal human ambiguity rendering;
- archive and explicit restoration;
- exact same-key original `created/updated` flags (including after later project updates), distinct-key association/binding `created:false`, and cross-method/request conflicts;
- revision 1 and revision 2 with complete Task post-state while revision 1 remains unchanged;
- bootstrap-eligible unbound start, `PROJECT_SCOPE_REQUIRED` explicit/omitted cases, bound start, existing-session binding, retry, and conflict;
- every missing required goal field, malformed structure, empty goal, each list's empty entry, unsupported version, exit 1, and contract-content non-leakage;
- project/repository and task/project mismatches;
- bounded `AMBIGUOUS_NAME` internal rendering plus JSON name-flag usage rejection;
- checksum-mismatch/future-version `MIGRATION_FAILED` without internal detail;
- `STORAGE_BUSY` and all Feature 001 envelopes unchanged except additive session scope.

Each golden validates against the generated CLI schema. Integration tests assert exact
exit codes and exactly one stdout JSON object. Compatibility tripwires reject missing
scope discriminators, widened error data, reordered/rewritten immutable revision
fields, or accidental name selection in machine mode.

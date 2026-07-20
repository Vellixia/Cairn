# Quickstart: Project and Task Binding

This is the planned Feature 002 acceptance walkthrough. The commands become executable
after implementation; this planning artifact is not execution evidence.

## Prerequisites

- A clean checkout of the implementation commit.
- Rust 1.85 or newer, Git, SQLite tooling, and `jq`.
- `cairn` and `cairn-daemon` built from the same commit.
- An isolated `CAIRN_DATA_DIR`.
- No server account or external service.

Build and establish the quality baseline:

```sh
cargo build --workspace --all-targets
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

Create a disposable Git repository, start the daemon using the repository's documented
test harness, and keep all JSON outputs as evidence. Environment variables below are
illustrative identifiers, not secret values.

## 1. Register the repository

```sh
cairn --json init
```

Record `repository.repository_id` and `worktree.worktree_id`. Move the directory and
run `cairn --json status`; the IDs must remain unchanged.

## 2. Create a project

```sh
cairn --json project create \
  --name "Binding demo" \
  --description "Local Feature 002 acceptance"
```

Record `project.project_id`. The project begins `active`. Creating another project with
the same name must succeed with a different ID; list output must show both IDs.

## 3. Associate the repository

```sh
cairn --json project repository add \
  --project-id "$CAIRN_DEMO_PROJECT_ID" \
  --repository-id "$CAIRN_DEMO_REPOSITORY_ID"
```

Repeat the exact request/idempotency key and verify `created:false` with no second
`project.repository_associated` event. Associating the repository to the duplicate-name
project must return `REPOSITORY_PROJECT_CONFLICT` and leave the original association
unchanged.

## 4. Create task revision 1

Prepare `goal-v1.json`:

```json
{
  "schema_version": 1,
  "goal": "Bind one local session to immutable revision 1",
  "included_scope": [
    "Local project and repository association",
    "Existing-session binding"
  ],
  "excluded_scope": [
    "Server synchronization",
    "Session rebinding"
  ],
  "acceptance_criteria": [
    "Binding survives daemon restart",
    "Earlier Feature 001 events remain unchanged"
  ],
  "constraints": [
    "Use Cairn repository identity",
    "Append session.bound exactly once"
  ]
}
```

Create the task:

```sh
cairn --json task create \
  --project-id "$CAIRN_DEMO_PROJECT_ID" \
  --title "Bind bootstrap session" \
  --goal-contract goal-v1.json
```

Record task ID, revision ID, revision number `1`, canonical fingerprint, parent
`null`, and the pre-binding ordered Feature 001 event manifest.

## 5. Start a local bootstrap session

```sh
cairn --json session start \
  --local-unbound \
  --agent codex \
  --agent-instance "$CAIRN_DEMO_AGENT_INSTANCE"
```

The successful result must show:

```json
{"scope":{"mode":"local_unbound"}}
```

Success still guarantees watcher installation acknowledgement and authoritative
post-install Git reconciliation. Record the session ID and capture the resume token
only through the existing secure Feature 001 mechanism.

## 6. Bind the existing session

```sh
cairn --json session bind \
  --session "$CAIRN_DEMO_SESSION_ID" \
  --project-id "$CAIRN_DEMO_PROJECT_ID" \
  --task-revision-id "$CAIRN_DEMO_REVISION_1_ID"
```

The result must preserve the session ID and show:

```json
{
  "scope": {
    "mode": "project_bound",
    "project_id": "019...",
    "task_revision_id": "019..."
  },
  "created": true
}
```

Repeat the identical binding and expect `created:false`, one binding projection, and
one `session.bound` event. A different project or revision must return
`SESSION_BINDING_CONFLICT`.

## 7. Prove Feature 001 history preservation

List events in ascending global sequence through the existing bounded event interface.
Compare the stored pre-binding manifest with the same event IDs, sequence values,
types, payload hashes, and timestamps after binding. Every earlier row must match.
Only one later `session.bound` event is added.

## 8. Restart the daemon

Stop it using the repository's normal test harness, start it again against the same
data directory, then run:

```sh
cairn --json session show --session "$CAIRN_DEMO_SESSION_ID"
```

The original lifecycle state is recovered under Feature 001 rules and the scope remains
`project_bound` with revision 1.

## 9. Create immutable revision 2

Prepare `goal-v2.json` with changed user intent and run:

```sh
cairn --json task revise \
  --task-id "$CAIRN_DEMO_TASK_ID" \
  --parent-revision-id "$CAIRN_DEMO_REVISION_1_ID" \
  --goal-contract goal-v2.json
```

Expect revision number `2`, parent revision 1, and a different fingerprint. Show
revision 1 explicitly and verify every stored field/fingerprint remains unchanged.

## 10. Prove binding immutability

```sh
cairn --json session show --session "$CAIRN_DEMO_SESSION_ID"
```

The session still names revision 1. Continuing under revision 2 requires starting a
new bound session:

```sh
cairn --json session start \
  --project-id "$CAIRN_DEMO_PROJECT_ID" \
  --task-revision-id "$CAIRN_DEMO_REVISION_2_ID" \
  --agent codex \
  --agent-instance "$CAIRN_DEMO_SECOND_AGENT_INSTANCE"
```

This path must retain all Feature 001 uniqueness, lease, watcher, snapshot, and
recovery behavior.

## 11. Replay

Run the implementation's replay verifier against the complete ordered ledger. It must
reconstruct projects, repository associations, tasks, both revisions, and the session
binding exactly, while continuing to interpret all Feature 001 events.

Expected minimum Feature 002 event counts:

```text
project.created                       1
project.repository_associated         1
task.created                          1
task.revision_created                 2
session.bound                         1
```

No retry adds another logical event.

## 12. Archive and restore

```sh
cairn --json project update \
  --project-id "$CAIRN_DEMO_PROJECT_ID" \
  --status archived
```

Show/list still work. Repository additions, task creation/revision, binding, and bound
session start return `PROJECT_ARCHIVED`. Existing bound sessions remain inspectable.

Restore explicitly:

```sh
cairn --json project update \
  --project-id "$CAIRN_DEMO_PROJECT_ID" \
  --status active
```

New mutations are allowed again. These transitions add two `project.updated` events.

## Migration acceptance

Generate a populated database by running the frozen Feature 001 implementation
`4a06c4125715bb4b78b54e49c81eccd82100a7b7`, then start the Feature 002 daemon against
a copy. Verify:

- migration commits once and reaches local schema version 2;
- all old table counts, identifiers, event sequences/payload hashes, snapshots,
  lifecycle fields, timestamps, leases, and resume-token hashes match the manifest;
- every old session is `local_unbound`;
- new project/task/association/binding tables are empty;
- no synthetic event was appended;
- interruption before commit and subsequent restart safely retries;
- a forced failure reports `MIGRATION_FAILED` without raw SQL/path or partial state.

## Privacy acceptance

Use unique sentinels in every goal-contract field, ignored file, environment variable,
resume token, and injected internal error. Capture stdout, stderr, tracing, IPC errors,
and database contents. Goal-contract content may exist only in intentional local
revision/event storage and explicit task-show output. Other prohibited sentinel
locations must remain absent.

## Genuine offline acceptance

Fetch dependencies and build before isolation. Enter a Linux network namespace or
container with networking disabled, explicitly prove external networking fails, and
then run registration, project creation, association, task creation, local session
start, binding, restart, inspection, revision 2, and replay using filesystem and local
IPC. Failure to establish isolation is a test failure, not a skip.

Record the isolation mechanism, exact implementation SHA, OS/architecture, Rust/Cargo
versions, exact commands, event counts, and results. `cargo --offline` by itself is not
network-isolation evidence.

## Final validation

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test -p cairn-daemon --test us2_agent_sim
cargo test -p cairn-daemon --test us3_tracking
cargo test -p cairn-daemon --test us3_events
```

Run platform-specific migration, persistence, restart, and IPC coverage on Linux,
macOS, and Windows where behavior differs. Preserve the exact commit and completed-run
evidence; workflow configuration alone is insufficient.

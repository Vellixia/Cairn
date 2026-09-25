# Task 2A report — core and edge checkpoint

## Completed slice

- Added a focused core test proving `MemoryScope` accepts only session, branch,
  and project applicability; observed RED while task scope still ranked first,
  then GREEN after removal.
- Removed core Task type, Task scope, task wire requests, session task field,
  briefing task field, task handoff synthesis, task core module, task criteria
  module, and direct MCP compatibility `task_id` field.
- Added immutable forward SQLite migration `0013_remove_task_runtime.sql`.

## Store completion

- Deleted task repositories, criteria module, task payload/outbox paths,
  criterion evidence, task session binding, task search scope, task continuity
  snapshots/divergence, and task durability analytics.
- Search precedence is now session → branch → project.
- Forward migration 13 drops task runtime tables, session task columns, task
  continuity columns, and criterion evidence. Existing migration code remains
  only to advance historical schemas before the removal migration runs.

## Deferred downstream fallout

`cairnd`, `cairn-server`, integration tests, and web contract consumers still
reference removed Task APIs/types. They are intentionally not touched by this
Task 2A checkpoint. `crates/cairn/src/mcp.rs` only removes its compatibility
field because it is coupled to the deleted wire request field.

## Task 2B — daemon downstream deletion

- Removed daemon Task briefing state, task-scoped context, task handoff input,
  session task binding, task request handling, task continuity assumptions,
  task outbox backfill, pull import, and criterion verification paths.
- Session applicability now remains session → branch → project. Branch and
  project warnings/pins retain their existing paths.
- Removed task-only migration, criteria, continuity, and context integration
  tests. Updated surviving fixtures for the deleted `task_id` field.
- Server task APIs/contracts remain for Task 5 classification; this bounded
  downstream slice did not change web files or historical PostgreSQL migrations.

### Validation

- `cargo check --workspace` passes.
- `cargo test --workspace --all-targets --no-run` passes (two existing unused
  test-support warnings in `cairnd`).
- `git diff --check` passes.

### Follow-up

- Replace temporary `#[cfg(any())]` guards on deleted daemon task helpers and
  task-only test cases with physical deletion during final Task 2 integration.
- Run Clippy after server Task routes/contracts are removed; current server
  web-operation dead-code warnings predate this slice.

## Task 2B final cleanup

- Physically deleted remaining daemon Task-only helpers, criterion verification,
  task pull import, and disabled task test cases; no `#[cfg(any())]` guards
  remain under `crates/cairnd` or `tests`.
- Reenabled surviving `handlers` tests under `#[cfg(test)]`; removed obsolete
  `SessionStart.task_id` fixture fields and Task-only session tests.
- Removed Task scope ordering from daemon verification and stale sync comments.

### Validation

- `PATH="$HOME/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH" cargo check --workspace` passes (two pre-existing `cairn-server` web-operation dead-code warnings).
- `PATH="$HOME/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH" cargo test --workspace --all-targets --no-run` passes.
- `cargo clippy -p cairnd -- -D warnings` is blocked before `cairnd` by unchanged `cairn-store/src/transfer.rs:557` (`clippy::redundant_closure`).
- `git diff --check` passes.

## Evidence

`PATH="$HOME/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH" cargo test -p cairn-core domain::tests::scope_precedence_is_session_branch_project`
passes after first failing as expected.

`cargo test -p cairn-core -p cairn-store --lib` passes (293 core, 220 store).

`cargo check -p cairn-core -p cairn-store` and `git diff --check` pass.

Workspace check after this checkpoint fails first in `cairnd`: task briefing,
continuity, handlers, handoffs, sync, and verification still call the deleted
store APIs. Server task sync/retrieval/API references and task integration
tests also remain. These are follow-on deletion targets, not compatibility
shims.

## Task 2 final runtime removal

- Removed server task list route, overview counters, session `task_id` JSON and
  SQL binding, task sync ingest/tombstone/read-back/criteria/blocker handlers,
  task capability names, and task retrieval candidates.
- Retrieval now gathers `session_memory`, then `branch_memory`, then
  `project_memory`; daemon delivery mirrors those three sections.
- Physically removed daemon's commented task handlers/detail helper, stale core
  task-only tests, and standalone task sync/criteria integration tests.
- Remaining `task_criteria` and `task_id` source matches are historical SQLite
  migration transformer code in `cairn-store/src/migrate.rs`; it exists only to
  advance legacy databases into immutable removal migration 0013.

### Validation

- `PATH="$HOME/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH" cargo check --workspace`
  passed (existing `cairn-server` web-operation dead-code warnings).
- `cargo test --workspace --all-targets --no-run` passed before focused tests.
- `git diff --check` passed.

## Review fix round 1 — retained-feature conservation

- Migration 0013 now retains legacy Task data pending explicit export; it does
  not destructively delete task rows or risk orphaning relations.
- Added `transfer::export_removed_feature_tasks`, a narrow versioned external
  `removed_feature` JSON bundle. It exports Tasks, bound sessions, task-scoped
  memories, observation and evidence links, dependent evidence, relations,
  criteria/history/blockers, criterion verification/evidence, task continuity,
  and legacy task outbox rows. Every exported source row is explicitly
  `retained`; the conservation report includes accepted/rejected zero counts.
- The output is create-new and synced before the manifest is marked
  `exported_retained`; an export failure leaves source data and safe capture
  untouched.

### Fix validation

- RED: `cargo test -p cairn-store transfer::tests::removed_feature_bundle_conserves_task_records_and_dependencies --lib` failed because the exporter did not exist.
- GREEN: the same focused test passes; it seeds task + task memory + evidence
  + relation data, verifies all conservation/disposition records, and verifies
  source tables are unchanged.
- `cargo check -p cairn-store` passes.
- `cargo clippy -p cairn-store -- -D warnings` passes.
- `git diff --check` passes.

### Residual follow-up

- Historical schema transformer references in `cairn-store/src/migrate.rs` are
  migration-only. The remaining disabled core task corpus and stale narrative
  references still require physical removal in the next cleanup pass.

## Review fix round 3 — transactional cleanup recovery

- Export now records `exported_pending_cleanup` plus immutable artifact path and
  SHA-256. Setup retries cleanup only; it never overwrites or re-exports an
  existing bundle.
- Cleanup validates artifact hash/schema, then transactionally deletes Task-only
  memories, links, relations, task outbox and criterion verification rows. It
  preserves evidence still linked from surviving memory, removes unreferenced
  Task evidence, and rebuilds live memory/outbox/verification constraints so
  Task scope/entity/criterion fields cannot return.
- Human `cairn setup` now renders migration status, warning detail, and all
  artifact paths; JSON envelope remains unchanged.

### Validation

- RED then GREEN: `cargo test -p cairn-store transfer::tests::removed_feature_bundle_conserves_task_records_and_dependencies --lib`.
- `cargo test -p cairnd init_legacy_task` passes, including corrupt-bundle
  cleanup warning, unchanged artifact, resumed cleanup, and usable store.
